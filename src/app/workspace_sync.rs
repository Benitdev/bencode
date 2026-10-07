//! Cached view of the active workspace (git + filesystem).
//!
//! Render functions must never shell out to git or walk the disk: GPUI
//! re-renders on every streamed token. Everything here is loaded on the
//! background executor and swapped in atomically; views read the cache.

use std::collections::HashMap;
use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant};

use gpui::Context;

use crate::app::BenCodeApp;
use crate::git::{self, GitDetailedStatus, GitFileChange};

/// MonoCode re-reads git state this often (`GIT_POLL_MS`).
const GIT_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// VS Code's `git.autofetchPeriod` (180s): how stale a directory's remote
/// refs may get before the background fetch runs again.
const AUTO_FETCH_INTERVAL: Duration = Duration::from_secs(180);
/// How often the auto-fetch loop looks at the open directory, so switching
/// to a project that was never fetched does not wait a full period.
const AUTO_FETCH_TICK: Duration = Duration::from_secs(30);
/// Focusing the window fetches when the last fetch is older than this.
const FOCUS_FETCH_AGE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct WorkspaceCache {
    /// Directory the cache was loaded for.
    pub cwd: String,
    pub branches: Vec<git::Branch>,
    pub changes: Vec<GitFileChange>,
    pub worktrees: Vec<crate::git::Worktree>,
    /// Last failed git action, shown in the Changes panel until the next one.
    pub git_error: Option<String>,
    /// MonoCode `GitInfo.repo`: the repository name session cards show.
    pub repo: Option<String>,
    /// `git::state_fingerprint` of the loaded snapshot.
    fingerprint: Option<u64>,
    generation: u64,
    /// The `generation` whose load last landed; behind `generation` while
    /// a load is in flight.
    loaded_generation: u64,
    /// Each directory's last snapshot, shown at once when the workspace
    /// returns to it while a fresh one loads (MonoCode `indexByCwd`).
    snapshots: HashMap<String, Snapshot>,
    /// A background `git fetch` is running.
    fetching: bool,
    /// When each directory's remote refs were last fetched.
    fetched_at: HashMap<String, Instant>,
}

impl WorkspaceCache {
    /// Uncommitted lines in `project` as of its last snapshot, so the rail
    /// can show stats for projects other than the open one.
    pub fn cached_diff_stats(&self, project: &str) -> Option<(usize, usize)> {
        let (_, snapshot) = self
            .snapshots
            .iter()
            .find(|(cwd, _)| crate::app::same_project_path(cwd, project))?;
        let files = snapshot.status.staged.iter().chain(&snapshot.status.unstaged);
        Some(files.fold((0, 0), |(add, del), f| (add + f.additions, del + f.deletions)))
    }
}

#[derive(Clone)]
struct Snapshot {
    fingerprint: Option<u64>,
    status: GitDetailedStatus,
    sync: git::sync::SyncInfo,
    history: Vec<git::sync::HistoryCommit>,
    branches: Vec<git::Branch>,
    changes: Vec<GitFileChange>,
    worktrees: Vec<crate::git::Worktree>,
    repo: Option<String>,
}

/// Runs the snapshot's git reads side by side: each is its own `git`
/// process, so the wall time is the slowest one rather than their sum.
fn load_snapshot(cwd: &str) -> Snapshot {
    std::thread::scope(|scope| {
        // One `git status` feeds the fingerprint, status and changes.
        let local = scope.spawn(|| git::read_local_state(cwd));
        let sync = scope.spawn(|| git::sync::sync_info(cwd));
        let history = scope.spawn(|| git::sync::history(cwd));
        let branches = scope.spawn(|| git::list_branches(cwd));
        let worktrees = scope.spawn(|| {
            crate::git::worktrees::list_worktrees(cwd).unwrap_or_else(|err| {
                // Expected for folders that are not git repositories.
                log::debug!("no worktrees for {cwd}: {err:#}");
                Vec::new()
            })
        });
        let repo = scope.spawn(|| git::sync::repo_name(cwd));
        let local = joined(local, "status");
        Snapshot {
            fingerprint: local.fingerprint,
            status: local.status,
            sync: joined(sync, "sync info"),
            history: joined(history, "history"),
            branches: joined(branches, "branches"),
            changes: local.changes,
            worktrees: joined(worktrees, "worktrees"),
            repo: joined(repo, "repo name"),
        }
    })
}

fn joined<T: Default>(handle: ScopedJoinHandle<'_, T>, what: &str) -> T {
    handle.join().unwrap_or_else(|_| {
        log::error!("loading git {what} panicked");
        T::default()
    })
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

    pub fn refresh_workspace(&mut self, cx: &mut Context<Self>) {
        self.refresh_skills(false, cx);
        // MonoCode keeps the composer's file index for its folder, so `@`
        // labels paint before the picker is ever opened.
        self.index_project_files(cx);
        let cwd = self.workspace_cwd();
        if self.workspace.cwd != cwd {
            self.show_cached_snapshot(&cwd, cx);
        }
        self.workspace.generation += 1;
        let generation = self.workspace.generation;
        let load_cwd = cwd.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load_snapshot(&load_cwd) });

        cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.workspace.generation != generation {
                    return; // a newer refresh superseded this one
                }
                app.workspace.loaded_generation = generation;
                app.workspace.snapshots.insert(cwd.clone(), snapshot.clone());
                app.apply_snapshot(cwd.clone(), snapshot, cx);
                app.reload_working_tree_docs(&cwd, cx);
                app.reload_session_reviews(&cwd, cx);
                app.refresh_file_tree(cx);
                app.recheck_open_files_on_disk(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("workspace snapshot after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Moving to another directory: its last snapshot if there is one,
    /// otherwise an empty workspace, never the previous directory's data.
    fn show_cached_snapshot(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let cache = &mut self.workspace;
        cache.git_error = None;
        let snapshot = cache.snapshots.get(cwd).cloned().unwrap_or_else(|| Snapshot {
            fingerprint: None,
            status: GitDetailedStatus::default(),
            sync: git::sync::SyncInfo::default(),
            history: Vec::new(),
            branches: Vec::new(),
            changes: Vec::new(),
            worktrees: Vec::new(),
            repo: None,
        });
        self.apply_snapshot(cwd.to_string(), snapshot, cx);
        cx.notify();
    }

    fn apply_snapshot(&mut self, cwd: String, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.git_status = snapshot.status;
        self.file_tree.invalidate_tints();
        self.git_sync = snapshot.sync;
        self.git_history = snapshot.history;
        self.changes_ui.invalidate_graph();
        let cache = &mut self.workspace;
        cache.cwd = cwd;
        cache.fingerprint = snapshot.fingerprint;
        cache.branches = snapshot.branches;
        cache.changes = snapshot.changes;
        cache.worktrees = snapshot.worktrees;
        cache.repo = snapshot.repo;
        self.refresh_branch_pr(cx);
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
                    // A load in flight already reads the newer state;
                    // restarting it would keep a slow repo from landing.
                    let settled = app.workspace.loaded_generation == app.workspace.generation;
                    if same_dir
                        && settled
                        && fingerprint.is_some()
                        && fingerprint != app.workspace.fingerprint
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

    /// Fetches the open directory's remote every `AUTO_FETCH_INTERVAL`, so
    /// new remote commits show as behind and offer Sync Changes.
    pub fn start_auto_fetch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTO_FETCH_TICK).await;
                if this
                    .update(cx, |app, cx| app.auto_fetch(AUTO_FETCH_INTERVAL, cx))
                    .is_err()
                {
                    return; // app dropped
                }
            }
        })
        .detach();
    }

    /// The window came back to the front: catch up on remote commits.
    pub fn auto_fetch_on_focus(&mut self, cx: &mut Context<Self>) {
        self.auto_fetch(FOCUS_FETCH_AGE, cx);
    }

    /// Fetches the open directory when its last fetch is older than
    /// `min_age`, then reloads the snapshot if a remote ref moved.
    fn auto_fetch(&mut self, min_age: Duration, cx: &mut Context<Self>) {
        let cwd = self.workspace.cwd.clone();
        let fresh = self
            .workspace
            .fetched_at
            .get(&cwd)
            .is_some_and(|at| at.elapsed() < min_age);
        // A Pull / Push / Sync the user started already talks to the
        // remote, and two fetches would fight over the ref locks.
        if cwd.is_empty()
            || fresh
            || self.workspace.fetching
            || self.git_sync.remote.is_none()
            || self.changes_ui.busy.is_some()
        {
            return;
        }
        self.workspace.fetching = true;
        // Stamped before it runs, so a failing remote waits a full period too.
        self.workspace.fetched_at.insert(cwd.clone(), Instant::now());
        let fetch_cwd = cwd.clone();
        let task = cx
            .background_executor()
            .spawn(async move { git::sync::auto_fetch(&fetch_cwd) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.workspace.fetching = false;
                match result {
                    Ok(true) if app.workspace.cwd == cwd => app.refresh_workspace(cx),
                    Ok(_) => {}
                    // Offline or signed out: expected, and nothing to show.
                    Err(err) => log::info!("auto-fetch in {cwd} failed: {err}"),
                }
            });
            if let Err(err) = landed {
                log::debug!("auto-fetch after app drop: {err:#}");
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

    /// Checks out `target` off the UI thread (MonoCode `BranchPicker`).
    /// Local changes in the way open the "Uncommitted changes" dialog.
    pub fn switch_to_branch(
        &mut self,
        target: BranchTarget,
        stash_first: bool,
        cx: &mut Context<Self>,
    ) {
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
                        app.finish_branch_switch(None, cx);
                    }
                    Err(git::SwitchError::BlockedByChanges) => {
                        // MonoCode trades the popover for the dialog.
                        app.close_branch_picker(false, cx);
                        app.blocked_branch_switch = Some(target);
                    }
                    Err(git::SwitchError::Failed(err)) => {
                        log::error!("branch switch failed: {err:#}");
                        // Shown in the popover when the switch came from it.
                        if !app.finish_branch_switch(Some(format!("{err:#}")), cx) {
                            app.workspace.git_error =
                                Some(format!("Switch branch failed: {err:#}"));
                        }
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
