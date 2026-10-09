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
    fingerprint: Option<git::StateFingerprint>,
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

#[derive(Clone, Default)]
struct Snapshot {
    fingerprint: Option<git::StateFingerprint>,
    status: GitDetailedStatus,
    changes: Vec<GitFileChange>,
    refs: RefsState,
}

/// The parts of a snapshot that only change when refs or config move. A new
/// snapshot field goes here when it depends on refs alone, else it is read
/// with `read_local_state`; the wrong home shows stale data.
#[derive(Clone, Default)]
struct RefsState {
    sync: git::sync::SyncInfo,
    history: Vec<git::sync::HistoryCommit>,
    branches: Vec<git::Branch>,
    worktrees: Vec<crate::git::Worktree>,
    repo: Option<String>,
}

/// How much of the workspace a refresh reloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RefreshScope {
    /// History, branches, worktrees, sync info and the repo name.
    pub refs: bool,
    /// The `git ls-files` index behind Go to File and `@` mentions.
    pub file_index: bool,
}

impl RefreshScope {
    pub(crate) const FULL: Self = Self {
        refs: true,
        file_index: true,
    };
}

/// What a poll that saw `new` must reload, given the loaded `old`; `None`
/// when nothing changed.
fn scope_for(
    old: Option<git::StateFingerprint>,
    new: git::StateFingerprint,
) -> Option<RefreshScope> {
    let Some(old) = old else {
        return Some(RefreshScope::FULL);
    };
    if old.refs != new.refs {
        Some(RefreshScope::FULL)
    } else if old.paths != new.paths {
        Some(RefreshScope {
            refs: false,
            file_index: true,
        })
    } else if old.tree != new.tree {
        Some(RefreshScope {
            refs: false,
            file_index: false,
        })
    } else {
        None
    }
}

/// Runs the snapshot's git reads side by side: each is its own `git`
/// process, so the wall time is the slowest one rather than their sum.
/// With `reuse` (the refs state of the last snapshot and the refs
/// fingerprint it was loaded for) only the status is read.
fn load_snapshot(cwd: &str, reuse: Option<(RefsState, u64)>) -> Snapshot {
    if let Some((refs, refs_fingerprint)) = reuse {
        let local = git::read_local_state(cwd);
        return Snapshot {
            // The refs part stays the one `refs` was read for, so a ref
            // that moved since still shows as a change to the next poll.
            fingerprint: local.fingerprint.map(|fingerprint| git::StateFingerprint {
                refs: refs_fingerprint,
                ..fingerprint
            }),
            status: local.status,
            changes: local.changes,
            refs,
        };
    }
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
            changes: local.changes,
            refs: RefsState {
                sync: joined(sync, "sync info"),
                history: joined(history, "history"),
                branches: joined(branches, "branches"),
                worktrees: joined(worktrees, "worktrees"),
                repo: joined(repo, "repo name"),
            },
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

    /// Reloads everything the workspace views show.
    pub fn refresh_workspace(&mut self, cx: &mut Context<Self>) {
        self.refresh_workspace_scoped(RefreshScope::FULL, cx);
    }

    /// The poll's refresh: only what `scope` says changed is read again.
    fn refresh_workspace_scoped(&mut self, mut scope: RefreshScope, cx: &mut Context<Self>) {
        self.refresh_skills(false, cx);
        let cwd = self.workspace_cwd();
        if self.workspace.cwd != cwd {
            self.show_cached_snapshot(&cwd, cx);
            scope = RefreshScope::FULL;
        }
        if scope.file_index {
            // MonoCode keeps the composer's file index for its folder, so `@`
            // labels paint before the picker is ever opened.
            self.index_project_files(cx);
        }
        // Without a snapshot that knows its refs, everything is read.
        let reuse = (!scope.refs)
            .then(|| self.workspace.snapshots.get(&cwd))
            .flatten()
            .and_then(|last| Some((last.refs.clone(), last.fingerprint?.refs)));
        let refs_reloaded = reuse.is_none();
        self.workspace.generation += 1;
        let generation = self.workspace.generation;
        let load_cwd = cwd.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load_snapshot(&load_cwd, reuse) });

        cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.workspace.generation != generation {
                    return; // a newer refresh superseded this one
                }
                app.workspace.loaded_generation = generation;
                app.workspace.snapshots.insert(cwd.clone(), snapshot.clone());
                app.apply_snapshot(cwd.clone(), snapshot, refs_reloaded, cx);
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
        let snapshot = cache.snapshots.get(cwd).cloned().unwrap_or_default();
        self.apply_snapshot(cwd.to_string(), snapshot, true, cx);
        cx.notify();
    }

    /// `refs_changed`: the snapshot's refs state is not the one shown, so
    /// the commit graph is laid out again.
    fn apply_snapshot(
        &mut self,
        cwd: String,
        snapshot: Snapshot,
        refs_changed: bool,
        cx: &mut Context<Self>,
    ) {
        self.git_status = snapshot.status;
        self.follow_checkout_branch(&cwd);
        self.file_tree.invalidate_tints();
        let cache = &mut self.workspace;
        cache.cwd = cwd;
        cache.fingerprint = snapshot.fingerprint;
        cache.changes = snapshot.changes;
        if refs_changed {
            let refs = snapshot.refs;
            self.git_sync = refs.sync;
            self.git_history = refs.history;
            self.changes_ui.invalidate_graph();
            cache.branches = refs.branches;
            cache.worktrees = refs.worktrees;
            cache.repo = refs.repo;
        }
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
                        && let Some(new) = fingerprint
                        && let Some(scope) = scope_for(app.workspace.fingerprint, new)
                    {
                        app.refresh_workspace_scoped(scope, cx);
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

    /// A thread with no prompt yet shows the branch its checkout is on now,
    /// as the composer's branch chip acts on that checkout.
    fn follow_checkout_branch(&mut self, cwd: &str) {
        let branch = Some(self.git_status.branch.clone()).filter(|b| !b.is_empty());
        if branch.is_none() {
            return;
        }
        let drafts: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.branch != branch && crate::app::same_project_path(s.work_dir(), cwd))
            .filter(|s| crate::app::tab_scope::is_blank_session(s, self.is_agent_running_in(&s.id)))
            .map(|s| s.id.clone())
            .collect();
        for id in drafts {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                session.branch = branch.clone();
            }
            self.persist_session(&id);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(refs: u64, paths: u64, tree: u64) -> git::StateFingerprint {
        git::StateFingerprint { refs, paths, tree }
    }

    #[test]
    fn a_first_poll_reloads_everything() {
        assert_eq!(scope_for(None, fingerprint(1, 1, 1)), Some(RefreshScope::FULL));
    }

    #[test]
    fn moved_refs_reload_everything() {
        let old = Some(fingerprint(1, 1, 1));
        assert_eq!(scope_for(old, fingerprint(2, 2, 2)), Some(RefreshScope::FULL));
        assert_eq!(scope_for(old, fingerprint(2, 1, 1)), Some(RefreshScope::FULL));
    }

    #[test]
    fn a_new_or_removed_path_reindexes_files_only() {
        let scope = scope_for(Some(fingerprint(1, 1, 1)), fingerprint(1, 2, 2));
        assert_eq!(
            scope,
            Some(RefreshScope {
                refs: false,
                file_index: true
            })
        );
    }

    #[test]
    fn a_content_change_reads_the_status_only() {
        let scope = scope_for(Some(fingerprint(1, 1, 1)), fingerprint(1, 1, 2));
        assert_eq!(
            scope,
            Some(RefreshScope {
                refs: false,
                file_index: false
            })
        );
    }

    #[test]
    fn an_unchanged_repository_reloads_nothing() {
        assert_eq!(scope_for(Some(fingerprint(1, 1, 1)), fingerprint(1, 1, 1)), None);
    }
}
