//! Cached view of the active workspace (git + filesystem).
//!
//! Render functions must never shell out to git or walk the disk: GPUI
//! re-renders on every streamed token. Everything here is loaded on the
//! background executor and swapped in atomically; views read the cache.

use std::path::Path;

use anyhow::Result;
use gpui::Context;

use crate::app::BenCodeApp;
use crate::git::{self, DiffLineKind, GitCommitInfo, GitDetailedStatus, GitFileChange};
use crate::workspace::{FsNode, list_workspace_files, scan_directory};

const RECENT_COMMIT_COUNT: usize = 8;
const FILE_TREE_DEPTH: usize = 2;
/// Mention candidates kept in memory; filtering them per keystroke is cheap.
const MENTION_FILE_LIMIT: usize = 2_000;

#[derive(Default)]
pub struct WorkspaceCache {
    /// Directory the cache was loaded for.
    pub cwd: String,
    pub branches: Vec<String>,
    pub changes: Vec<GitFileChange>,
    pub files: Vec<String>,
    pub tree: Vec<FsNode>,
    pub diff_path: Option<String>,
    pub diff: Vec<DiffLineKind>,
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
    files: Vec<String>,
    tree: Vec<FsNode>,
}

fn load_snapshot(cwd: &str) -> Snapshot {
    let root = Path::new(cwd);
    Snapshot {
        status: git::get_detailed_status(cwd),
        commits: git::get_recent_commits(cwd, RECENT_COMMIT_COUNT),
        branches: git::get_branches(cwd),
        changes: git::get_workspace_changes(cwd),
        files: list_workspace_files(root, MENTION_FILE_LIMIT),
        tree: scan_directory(root, FILE_TREE_DEPTH),
    }
}

impl BenCodeApp {
    /// The directory the workspace views describe: the open thread's cwd,
    /// falling back to the directory BenCode was launched in.
    pub fn workspace_cwd(&self) -> String {
        self.selected_session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
            .unwrap_or_else(|| self.current_cwd.clone())
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
        let task = cx.background_executor().spawn(async move { load_snapshot(&load_cwd) });

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
                cache.tree = snapshot.tree;
                let diff_path = app
                    .selected_diff_path
                    .clone()
                    .or_else(|| app.workspace.changes.first().map(|c| c.path.clone()));
                app.load_diff(diff_path, cx);
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
            return;
        };
        let cwd = self.workspace_cwd();
        let file = path.clone();
        let task = cx.background_executor().spawn(async move { git::get_file_diff(&cwd, &file) });

        cx.spawn(async move |this, cx| {
            let diff = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.workspace.diff_generation == generation {
                    app.workspace.diff_path = Some(path);
                    app.workspace.diff = diff;
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
