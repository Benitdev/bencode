//! Cached view of the active workspace (git + filesystem).
//!
//! Render functions must never shell out to git or walk the disk: GPUI
//! re-renders on every streamed token. Everything here is loaded on the
//! background executor and swapped in atomically; views read the cache.

use std::path::Path;

use anyhow::Result;
use gpui::{Context, SharedString};

use crate::app::BenCodeApp;
use crate::git::{self, DiffRow, DiffSource, GitCommitInfo, GitDetailedStatus, GitFileChange};
use crate::workspace::list_workspace_files;

const RECENT_COMMIT_COUNT: usize = 8;
/// MonoCode re-reads git state this often (`GIT_POLL_MS`).
const GIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// Mention candidates kept in memory; filtering them per keystroke is cheap.
const MENTION_FILE_LIMIT: usize = 2_000;

/// A past commit opened from the history: its files and which one is shown.
#[derive(Clone, Debug)]
pub struct CommitView {
    pub sha: String,
    pub short_hash: String,
    pub subject: String,
    pub files: Vec<GitFileChange>,
}

#[derive(Default)]
pub struct WorkspaceCache {
    /// Directory the cache was loaded for.
    pub cwd: String,
    pub branches: Vec<git::Branch>,
    pub changes: Vec<GitFileChange>,
    /// Repo-relative paths; `SharedString` so views clone by refcount.
    pub files: Vec<SharedString>,
    pub worktrees: Vec<crate::git::Worktree>,
    pub diff_path: Option<String>,
    /// Which change `diff` shows for `diff_path`.
    pub diff_source: DiffSource,
    /// Set while a commit from the history is open in the Changes view.
    pub commit_view: Option<CommitView>,
    pub diff: Vec<DiffRow>,
    /// Unified text of `diff`, prepared once for the copy button.
    pub diff_text: SharedString,
    /// Last failed git action, shown in the Changes panel until the next one.
    pub git_error: Option<String>,
    /// `git::state_fingerprint` of the loaded snapshot.
    fingerprint: Option<u64>,
    generation: u64,
    diff_generation: u64,
}

struct Snapshot {
    fingerprint: Option<u64>,
    status: GitDetailedStatus,
    commits: Vec<GitCommitInfo>,
    branches: Vec<git::Branch>,
    changes: Vec<GitFileChange>,
    files: Vec<SharedString>,
    worktrees: Vec<crate::git::Worktree>,
}

fn load_snapshot(cwd: &str) -> Snapshot {
    let root = Path::new(cwd);
    Snapshot {
        fingerprint: git::state_fingerprint(cwd),
        status: git::get_detailed_status(cwd),
        commits: git::get_recent_commits(cwd, RECENT_COMMIT_COUNT),
        branches: git::list_branches(cwd),
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
        if let Some(focus) = self.worktree_focus() {
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
        self.refresh_skills(false, cx);
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
                cache.fingerprint = snapshot.fingerprint;
                cache.branches = snapshot.branches;
                cache.changes = snapshot.changes;
                cache.files = snapshot.files;
                cache.worktrees = snapshot.worktrees;
                if app.workspace.commit_view.is_none() {
                    let diff_path = app
                        .selected_diff_path
                        .clone()
                        .or_else(|| app.workspace.changes.first().map(|c| c.path.clone()));
                    let source = app.workspace.diff_source.clone();
                    app.load_diff(diff_path, source, cx);
                }
                app.refresh_file_tree(cx);
                app.recheck_open_files_on_disk(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Polls git every `GIT_POLL_INTERVAL` and refreshes when the repository
    /// changed outside BenCode (terminal commits, checkouts, agent edits).
    pub fn start_git_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GIT_POLL_INTERVAL).await;
                let Ok(cwd) = this.update(cx, |app, _| app.workspace.cwd.clone()) else {
                    return; // app dropped
                };
                if cwd.is_empty() {
                    continue;
                }
                let probe = cwd.clone();
                let fingerprint = cx
                    .background_executor()
                    .spawn(async move { git::state_fingerprint(&probe) })
                    .await;
                let changed = this.update(cx, |app, cx| {
                    let same_dir = app.workspace.cwd == cwd;
                    if same_dir && fingerprint.is_some() && fingerprint != app.workspace.fingerprint
                    {
                        app.refresh_workspace(cx);
                    }
                });
                if changed.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Refreshes only if the active thread points at a different directory.
    pub fn refresh_workspace_if_moved(&mut self, cx: &mut Context<Self>) {
        if self.workspace.cwd != self.workspace_cwd() {
            self.refresh_workspace(cx);
        }
    }

    /// Shows the HEAD↔work-tree diff of `path` (the Changes view list).
    pub fn select_diff_path(&mut self, path: String, cx: &mut Context<Self>) {
        self.select_diff(path, DiffSource::WorkingTree, cx);
    }

    pub fn select_diff(&mut self, path: String, source: DiffSource, cx: &mut Context<Self>) {
        if !matches!(source, DiffSource::Commit(_)) {
            self.workspace.commit_view = None;
        }
        self.selected_diff_path = Some(path.clone());
        self.load_diff(Some(path), source, cx);
        cx.notify();
    }

    /// Opens a commit from the history: lists its files and shows the first.
    pub fn open_commit(&mut self, commit: &GitCommitInfo, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        let sha = commit.hash.clone();
        let (short_hash, subject) = (commit.short_hash.clone(), commit.message.clone());
        let task = cx
            .background_executor()
            .spawn(async move { git::commit_files(&cwd, &sha).map(|files| (sha, files)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| match result {
                Ok((sha, files)) => {
                    let first = files.first().map(|f| f.path.clone());
                    app.workspace.commit_view = Some(CommitView {
                        sha: sha.clone(),
                        short_hash,
                        subject,
                        files,
                    });
                    app.workspace.git_error = None;
                    match first {
                        Some(path) => app.select_diff(path, DiffSource::Commit(sha), cx),
                        None => app.load_diff(None, DiffSource::Commit(sha), cx),
                    }
                    cx.notify();
                }
                Err(err) => {
                    log::error!("could not open commit: {err:#}");
                    app.workspace.git_error = Some(format!("Could not open commit: {err}"));
                    cx.notify();
                }
            });
            if let Err(err) = updated {
                log::debug!("commit opened after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Leaves an open commit and returns to the working-tree changes.
    pub fn close_commit(&mut self, cx: &mut Context<Self>) {
        self.workspace.commit_view = None;
        let path = self.workspace.changes.first().map(|c| c.path.clone());
        self.selected_diff_path = path.clone();
        self.load_diff(path, DiffSource::WorkingTree, cx);
        cx.notify();
    }

    fn load_diff(&mut self, path: Option<String>, source: DiffSource, cx: &mut Context<Self>) {
        self.workspace.diff_source = source.clone();
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
            let rows = git::number_rows(git::diff_for(&cwd, &file, &source));
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

    /// Checks out `target` off the UI thread (MonoCode `BranchPicker`).
    /// Local changes in the way open the "Uncommitted changes" dialog.
    pub fn switch_to_branch(
        &mut self,
        target: BranchTarget,
        stash_first: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_branch_picker_open = false;
        self.blocked_branch_switch = None;
        let cwd = self.workspace_cwd();
        let job = target.clone();
        let task = cx.background_executor().spawn(async move {
            if stash_first {
                git::stash_changes(&cwd, &format!("BenCode: switch to {}", job.label()))
                    .map_err(git::SwitchError::Failed)?;
            }
            match &job {
                BranchTarget::Existing(branch) => git::switch_branch(&cwd, branch),
                BranchTarget::New(name) => git::create_branch(&cwd, name),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                match result {
                    Ok(name) => {
                        app.workspace.git_error = None;
                        if let Some(session) = app.selected_session_mut() {
                            session.branch = Some(name);
                        }
                    }
                    Err(git::SwitchError::BlockedByChanges) => {
                        app.blocked_branch_switch = Some(target);
                    }
                    Err(git::SwitchError::Failed(err)) => {
                        log::error!("branch switch failed: {err:#}");
                        app.workspace.git_error = Some(format!("Switch branch failed: {err:#}"));
                    }
                }
                app.refresh_workspace(cx);
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("branch switch finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode's `SwitchBranchDialog`: Cancel or Stash & switch.
    pub fn render_branch_switch_confirm(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        use ely_gpui_component::overlays::ConfirmDialog;
        use gpui::IntoElement;
        let target = self.blocked_branch_switch.clone()?;
        let cancel = crate::ui::app_callback::app_callback(cx, |this, cx| {
            this.blocked_branch_switch = None;
            cx.notify();
        });
        let label = target.label().to_string();
        let stash = crate::ui::app_callback::app_callback(cx, move |this, cx| {
            this.switch_to_branch(target.clone(), true, cx)
        });
        Some(
            ConfirmDialog::new(
                "branch-switch-blocked",
                "Uncommitted changes",
                format!("Your local changes would be overwritten by switching to {label}. Stash them and switch?"),
                cancel,
            )
            .confirm("Stash & switch")
            .on_confirm(stash)
            .into_any_element(),
        )
    }
}

/// What the branch picker asked for.
#[derive(Clone, Debug)]
pub enum BranchTarget {
    Existing(git::Branch),
    New(String),
}

impl BranchTarget {
    pub fn label(&self) -> &str {
        match self {
            Self::Existing(branch) => &branch.name,
            Self::New(name) => name,
        }
    }
}
