//! What a thread's composer and queue hold between turns, by session id
//! (MonoCode keeps these on the `Session`, `features/sessions/model/session.ts`,
//! and drops them in `sessionRemoval.ts`).

use std::sync::Arc;

use crate::app::{BenCodeApp, TurnInput};
use crate::harness::Attachment;
use crate::ui::composer::cards::ComposerCard;
use crate::ui::composer::mcp_tags::McpTag;
use crate::ui::composer::question::QuestionUi;
use crate::ui::composer::usage_limit::UsageLimit;

#[derive(Default)]
pub struct ThreadState {
    /// The composer has Plan mode / Draft on.
    pub plan_mode: bool,
    pub draft_mode: bool,
    /// The prompt left in the composer when another thread took the focus.
    pub draft: Option<String>,
    /// The MCP tags kept with that draft (MonoCode `getComposerMcpTags`).
    pub mcp_tags: Option<Arc<Vec<McpTag>>>,
    /// Files attached in the composer, not sent yet.
    pub attachments: Vec<Attachment>,
    /// File reads in flight, and whether a send waits for them.
    pub attaching: usize,
    pub send_after_attach: bool,
    /// MonoCode's composer card (note, handoff), carried until the next send.
    pub composer_card: Option<ComposerCard>,
    /// The form state of the agent's pending question.
    pub question_ui: Option<QuestionUi>,
    /// The thread was stopped by its provider's usage limit.
    pub usage_limit: Option<UsageLimit>,
    /// The provider is rewinding for an edited resend.
    pub edit_rewinding: bool,
    /// A worktree is being made for the first send.
    pub preparing_worktree: bool,
    /// Prompts sent while the thread was busy, oldest first.
    pub queue: Vec<TurnInput>,
    /// The next message waits for the edit of the queue's head to end.
    pub queue_held: bool,
    /// MonoCode `queueStatus: "paused"`: stopped with messages waiting;
    /// nothing is sent from the queue until Resume.
    pub queue_paused: bool,
}

impl ThreadState {
    /// One file read ended; true when a send waited on the last one.
    pub fn finish_attaching(&mut self) -> bool {
        self.attaching = self.attaching.saturating_sub(1);
        self.attaching == 0 && std::mem::take(&mut self.send_after_attach)
    }

    /// Send pressed while files are still being read: it goes once they land.
    pub fn defer_send(&mut self) -> bool {
        self.send_after_attach |= self.attaching > 0;
        self.attaching > 0
    }
}

impl BenCodeApp {
    /// A thread's state, if it has any.
    pub fn thread(&self, id: &str) -> Option<&ThreadState> {
        self.threads.get(id)
    }

    /// A thread's state, created empty on first use.
    pub fn thread_mut(&mut self, id: &str) -> &mut ThreadState {
        self.threads.entry(id.to_string()).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attaching_counts_down_and_releases_a_waiting_send() {
        let mut thread = ThreadState {
            attaching: 2,
            ..Default::default()
        };
        assert!(thread.defer_send());
        assert!(thread.send_after_attach);
        assert!(!thread.finish_attaching());
        assert!(thread.finish_attaching());
        // The waiting send was released once.
        assert!(!thread.finish_attaching());
        assert_eq!(thread.attaching, 0);
    }

    #[test]
    fn a_send_with_nothing_attaching_goes_now() {
        let mut thread = ThreadState::default();
        assert!(!thread.defer_send());
        assert!(!thread.send_after_attach);
        assert!(!thread.finish_attaching());
    }

    #[test]
    fn forgetting_a_thread_drops_all_of_it() {
        let mut threads = std::collections::HashMap::new();
        threads.insert(
            "s1".to_string(),
            ThreadState {
                draft: Some("half a thought".into()),
                queue: vec![TurnInput::default()],
                plan_mode: true,
                ..Default::default()
            },
        );
        threads.remove("s1");
        assert!(threads.is_empty());
    }
}
