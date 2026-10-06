//! MonoCode's session review (`checkpoint.ts`, `SessionReview.tsx`): the
//! files a thread's agent changed, kept in `git::checkpoint` around its edit
//! tools, with Keep, Undo and a read-only review of them.

use std::collections::HashMap;
use std::sync::mpsc;

use gpui::Context;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::git::checkpoint::{CheckpointFile, CheckpointStatus, CheckpointStore};

/// MonoCode shows three files until the card is expanded.
pub const COLLAPSED_FILES: usize = 3;

type Job = Box<dyn FnOnce(&CheckpointStore) + Send>;

/// What a thread's card shows.
#[derive(Default)]
pub struct SessionReview {
    pub files: Vec<CheckpointFile>,
    pub expanded: bool,
    /// Keep or Undo is running.
    pub acting: bool,
    pub error: Option<String>,
    /// Grows whenever the card's height may have changed.
    pub stamp: u64,
    generation: u64,
}

/// The checkpoint store and the cards read from it. Store work runs in
/// order on one thread (MonoCode `enqueueCheckpoint`), so a tool's snapshot
/// always lands after the turn's baseline.
pub struct Checkpoints {
    pub store: CheckpointStore,
    jobs: mpsc::Sender<Job>,
    pub reviews: HashMap<String, SessionReview>,
    /// The thread whose "Undo all" waits for confirmation.
    pub confirm_undo: Option<String>,
}

impl Checkpoints {
    pub fn new(store: CheckpointStore) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        let worker = store.clone();
        let spawned = std::thread::Builder::new()
            .name("bencode-checkpoints".into())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    job(&worker);
                }
            });
        if let Err(err) = spawned {
            log::error!("could not start the checkpoint thread: {err}");
        }
        Self {
            store,
            jobs,
            reviews: HashMap::new(),
            confirm_undo: None,
        }
    }

    /// Queues `job`; its result arrives on the returned channel.
    fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&CheckpointStore) -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (done, result) = oneshot::channel();
        let queued = self.jobs.send(Box::new(move |store| {
            // The receiver is gone when nobody waits for the result.
            if done.send(job(store)).is_err() {
                log::trace!("checkpoint result dropped");
            }
        }));
        if queued.is_err() {
            log::error!("the checkpoint thread is not running");
        }
        result
    }

    /// Queues `job` for its effect; a failure is only logged.
    fn run_logged(
        &self,
        what: &'static str,
        job: impl FnOnce(&CheckpointStore) -> Result<(), String> + Send + 'static,
    ) {
        drop(self.run(move |store| {
            if let Err(err) = job(store) {
                log::warn!("checkpoint {what} failed: {err}");
            }
        }));
    }

    /// MonoCode `beginSessionTurn`: the baseline before a live turn.
    pub fn begin_turn(&self, session_id: &str, cwd: &str) {
        if cwd.is_empty() || cwd == "~" {
            return;
        }
        let (id, cwd) = (session_id.to_string(), cwd.to_string());
        self.run_logged("baseline", move |store| store.ensure(&id, &cwd));
    }

    /// MonoCode `trackSessionEdits`, at a tool start.
    pub fn prepare(&self, session_id: &str, cwd: &str, paths: Vec<String>) {
        let (id, cwd) = (session_id.to_string(), cwd.to_string());
        self.run_logged("prepare", move |store| store.prepare(&id, &cwd, &paths));
    }

    /// MonoCode `trackSessionEdits`, when the tool completed.
    pub fn capture(&self, session_id: &str, cwd: &str, paths: Vec<String>) {
        let (id, cwd) = (session_id.to_string(), cwd.to_string());
        self.run_logged("capture", move |store| store.capture(&id, &cwd, &paths));
    }

    pub fn forget(&mut self, session_id: &str) {
        self.reviews.remove(session_id);
        let id = session_id.to_string();
        self.run_logged("forget", move |store| store.forget(&id));
    }
}

/// The files an edit tool's input names: Claude and Antigravity pass one
/// path, Codex a `changes` list or map.
pub fn edit_paths(input: &Value) -> Vec<String> {
    const KEYS: [&str; 6] = [
        "file_path",
        "path",
        "notebook_path",
        "TargetFile",
        "target_file",
        "filePath",
    ];
    let mut paths: Vec<String> = KEYS
        .iter()
        .filter_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    match input.get("changes") {
        Some(Value::Array(items)) => paths.extend(
            items
                .iter()
                .filter_map(|item| item.get("path").and_then(Value::as_str))
                .map(str::to_string),
        ),
        Some(Value::Object(map)) => paths.extend(map.keys().cloned()),
        _ => {}
    }
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| !path.trim().is_empty() && seen.insert(path.clone()));
    paths
}

/// What a card's action does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewAction {
    Keep,
    Undo,
}

impl BenCodeApp {
    fn session_work_dir(&self, session_id: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.work_dir().to_string())
            .filter(|cwd| !cwd.is_empty() && cwd != "~")
    }

    /// The card is the result of a turn: hidden while one runs.
    pub fn session_review_shown(&self, session_id: &str) -> bool {
        !self.is_agent_running_in(session_id)
            && self
                .checkpoints
                .reviews
                .get(session_id)
                .is_some_and(|review| !review.files.is_empty())
    }

    /// MonoCode `undoLocked`: another thread's agent is at work in the
    /// same directory.
    pub fn session_undo_locked(&self, session_id: &str) -> bool {
        let Some(cwd) = self.session_work_dir(session_id) else {
            return false;
        };
        self.sessions
            .iter()
            .any(|s| s.id != session_id && s.work_dir() == cwd && self.is_agent_running_in(&s.id))
    }

    fn apply_review(&mut self, session_id: &str, files: Vec<CheckpointFile>, cx: &mut Context<Self>) {
        let review = self.checkpoints.reviews.entry(session_id.to_string()).or_default();
        if review.files != files {
            review.files = files;
            review.stamp += 1;
            if review.files.len() <= COLLAPSED_FILES {
                review.expanded = false;
            }
            // An open review of these changes shows the new state.
            let open = self.file_pane.entries().iter().find_map(|entry| match &entry.tab {
                PaneTab::SessionChanges { session_id: id, .. } if id == session_id => {
                    Some(entry.tab.key())
                }
                _ => None,
            });
            if let Some(key) = open {
                self.load_diff_doc(&key, cx);
            }
        }
        cx.notify();
    }

    /// Reads what the thread's agent changed; a newer read wins.
    pub fn load_session_review(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(cwd) = self.session_work_dir(session_id) else {
            return;
        };
        let review = self.checkpoints.reviews.entry(session_id.to_string()).or_default();
        review.generation += 1;
        let generation = review.generation;
        let id = session_id.to_string();
        let job_id = id.clone();
        let result = self.checkpoints.run(move |store| store.status(&job_id, &cwd));
        cx.spawn(async move |this, cx| {
            let Ok(status) = result.await else {
                return;
            };
            let landed = this.update(cx, |app, cx| {
                let current = app.checkpoints.reviews.get(&id).map(|r| r.generation);
                if current != Some(generation) {
                    return;
                }
                match status {
                    Ok(status) => app.apply_review(&id, status.files, cx),
                    Err(err) => {
                        log::warn!("could not read session changes of {id}: {err}");
                        app.apply_review(&id, Vec::new(), cx);
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("session review after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Reloads the cards that have files and belong to `cwd` (git moved).
    pub fn reload_session_reviews(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.work_dir() == cwd)
            .filter(|s| {
                self.checkpoints
                    .reviews
                    .get(&s.id)
                    .is_some_and(|review| !review.files.is_empty())
            })
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            self.load_session_review(&id, cx);
        }
    }

    pub fn toggle_session_review(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some(review) = self.checkpoints.reviews.get_mut(session_id) {
            review.expanded = !review.expanded;
            review.stamp += 1;
            cx.notify();
        }
    }

    /// Keep all, or Undo all, of a thread's changes.
    pub fn resolve_session_review(
        &mut self,
        session_id: &str,
        action: ReviewAction,
        cx: &mut Context<Self>,
    ) {
        self.checkpoints.confirm_undo = None;
        let Some(cwd) = self.session_work_dir(session_id) else {
            return;
        };
        if action == ReviewAction::Undo && self.session_undo_locked(session_id) {
            return;
        }
        let Some(review) = self.checkpoints.reviews.get_mut(session_id) else {
            return;
        };
        if review.acting {
            return;
        }
        review.acting = true;
        review.error = None;
        review.generation += 1;
        let id = session_id.to_string();
        let job_id = id.clone();
        let result = self.checkpoints.run(move |store| match action {
            ReviewAction::Keep => store.keep(&job_id, &cwd, None),
            ReviewAction::Undo => store.undo(&job_id, &cwd, None),
        });
        cx.spawn(async move |this, cx| {
            let outcome: Result<CheckpointStatus, String> = result
                .await
                .unwrap_or_else(|_| Err("The checkpoint store is not running".into()));
            let landed = this.update(cx, |app, cx| {
                if let Some(review) = app.checkpoints.reviews.get_mut(&id) {
                    review.acting = false;
                    review.stamp += 1;
                }
                match outcome {
                    Ok(status) => app.apply_review(&id, status.files, cx),
                    Err(err) => {
                        log::warn!("session {action:?} failed for {id}: {err}");
                        if let Some(review) = app.checkpoints.reviews.get_mut(&id) {
                            review.error = Some(err);
                        }
                        app.load_session_review(&id, cx);
                    }
                }
                // Undo rewrote files: git state, the tree and open editors follow.
                app.refresh_workspace(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("session review action after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode `onOpenDiff(path, session)`: the thread's changes as a review.
    pub fn open_session_changes(
        &mut self,
        session_id: &str,
        focus: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.session_work_dir(session_id) else {
            return;
        };
        let tab = PaneTab::SessionChanges {
            cwd,
            session_id: session_id.to_string(),
            focus,
        };
        self.open_pane_tab(tab, true, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edit_inputs_name_their_files() {
        assert_eq!(edit_paths(&json!({"file_path": "/r/a.rs", "old_string": "x"})), ["/r/a.rs"]);
        assert_eq!(edit_paths(&json!({"TargetFile": "b.rs", "path": "b.rs"})), ["b.rs"]);
        assert_eq!(
            edit_paths(&json!({"changes": [{"path": "a"}, {"path": "b"}, {"kind": "x"}]})),
            ["a", "b"]
        );
        assert_eq!(edit_paths(&json!({"changes": {"src/x.rs": {"update": {}}}})), ["src/x.rs"]);
        assert!(edit_paths(&json!({"command": "ls"})).is_empty());
        assert!(edit_paths(&Value::Null).is_empty());
    }

    #[test]
    fn queued_work_runs_in_order() {
        let dir = std::env::temp_dir().join(format!("bencode-queue-{}", std::process::id()));
        let checkpoints = Checkpoints::new(CheckpointStore::new(dir));
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        for n in 0..20 {
            let log = log.clone();
            drop(checkpoints.run(move |_| log.lock().unwrap().push(n)));
        }
        let last = checkpoints.run(|_| ());
        last.blocking_recv().unwrap();
        assert_eq!(*log.lock().unwrap(), (0..20).collect::<Vec<_>>());
    }
}
