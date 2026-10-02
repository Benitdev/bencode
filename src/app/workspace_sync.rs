//! Cached view of the active workspace (git + filesystem).
//!
//! Render functions must never shell out to git or walk the disk: GPUI
//! re-renders on every streamed token. Everything here is loaded on the
//! background executor and swapped in atomically; views read the cache.

use std::path::Path;

use anyhow::Result;
use gpui::{Context, SharedString};

use crate::app::BenCodeApp;
use crate::git::{self, DiffRow, GitCommitInfo, GitDetailedStatus, GitFileChange};
use crate::workspace::list_workspace_files;

const RECENT_COMMIT_COUNT: usize = 8;
/// Mention candidates kept in memory; filtering them per keystroke is cheap.
const MENTION_FILE_LIMIT: usize = 2_000;

#[derive(Default)]
pub struct WorkspaceCache {
    /// Directory the cache was loaded for.
    pub cwd: String,
    pub branches: Vec<String>,
    pub changes: Vec<GitFileChange>,
    /// Repo-relative paths; `SharedString` so views clone by refcount.
    pub files: Vec<SharedString>,
    pub worktrees: Vec<crate::git::Worktree>,
    pub diff_path: Option<String>,
    pub diff: Vec<DiffRow>,
    /// Unified text of `diff`, prepared once for the copy button.
    pub diff_text: SharedString,
    /// Last failed git action, shown in the Changes panel until the next one.
    pub git_error: Option<String>,
    generation: u64,
    diff_generation: u64,
}

struct Snapshot {
    status: GitDetailedStatus,
    commits: Vec<GitCommitInfo>,
    branches: Vec<String>,
    changes: Vec<GitFileChange>,
    files: Vec<SharedString>,
    worktrees: Vec<crate::git::Worktree>,
}

fn load_snapshot(cwd: &str) -> Snapshot {
    let root = Path::new(cwd);
    Snapshot {
        status: git::get_detailed_status(cwd),
        commits: git::get_recent_commits(cwd, RECENT_COMMIT_COUNT),
        branches: git::get_branches(cwd),
        changes: git::get_workspace_changes(cwd),
        files: list_workspace_files(root, MENTION_FILE_LIMIT)
            .into_iter()
            .map(SharedString::from)
            .collect(),
        worktrees: crate::git::worktrees::list_worktrees(cwd).unwrap_or_else(|err| {
            // Expected for folders that are not git repositories.
            log::debug!("no worktrees for {cwd}: {err:#}");
            Vec::new()
        }),
    }
}

impl BenCodeApp {
    /// The directory the workspace views describe: the focused worktree cwd,
    /// or the open thread's cwd, falling back to the current project directory.
    pub fn workspace_cwd(&self) -> String {
        if let Some(focus) = &self.worktree_focus {
            return focus.path.clone();
        }
        if let Some(session) = self
            .selected_session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            && !session.cwd.is_empty()
            && crate::app::is_path_in_project(&session.cwd, &self.current_cwd)
        {
            return session.work_dir().to_string();
        }
        self.current_cwd.clone()
    }

    /// Kept for existing call sites; reloads the whole workspace snapshot.
    pub fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        self.refresh_workspace(cx);
    }

    pub fn refresh_workspace(&mut self, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        self.workspace.generation += 1;
        let generation = self.workspace.generation;
        let load_cwd = cwd.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load_snapshot(&load_cwd) });

        cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.workspace.generation != generation {
                    return; // a newer refresh superseded this one
                }
                app.git_status = snapshot.status;
                app.git_commits = snapshot.commits;
                let cache = &mut app.workspace;
                cache.cwd = cwd;
                cache.branches = snapshot.branches;
                cache.changes = snapshot.changes;
                cache.files = snapshot.files;
                cache.worktrees = snapshot.worktrees;
                let diff_path = app
                    .selected_diff_path
                    .clone()
                    .or_else(|| app.workspace.changes.first().map(|c| c.path.clone()));
                app.load_diff(diff_path, cx);
                app.refresh_file_tree(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Refreshes only if the active thread points at a different directory.
    pub fn refresh_workspace_if_moved(&mut self, cx: &mut Context<Self>) {
        if self.workspace.cwd != self.workspace_cwd() {
            self.refresh_workspace(cx);
        }
    }

    pub fn select_diff_path(&mut self, path: String, cx: &mut Context<Self>) {
        self.selected_diff_path = Some(path.clone());
        self.load_diff(Some(path), cx);
        cx.notify();
    }

    fn load_diff(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.workspace.diff_generation += 1;
        let generation = self.workspace.diff_generation;
        let Some(path) = path else {
            self.workspace.diff_path = None;
            self.workspace.diff.clear();
            self.workspace.diff_text = SharedString::default();
            return;
        };
        let cwd = self.workspace_cwd();
        let file = path.clone();
        let task = cx.background_executor().spawn(async move {
            let rows = git::number_rows(git::get_file_diff(&cwd, &file));
            let text = SharedString::from(git::unified_text(&rows));
            (rows, text)
        });

        cx.spawn(async move |this, cx| {
            let (diff, diff_text) = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.workspace.diff_generation == generation {
                    app.workspace.diff_path = Some(path);
                    app.workspace.diff = diff;
                    app.workspace.diff_text = diff_text;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Runs a mutating git command off the UI thread, surfaces failures in
    /// the Changes panel, then reloads the snapshot.
    pub fn run_git_action(
        &mut self,
        label: &'static str,
        action: impl FnOnce(&str) -> Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.workspace_cwd();
        let task = cx.background_executor().spawn(async move { action(&cwd) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.workspace.git_error = result.err().map(|err| {
                    log::error!("git {label} failed: {err:#}");
                    format!("{label} failed: {err:#}")
                });
                app.refresh_workspace(cx);
            });
        })
        .detach();
    }
}
