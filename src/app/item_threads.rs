//! BenCode's own: which threads each Inbox item has, and what they are
//! doing, for the Inbox's rows and detail. A map from an item's key
//! (`WorkItem::key`, `LinkedWorkItem::item_key`) to the threads linked to
//! it, rebuilt when the threads in memory or their links change, never per
//! frame: a link written bumps `links_changed`, and a thread added or
//! dropped changes how many there are.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::app::BenCodeApp;
use crate::app::live_agents::{RunFlags, live_agent};
use crate::app::session_list::session_title;
use crate::db::SessionRow;

/// What a linked thread is doing, least urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThreadState {
    Idle,
    /// Finished, not looked at since.
    Done,
    Working,
    /// Waiting on an approval or an answer.
    NeedsYou,
}

impl ThreadState {
    /// The navigator's Agents section's reading of a thread.
    pub fn of(session: &SessionRow, flags: RunFlags) -> Self {
        match live_agent(session, flags) {
            None => Self::Idle,
            Some(agent) if agent.needs_approval => Self::NeedsYou,
            Some(agent) if agent.done => Self::Done,
            Some(_) => Self::Working,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Done => "Done",
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
        }
    }
}

/// A thread linked to an item, as the Inbox shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemThread {
    pub id: String,
    pub title: String,
    pub harness: String,
    pub model: String,
    pub updated_at: i64,
    pub state: ThreadState,
}

/// What an item's main button does: open its latest thread, or start one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimaryAction {
    Open(String),
    Send,
}

/// `threads` newest first (`ItemThreads::for_item`): the newest opens.
pub fn primary_action(threads: &[ItemThread]) -> PrimaryAction {
    threads.first().map_or(PrimaryAction::Send, |thread| {
        PrimaryAction::Open(thread.id.clone())
    })
}

/// The most urgent state among `threads`.
pub fn most_urgent(threads: &[ItemThread]) -> Option<ThreadState> {
    threads.iter().map(|thread| thread.state).max()
}

/// The linked threads' ids by item key, in `sessions`' order.
fn index_links(sessions: &[SessionRow]) -> HashMap<String, Vec<String>> {
    let mut by_item: HashMap<String, Vec<String>> = HashMap::new();
    for session in sessions {
        if let Some(item) = &session.linked_work_item {
            by_item
                .entry(item.item_key())
                .or_default()
                .push(session.id.clone());
        }
    }
    by_item
}

#[derive(Default)]
struct Index {
    /// How many threads there were and `generation`, when built.
    built_for: Option<(usize, u64)>,
    by_item: HashMap<String, Vec<String>>,
}

#[derive(Default)]
pub struct ItemThreads {
    /// Built while drawing, which reads the app.
    index: RefCell<Index>,
    /// Bumped by every link written.
    generation: u64,
    /// Threads already searched for a Backlog link (`work_item_link.rs`).
    pub(crate) searched: HashSet<String>,
}

impl ItemThreads {
    /// Ids of the threads linked to item `key`, as last indexed.
    fn ids(&self, sessions: &[SessionRow], key: &str) -> Vec<String> {
        let mut index = self.index.borrow_mut();
        let stamp = (sessions.len(), self.generation);
        if index.built_for != Some(stamp) {
            index.by_item = index_links(sessions);
            index.built_for = Some(stamp);
        }
        index.by_item.get(key).cloned().unwrap_or_default()
    }
}

impl BenCodeApp {
    /// A thread's link was written: the index is built again when next read.
    pub(crate) fn links_changed(&mut self) {
        self.item_threads.generation += 1;
    }

    /// What the app knows about a thread's run (the Agents section's view).
    pub(crate) fn run_flags(&self, id: &str) -> RunFlags {
        let run = self.runs.get(id);
        RunFlags {
            busy: run.is_some(),
            needs_approval: run.is_some_and(|run| run.pending_permission.is_some()),
            unseen_finished: self.title_strip.unseen_finished.contains(id),
        }
    }

    /// The threads linked to item `key`, newest first; archived ones left
    /// out.
    pub(crate) fn item_threads(&self, key: &str) -> Vec<ItemThread> {
        let ids = self.item_threads.ids(&self.sessions, key);
        let mut threads: Vec<ItemThread> = ids
            .iter()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .filter(|session| !session.archived)
            .map(|session| ItemThread {
                id: session.id.clone(),
                title: session_title(session),
                harness: session.harness.clone(),
                model: session.model.clone(),
                updated_at: session.updated_at,
                state: ThreadState::of(session, self.run_flags(&session.id)),
            })
            .collect();
        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        threads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::LinkedWorkItem;

    fn linked(id: &str, link: Option<LinkedWorkItem>) -> SessionRow {
        SessionRow {
            id: id.into(),
            linked_work_item: link,
            ..Default::default()
        }
    }

    #[test]
    fn threads_are_indexed_by_their_items_key() {
        let backlog = || LinkedWorkItem::backlog("WEB-42", "acme.backlog.com");
        let github = LinkedWorkItem::parse_url("https://github.com/Acme/App/pull/7");
        let sessions = [
            linked("a", backlog()),
            linked("b", None),
            linked("c", github),
            linked("d", backlog()),
        ];
        let index = index_links(&sessions);
        assert_eq!(index["backlog:web-42"], ["a", "d"]);
        assert_eq!(index["acme/app:pr:7"], ["c"]);
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn the_index_is_rebuilt_only_when_threads_or_links_change() {
        let mut threads = ItemThreads::default();
        let mut sessions = vec![linked(
            "a",
            LinkedWorkItem::backlog("WEB-1", "a.backlog.jp"),
        )];
        assert_eq!(threads.ids(&sessions, "backlog:web-1"), ["a"]);
        // Changed in place without a bump: still the old reading.
        sessions[0].linked_work_item = None;
        assert_eq!(threads.ids(&sessions, "backlog:web-1"), ["a"]);
        threads.generation += 1;
        assert!(threads.ids(&sessions, "backlog:web-1").is_empty());
        sessions.push(linked(
            "b",
            LinkedWorkItem::backlog("WEB-1", "a.backlog.jp"),
        ));
        assert_eq!(threads.ids(&sessions, "backlog:web-1"), ["b"]);
    }

    fn thread(id: &str, updated_at: i64, state: ThreadState) -> ItemThread {
        ItemThread {
            id: id.into(),
            title: String::new(),
            harness: String::new(),
            model: String::new(),
            updated_at,
            state,
        }
    }

    #[test]
    fn a_linked_item_opens_its_newest_thread() {
        assert_eq!(primary_action(&[]), PrimaryAction::Send);
        let threads = [
            thread("new", 20, ThreadState::Idle),
            thread("old", 10, ThreadState::Working),
        ];
        assert_eq!(primary_action(&threads), PrimaryAction::Open("new".into()));
        assert_eq!(most_urgent(&threads), Some(ThreadState::Working));
        assert_eq!(most_urgent(&[]), None);
        let asking = [
            thread("a", 1, ThreadState::Working),
            thread("b", 2, ThreadState::NeedsYou),
        ];
        assert_eq!(most_urgent(&asking), Some(ThreadState::NeedsYou));
    }

    #[test]
    fn states_follow_the_agents_section() {
        let session = linked("a", None);
        let busy = RunFlags {
            busy: true,
            ..Default::default()
        };
        assert_eq!(
            ThreadState::of(&session, RunFlags::default()),
            ThreadState::Idle
        );
        assert_eq!(ThreadState::of(&session, busy), ThreadState::Working);
        let asking = RunFlags {
            needs_approval: true,
            ..busy
        };
        assert_eq!(ThreadState::of(&session, asking), ThreadState::NeedsYou);
        let done = RunFlags {
            unseen_finished: true,
            ..Default::default()
        };
        assert_eq!(ThreadState::of(&session, done), ThreadState::Done);
    }
}
