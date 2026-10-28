//! MonoCode `GitChangesPanel` actions: stage, discard, commit (and amend),
//! push, pull, sync, pull requests, and the branch's PR lookup. The panel's
//! view is `ui/git_changes_panel`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::git::sync as git_sync;
use crate::git::{GitFileStatus, stage_all, stage_file, unstage_all, unstage_file};
use crate::ui::git_changes_panel::{Busy, GitConfirm, GitError, GitFailure, PendingCommit, Side};

/// MonoCode clears its status line after 4s.
const STATUS_FOR: Duration = Duration::from_secs(4);

/// How often opening the Changes tab may read the branch's pull request again.
const PR_READ_AGE: Duration = Duration::from_secs(10);

/// Where a git action runs: the folder git starts in, and the checkout
/// (`WorkspaceCache::checkout_key`) its busy state and draft are kept by.
#[derive(Clone)]
pub(crate) struct GitTarget {
    cwd: String,
    key: String,
}

impl BenCodeApp {
    pub(crate) fn on_default_branch(&self) -> bool {
        let sync = &self.git_sync;
        sync.branch.is_some() && sync.branch == sync.default_branch
    }

    pub(crate) fn has_changes(&self) -> bool {
        !self.git_status.staged.is_empty() || !self.git_status.unstaged.is_empty()
    }

    pub(crate) fn can_commit(&self, cx: &gpui::App) -> bool {
        (!self.git_status.staged.is_empty() || self.changes_ui.amend.is_some())
            && !self.git_commit_input.read(cx).text().trim().is_empty()
            && self.changes_ui.busy.is_none()
    }

    pub(crate) fn has_open_pr(&self) -> bool {
        self.changes_ui
            .pr
            .as_ref()
            .is_some_and(|pr| pr.state == "open")
    }

    /// MonoCode `canCommitPush` and `canCommitPushPr`.
    pub(crate) fn commit_push_allowed(&self, cx: &gpui::App) -> (bool, bool) {
        let sync = &self.git_sync;
        let amend = self.changes_ui.amend.is_some();
        let diverged = sync.ahead > 0 && sync.behind > 0;
        let push = self.can_commit(cx)
            && sync.remote.is_some()
            && !diverged
            && (!amend || !sync.head_pushed);
        (
            push,
            push && !self.has_open_pr() && !self.on_default_branch(),
        )
    }

    /// MonoCode `canCreatePr`.
    pub(crate) fn can_create_pr(&self) -> bool {
        let sync = &self.git_sync;
        sync.remote.is_some()
            && sync.branch.is_some()
            && sync.default_branch.is_some()
            && !self.has_open_pr()
            && !self.on_default_branch()
            && !(sync.ahead > 0 && sync.behind > 0)
            && !self.has_changes()
            && sync.ahead_of_default > 0
            && sync.behind == 0
    }

    /// Shows `text` beside the header for a few seconds.
    pub(crate) fn set_changes_status(&mut self, text: &str, cx: &mut Context<Self>) {
        self.changes_ui.status = Some(text.to_string());
        self.changes_ui.status_generation += 1;
        let generation = self.changes_ui.status_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_FOR).await;
            let cleared = this.update(cx, |app, cx| {
                if app.changes_ui.status_generation == generation {
                    app.changes_ui.status = None;
                    cx.notify();
                }
            });
            if let Err(err) = cleared {
                log::debug!("status cleared after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Reads the branch's pull request when the branch changed (MonoCode
    /// `usePrStatus`); `gh` is slow, so it never blocks the snapshot.
    pub fn refresh_branch_pr(&mut self, cx: &mut Context<Self>) {
        let Some(branch) = self.git_sync.branch.clone() else {
            self.changes_ui.pr = None;
            self.changes_ui.pr_key = None;
            return;
        };
        let cwd = self.workspace(cx).cwd.clone();
        let key = format!("{cwd}\0{branch}");
        if self.changes_ui.pr_key.as_deref() == Some(key.as_str()) {
            return;
        }
        // Another project or branch: never the last one's pull request
        // while `gh` reads this one's.
        let known = self.changes_ui.pr_known.get(&key).cloned().flatten();
        if self.changes_ui.pr != known {
            self.changes_ui.pr = known;
            cx.notify();
        }
        self.load_branch_pr(key, cwd, cx);
    }

    pub(crate) fn load_branch_pr(&mut self, key: String, cwd: String, cx: &mut Context<Self>) {
        // A pull request does not disappear once opened: a miss for a
        // branch that had one is `gh` failing (offline, signed out), and
        // keeps what was found.
        self.changes_ui.pr_key = Some(key.clone());
        self.changes_ui.pr_read_at = Some(std::time::Instant::now());
        let task = cx
            .background_executor()
            .spawn(async move { git_sync::pr_status(&cwd) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let updated = this.update(cx, |app, cx| {
                let known = app.changes_ui.pr_known.get(&key).cloned().flatten();
                let pr = found.or(known);
                app.changes_ui.pr_known.insert(key.clone(), pr.clone());
                if app.changes_ui.pr_key.as_deref() == Some(key.as_str()) && app.changes_ui.pr != pr
                {
                    app.changes_ui.pr = pr;
                    cx.notify();
                }
            });
            if let Err(err) = updated {
                log::debug!("pr status after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Opening the Changes tab: a pull request opened on GitHub since the
    /// last read shows.
    pub(crate) fn reload_branch_pr_if_stale(&mut self, cx: &mut Context<Self>) {
        let fresh = self
            .changes_ui
            .pr_read_at
            .is_some_and(|at| at.elapsed() < PR_READ_AGE);
        if !fresh && self.git_sync.remote.is_some() {
            self.reload_branch_pr(cx);
        }
    }

    pub(crate) fn reload_branch_pr(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.changes_ui.pr_key.clone() {
            let cwd = self.workspace(cx).cwd.clone();
            self.load_branch_pr(key, cwd, cx);
        }
    }

    /// What the panel shows: the folder git runs in and its checkout.
    /// `None` until the folder's snapshot knows its checkout: a key
    /// guessed before then could change under an action in flight.
    fn shown_target(&self, cx: &gpui::App) -> Option<GitTarget> {
        let cache = self.workspace(cx);
        let key = cache.known_checkout(&cache.cwd)?.to_string();
        Some(GitTarget {
            cwd: cache.cwd.clone(),
            key,
        })
    }

    /// Whether the panel shows the checkout `key`.
    fn shows(&self, key: &str) -> bool {
        self.changes_ui.shown_key == key
    }

    /// Marks `key`'s action in flight (or none); the panel's `busy`
    /// follows it while `key` is shown.
    fn set_running(&mut self, key: &str, busy: Option<Busy>) {
        if self.shows(key) {
            self.changes_ui.busy = busy.clone();
        }
        match busy {
            Some(busy) => self.changes_ui.running.insert(key.to_string(), busy),
            None => self.changes_ui.running.remove(key),
        };
    }

    /// `key`'s commit message: the field while it is shown, else its draft.
    fn commit_message_in(&self, key: &str, cx: &gpui::App) -> String {
        if self.shows(key) {
            self.git_commit_input.read(cx).text().to_string()
        } else {
            self.changes_ui.drafts.get(key).cloned().unwrap_or_default()
        }
    }

    fn set_commit_message_in(&mut self, key: &str, message: String, cx: &mut Context<Self>) {
        if self.shows(key) {
            self.git_commit_input
                .update(cx, |input, cx| input.set_text(message, cx));
        } else if message.is_empty() {
            self.changes_ui.drafts.remove(key);
        } else {
            self.changes_ui.drafts.insert(key.to_string(), message);
        }
    }

    /// A failure in `key`: shown in the panel, or kept until `key` is.
    fn show_git_error_in(&mut self, key: String, error: GitError, cx: &mut Context<Self>) {
        if self.shows(&key) {
            self.workspace_state.as_mut(cx).git_error = Some(error);
        } else {
            self.changes_ui.missed_errors.insert(key, error);
        }
    }

    /// Runs `work` off the UI thread as `busy`; a failure shows in the
    /// panel, success may set a status. The snapshot reloads either way.
    pub(crate) fn run_changes_action(
        &mut self,
        busy: Busy,
        done: Option<&'static str>,
        work: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
        after: impl FnOnce(&mut Self, &GitTarget, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.shown_target(cx) {
            self.run_changes_in(target, busy, done, work, after, cx);
        }
    }

    /// `run_changes_action` in `target`, shown or not (the next step of an
    /// action that started there).
    fn run_changes_in(
        &mut self,
        target: GitTarget,
        busy: Busy,
        done: Option<&'static str>,
        work: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
        after: impl FnOnce(&mut Self, &GitTarget, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.run_changes_steps(
            target,
            busy,
            done,
            move |cwd| work(cwd).map_err(GitFailure::from),
            after,
            cx,
        );
    }

    /// `run_changes_action` for work of more than one step, whose failure
    /// names the step that failed rather than the whole action. It keeps
    /// running when another project is shown, and `after` gets the
    /// project it ran in.
    fn run_changes_steps(
        &mut self,
        target: GitTarget,
        busy: Busy,
        done: Option<&'static str>,
        work: impl FnOnce(&str) -> Result<(), GitFailure> + Send + 'static,
        after: impl FnOnce(&mut Self, &GitTarget, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        // One action at a time per checkout, whichever folder of it started.
        if self.changes_ui.running.contains_key(&target.key) {
            return;
        }
        self.set_running(&target.key, Some(busy.clone()));
        if self.shows(&target.key) {
            self.workspace_state.as_mut(cx).git_error = None;
        }
        self.changes_ui.missed_errors.remove(&target.key);
        self.changes_ui.missed_status.remove(&target.key);
        let work_cwd = target.cwd.clone();
        let task = cx
            .background_executor()
            .spawn(async move { work(&work_cwd) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                let key = target.key.clone();
                app.set_running(&key, None);
                match result {
                    Ok(()) => {
                        if let Some(done) = done {
                            if app.shows(&key) {
                                app.set_changes_status(done, cx);
                            } else {
                                app.changes_ui.missed_status.insert(key.clone(), done);
                            }
                        }
                        after(app, &target, cx);
                    }
                    Err(failure) => {
                        log::warn!("git action failed in {}: {}", target.cwd, failure.output());
                        app.show_git_error_in(key.clone(), failure.into_error(&busy), cx);
                    }
                }
                if app.shows(&key) {
                    app.refresh_workspace(cx);
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("git action after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn file_action(
        &mut self,
        path: String,
        side: Side,
        discard: bool,
        cx: &mut Context<Self>,
    ) {
        if discard {
            let untracked = self
                .git_status
                .unstaged
                .iter()
                .any(|f| f.path == path && f.status == GitFileStatus::Untracked);
            self.git_confirm = Some(GitConfirm::DiscardFile { path, untracked });
            cx.notify();
            return;
        }
        let file = path.clone();
        self.run_changes_action(
            Busy::File(path),
            None,
            move |cwd| {
                match side {
                    Side::Unstaged => stage_file(cwd, &file),
                    Side::Staged => unstage_file(cwd, &file),
                }
                .map_err(|err| format!("{err:#}"))
            },
            |_, _, _| {},
            cx,
        );
    }

    /// MonoCode `runFolder`: stages or unstages everything under a folder.
    pub(crate) fn folder_action(&mut self, dir: String, side: Side, cx: &mut Context<Self>) {
        let target = dir.clone();
        self.run_changes_action(
            Busy::Folder(dir),
            None,
            move |cwd| {
                match side {
                    Side::Unstaged => stage_file(cwd, &target),
                    Side::Staged => unstage_file(cwd, &target),
                }
                .map_err(|err| format!("{err:#}"))
            },
            |_, _, _| {},
            cx,
        );
    }

    /// Asks before discarding every unstaged change under a folder.
    pub(crate) fn discard_folder_action(&mut self, dir: String, cx: &mut Context<Self>) {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        let files = self
            .git_status
            .unstaged
            .iter()
            .filter(|f| f.path.starts_with(&prefix))
            .count();
        self.git_confirm = Some(GitConfirm::DiscardFolder { path: dir, files });
        cx.notify();
    }

    pub(crate) fn all_action(&mut self, stage: bool, cx: &mut Context<Self>) {
        self.run_changes_action(
            Busy::All,
            None,
            move |cwd| {
                if stage {
                    stage_all(cwd)
                } else {
                    unstage_all(cwd)
                }
                .map_err(|e| format!("{e:#}"))
            },
            |_, _, _| {},
            cx,
        );
    }

    /// MonoCode `onOpenWorkingTreeDiff`: the file's review, as a preview
    /// tab unless `pin` (a double click).
    pub(crate) fn open_change(
        &mut self,
        path: String,
        side: Side,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        let deleted = match side {
            Side::Staged => &self.git_status.staged,
            Side::Unstaged => &self.git_status.unstaged,
        }
        .iter()
        .any(|f| f.path == path && f.status == GitFileStatus::Deleted);
        if deleted {
            return;
        }
        let cwd = self.workspace(cx).cwd.clone();
        self.open_pane_tab(PaneTab::Review { cwd, path, side }, pin, cx);
    }

    /// MonoCode "Open All Changes": that section's files stacked in one review.
    pub(crate) fn open_all_changes(&mut self, side: Side, cx: &mut Context<Self>) {
        let tab = PaneTab::Changes {
            cwd: self.workspace(cx).cwd.clone(),
            side: Some(side),
            focus: None,
        };
        self.open_pane_tab(tab, false, cx);
    }

    /// The commit field's Submit (⌘↩).
    pub fn commit_staged_changes(&mut self, cx: &mut Context<Self>) {
        self.commit_from_panel(
            PendingCommit {
                push: false,
                pr: false,
            },
            false,
            false,
            cx,
        );
    }

    /// MonoCode `commit(push, createPr)`: asks before pushing the default
    /// branch or amending a pushed commit, then commits, pushes, opens a PR.
    pub(crate) fn commit_from_panel(
        &mut self,
        pending: PendingCommit,
        default_ok: bool,
        amend_ok: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.can_commit(cx) {
            return;
        }
        if (pending.push || pending.pr) && !default_ok && self.on_default_branch() {
            self.git_confirm = Some(GitConfirm::PushDefault(pending));
            cx.notify();
            return;
        }
        if self.changes_ui.amend.is_some() && self.git_sync.head_pushed && !amend_ok {
            self.git_confirm = Some(GitConfirm::AmendPushed(pending));
            cx.notify();
            return;
        }
        let message = self.git_commit_input.read(cx).text().to_string();
        let amend = self.changes_ui.amend.is_some();
        let Some(target) = self.shown_target(cx) else {
            return;
        };
        self.run_changes_steps(
            target,
            if pending.pr { Busy::Pr } else { Busy::Commit },
            None,
            move |cwd| {
                git_sync::commit(cwd, &message, amend)
                    .map_err(|err| GitFailure::titled("Couldn't commit", err))?;
                if pending.push || pending.pr {
                    // The commit is made: saying it failed invites a second.
                    git_sync::push(cwd)
                        .map_err(|err| GitFailure::titled("Committed, but couldn't push", err))?;
                }
                Ok(())
            },
            move |app, target, cx| {
                app.set_commit_message_in(&target.key, String::new(), cx);
                if app.shows(&target.key) {
                    app.changes_ui.amend = None;
                }
                if pending.pr {
                    app.open_created_pr(target.clone(), cx);
                }
            },
            cx,
        );
    }

    /// MonoCode `openCreatedPr`: Claude writes the PR, `gh` opens it.
    pub(crate) fn open_created_pr(&mut self, target: GitTarget, cx: &mut Context<Self>) {
        // Opened from the UI thread with GPUI's `open_url` once the PR exists.
        let created: Arc<std::sync::Mutex<Option<String>>> = Default::default();
        let slot = created.clone();
        self.run_changes_in(
            target,
            Busy::Pr,
            None,
            move |cwd| {
                let content = crate::git::text::generate_pr_content(cwd)?;
                let url = git_sync::pr_create(
                    cwd,
                    &content.title,
                    &content.body,
                    &content.base,
                    &content.head,
                )?;
                let url = url.trim().to_string();
                if url.starts_with("https://") || url.starts_with("http://") {
                    *slot.lock().map_err(|err| err.to_string())? = Some(url);
                }
                Ok(())
            },
            move |app, target, cx| {
                let url = created.lock().ok().and_then(|mut url| url.take());
                if let Some(url) = url {
                    cx.open_url(&url);
                }
                // Another checkout's pull request is read when it is shown.
                if app.shows(&target.key) {
                    app.reload_branch_pr(cx);
                }
            },
            cx,
        );
    }

    /// MonoCode `createPr`: pushes what is ahead, then opens the PR.
    pub(crate) fn create_pr(&mut self, default_ok: bool, cx: &mut Context<Self>) {
        if !self.can_create_pr() {
            return;
        }
        if !default_ok && self.on_default_branch() {
            self.git_confirm = Some(GitConfirm::CreatePrDefault);
            cx.notify();
            return;
        }
        let ahead = self.git_sync.ahead > 0;
        self.run_changes_action(
            Busy::Pr,
            None,
            move |cwd| if ahead { git_sync::push(cwd) } else { Ok(()) },
            |app, target, cx| app.open_created_pr(target.clone(), cx),
            cx,
        );
    }

    pub(crate) fn sync_changes(&mut self, cx: &mut Context<Self>) {
        self.run_changes_action(
            Busy::Sync,
            None,
            git_sync::sync,
            |app, target, cx| {
                if app.shows(&target.key) {
                    app.reload_branch_pr(cx);
                }
            },
            cx,
        );
    }

    /// VS Code "Undo Last Commit": HEAD comes off the branch, its changes
    /// stay staged and its message goes back to an empty commit box. One
    /// that is already pushed asks first (`pushed_ok` once it has).
    pub(crate) fn undo_last_commit(&mut self, pushed_ok: bool, cx: &mut Context<Self>) {
        if self.git_sync.head_pushed && !pushed_ok {
            self.git_confirm = Some(GitConfirm::UndoPushed);
            cx.notify();
            return;
        }
        let undone = std::sync::Arc::new(std::sync::Mutex::new(None));
        let kept = undone.clone();
        self.run_changes_action(
            Busy::Undo,
            Some("Undid the last commit"),
            move |cwd| {
                let message = git_sync::head_message(cwd).ok();
                git_sync::undo_last_commit(cwd)?;
                if let Ok(mut slot) = kept.lock() {
                    *slot = message;
                }
                Ok(())
            },
            move |app, target, cx| {
                let message = undone.lock().ok().and_then(|mut slot| slot.take());
                if let Some(message) = message
                    && app.commit_message_in(&target.key, cx).trim().is_empty()
                {
                    app.set_commit_message_in(&target.key, message, cx);
                }
            },
            cx,
        );
    }

    /// A new commit that takes back what `sha` changed.
    pub(crate) fn revert_commit(&mut self, sha: String, cx: &mut Context<Self>) {
        self.run_changes_action(
            Busy::Revert,
            Some("Reverted the commit"),
            move |cwd| git_sync::revert_commit(cwd, &sha),
            |_, _, _| {},
            cx,
        );
    }

    /// `sha`'s whole message on the clipboard.
    pub(crate) fn copy_commit_message(&mut self, sha: String, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        let task = cx
            .background_executor()
            .spawn(async move { git_sync::commit_message(&cwd, &sha) });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let updated = this.update(cx, |app, cx| match message {
                Ok(message) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(message)),
                Err(err) => {
                    app.workspace_state.as_mut(cx).git_error =
                        Some(GitError::new("Couldn't read the commit message", err));
                    cx.notify();
                }
            });
            if let Err(err) = updated {
                log::debug!("commit message after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub(crate) fn pull_changes(&mut self, cx: &mut Context<Self>) {
        self.run_changes_action(
            Busy::Pull,
            Some("Pull complete"),
            git_sync::pull,
            |_, _, _| {},
            cx,
        );
    }

    /// MonoCode `toggleAmend`: HEAD's message fills an empty field.
    pub(crate) fn toggle_amend(&mut self, cx: &mut Context<Self>) {
        if self.changes_ui.amend.take().is_some() {
            cx.notify();
            return;
        }
        let Some(GitTarget { cwd, key }) = self.shown_target(cx) else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { git_sync::head_message(&cwd) });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let updated = this.update(cx, |app, cx| {
                // Amending is the shown checkout's; another is shown now.
                if !app.shows(&key) {
                    return;
                }
                match message {
                    Ok(message) => {
                        if app.git_commit_input.read(cx).text().trim().is_empty() {
                            app.git_commit_input
                                .update(cx, |input, cx| input.set_text(message, cx));
                        }
                        app.changes_ui.amend =
                            Some((app.git_sync.branch.clone(), app.git_sync.head.clone()));
                    }
                    Err(err) => {
                        app.workspace_state.as_mut(cx).git_error =
                            Some(GitError::new("Couldn't amend", err))
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("amend after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// ✨: Claude writes the message; a second press cancels. The message
    /// lands in its checkout's commit field, shown or not.
    pub(crate) fn toggle_generate(&mut self, cx: &mut Context<Self>) {
        let Some(GitTarget { cwd, key }) = self.shown_target(cx) else {
            return;
        };
        if let Some(cancel) = self.changes_ui.generate_cancel.remove(&key) {
            cancel.store(true, Ordering::Relaxed);
            self.set_running(&key, None);
            cx.notify();
            return;
        }
        if self.changes_ui.running.contains_key(&key) || !self.has_changes() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.changes_ui
            .generate_cancel
            .insert(key.clone(), cancel.clone());
        self.set_running(&key, Some(Busy::Generate));
        let flag = cancel.clone();
        let task = cx
            .background_executor()
            .spawn(async move { crate::git::text::generate_commit_message(&cwd, flag) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                app.changes_ui.generate_cancel.remove(&key);
                app.set_running(&key, None);
                match result {
                    Ok(message) => app.set_commit_message_in(&key, message, cx),
                    Err(err) => {
                        app.show_git_error_in(key, GitError::for_busy(&Busy::Generate, err), cx)
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("commit message after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode's amend ends when the branch or HEAD moves.
    pub(crate) fn check_amend_target(&mut self) {
        if let Some((branch, head)) = &self.changes_ui.amend
            && (branch != &self.git_sync.branch || head != &self.git_sync.head)
        {
            self.changes_ui.amend = None;
        }
    }
}
