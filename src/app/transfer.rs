//! A thread moved to another agent mid-conversation (MonoCode GPUI's
//! provider switch, `runtime.rs` and `transcript/provider_switch.rs`):
//! the model picker sets a model of another harness, which has no session
//! of this conversation. Its first turn reads the Handoff's recap of the
//! conversation so far ahead of the message, so it continues rather than
//! starting blind, and the message keeps `transferFrom` (the agent and
//! model it came from) for the transcript: a rule with the new agent's
//! mark, "Transferred from <model>". MonoCode has the outgoing model write
//! the recap, a wait it shows as a spinning step; BenCode's is built from
//! the transcript (`handoff::deterministic_brief`), at once and at no
//! token cost.

use serde_json::{Value, json};

use crate::db::{Block, SessionRow};
use crate::harness::HarnessKind;

/// The extra field a transferred turn's message keeps.
pub const TRANSFER_FROM: &str = "transferFrom";

/// Where a thread's next turn comes from, when it changes agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub from: HarnessKind,
    /// The model it ran, as the picker names it.
    pub model: String,
}

impl Transfer {
    pub fn to_extra(&self) -> Value {
        json!({ "harness": self.from.id(), "model": self.model })
    }

    /// A message's `transferFrom`.
    pub fn from_block(block: &Block) -> Option<Self> {
        let value = block.extra.get(TRANSFER_FROM)?;
        Some(Self {
            from: HarnessKind::from_id(value.get("harness")?.as_str()?)?,
            model: value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// "Transferred from Opus 4.6", or the agent's name without a model.
    pub fn label(&self) -> String {
        let from = match self.model.trim() {
            "" => self.from.label(),
            model => model,
        };
        format!("Transferred from {from}")
    }
}

/// The harness a message's turn ran on, from its provenance.
pub fn turn_harness(block: &Block) -> Option<&str> {
    block.turn_model.as_ref()?.harness.as_deref()
}

/// The last turn's agent when the thread now runs another one that has
/// no session of this conversation yet; `None` for a thread that stays
/// with its agent, or starts.
pub fn pending_transfer(session: &SessionRow) -> Option<Transfer> {
    if session
        .provider_session_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
    {
        return None;
    }
    let last = session
        .blocks
        .iter()
        .rev()
        .find(|b| b.role == "user" && b.turn_model.is_some())?;
    let model = last.turn_model.as_ref()?;
    let from = model.harness.as_deref()?;
    if from == session.harness {
        return None;
    }
    Some(Transfer {
        from: HarnessKind::from_id(from)?,
        model: model.name.clone().unwrap_or_default(),
    })
}

/// The harness the message at `ix` moved the thread to, when the message
/// before it ran on another (MonoCode `HandoffDivider`).
pub fn switched_to(blocks: &[std::sync::Arc<Block>], ix: usize) -> Option<&str> {
    let now = turn_harness(blocks.get(ix)?)?;
    let before = blocks[..ix]
        .iter()
        .rev()
        .filter(|b| b.role == "user")
        .find_map(|b| turn_harness(b))?;
    (before != now).then_some(now)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::TurnModel;

    fn user(harness: &str, model: &str) -> Arc<Block> {
        let mut block = Block::new(format!("u-{harness}-{model}"), "user", "hi");
        block.turn_model = Some(TurnModel {
            harness: Some(harness.into()),
            id: Some(format!("{harness}:{model}")),
            name: Some(model.into()),
        });
        block.into()
    }

    fn thread(harness: &str, blocks: Vec<Arc<Block>>) -> SessionRow {
        SessionRow {
            harness: harness.into(),
            blocks,
            ..Default::default()
        }
    }

    #[test]
    fn a_new_agent_without_a_session_gets_the_transfer() {
        let blocks = vec![user("claude", "Opus 4.6")];
        let transfer = pending_transfer(&thread("codex", blocks.clone())).unwrap();
        assert_eq!(transfer.from, HarnessKind::Claude);
        assert_eq!(transfer.label(), "Transferred from Opus 4.6");
        // The same agent, or one already holding a session, needs none.
        assert!(pending_transfer(&thread("claude", blocks.clone())).is_none());
        let mut resumed = thread("codex", blocks);
        resumed.provider_session_id = Some("s1".into());
        assert!(pending_transfer(&resumed).is_none());
        assert!(pending_transfer(&thread("codex", Vec::new())).is_none());
    }

    #[test]
    fn the_divider_marks_the_first_message_another_agent_answers() {
        let blocks = vec![
            user("claude", "Opus"),
            Arc::new(Block::new("a", "assistant", "ok")),
            user("claude", "Opus"),
            user("codex", "GPT-5"),
            user("codex", "GPT-5"),
        ];
        assert_eq!(switched_to(&blocks, 0), None);
        assert_eq!(switched_to(&blocks, 2), None);
        assert_eq!(switched_to(&blocks, 3), Some("codex"));
        assert_eq!(switched_to(&blocks, 4), None);
    }

    #[test]
    fn the_message_keeps_where_it_came_from() {
        let transfer = Transfer {
            from: HarnessKind::Codex,
            model: String::new(),
        };
        let mut block = Block::new("u", "user", "go on");
        block
            .extra
            .insert(TRANSFER_FROM.into(), transfer.to_extra());
        let back = Transfer::from_block(&block).unwrap();
        assert_eq!(back, transfer);
        assert_eq!(
            back.label(),
            format!("Transferred from {}", HarnessKind::Codex.label())
        );
    }
}
