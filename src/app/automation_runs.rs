//! Running automations. The scheduler does what MonoCode's App does: every
//! 30 seconds it claims each due occurrence (`claimDueAutomations`) and
//! starts it in a background thread; runs a previous BenCode left open are
//! closed at startup (`automation_runs_recover`). Run now starts one the
//! same way (`createManualAutomationRun`). A run gets its thread (a new
//! one, or the last run's), its model, access and sidebar folder, and a
//! fresh worktree when the automation asks for one (`launchAutomationRun`).

use std::hash::{Hash, Hasher};
use std::time::Duration;

use gpui::Context;
use jiff::tz::TimeZone;

use super::automations::{WorkingCopy, grace_minutes};
use super::session_folders::{FolderTarget, place_session};
use super::{BenCodeApp, PermissionMode, now_ms, unique_id};
use crate::db::{AppDb, AutomationRow, AutomationRunRow, RunStatus, RunTrigger};
use crate::git::worktrees::{Worktree, create_worktree};
use crate::harness::catalog;
use crate::schedule;
use crate::ui::composer::new_worktree::temporary_branch_name;

/// MonoCode polls due automations this often.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// MonoCode `claimDueAutomations`: claims every due occurrence and returns
/// what to start. Runs on the database thread.
fn claim_due(db: &AppDb, now: i64) -> anyhow::Result<Vec<(AutomationRow, AutomationRunRow)>> {
    let tz = TimeZone::system();
    let mut claimed = Vec::new();
    // Read the table each time: MonoCode may have edited it.
    for auto in db.list_automations()? {
        if !(auto.enabled && auto.next_run_at > 0 && auto.next_run_at <= now) {
            continue;
        }
        let Some(next) = schedule::next_automation_run_at(&auto, now, &tz) else {
            log::warn!("automation {} has an unreadable schedule", auto.id);
            continue;
        };
        let grace = grace_minutes(&auto);
        match db.claim_due_automation(&auto.id, auto.next_run_at, next, now, grace) {
            Ok(Some(run)) => claimed.push((auto, run)),
            Ok(None) => {}
            Err(err) => log::error!("could not claim automation {}: {err:#}", auto.id),
        }
    }
    Ok(claimed)
}

/// The branch of a run's worktree. Seeded by the run, so two automations
/// due in the same tick never ask for the same branch.
fn run_branch(run_id: &str, now: i64) -> String {
    let mut seed = std::collections::hash_map::DefaultHasher::new();
    (run_id, now).hash(&mut seed);
    temporary_branch_name(seed.finish())
}

impl BenCodeApp {
    /// Closes stale runs, then checks for due automations now and every
    /// `POLL_INTERVAL`.
    pub fn start_automation_scheduler(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        // Queued ahead of the first claim, so it still runs first.
        self.db_write("recover automation runs", move |db| {
            let closed = db.recover_automation_runs(now, now)?;
            if closed > 0 {
                log::info!("closed {closed} automation runs left open");
            }
            Ok(())
        });
        self.run_due_automations(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this
                    .update(cx, |app, cx| app.run_due_automations(cx))
                    .is_err()
                {
                    return; // app dropped
                }
            }
        })
        .detach();
    }

    /// Claims the due occurrences off the UI thread, then starts each one
    /// that was not skipped.
    fn run_due_automations(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        self.db_then(
            cx,
            move |db| claim_due(db, now),
            |app, claimed, cx| {
                let claimed = match claimed {
                    Ok(claimed) => claimed,
                    Err(err) => {
                        log::error!("could not claim due automations: {err:#}");
                        return;
                    }
                };
                if claimed.is_empty() {
                    return;
                }
                for (auto, run) in &claimed {
                    if run.status == RunStatus::Pending {
                        app.launch_automation_run(auto, &run.id, cx);
                    }
                }
                app.refresh_automations(cx);
            },
        );
    }

    /// MonoCode `createManualAutomationRun` and its launch: the stored
    /// automation runs in the background and the view stays open.
    pub(crate) fn run_automation_now(&mut self, cx: &mut Context<Self>) {
        let Some(auto) = self.edited_automation().cloned() else {
            return;
        };
        let now = now_ms();
        let run = AutomationRunRow {
            id: unique_id("run"),
            automation_id: auto.id.clone(),
            trigger: RunTrigger::Manual,
            scheduled_for: now,
            created_at: now,
            started_at: None,
            completed_at: None,
            status: RunStatus::Pending,
            session_id: None,
            error: None,
        };
        self.db_then(
            cx,
            move |db| db.create_automation_run(&run).map(|()| run.id),
            move |app, created, cx| match created {
                Ok(run_id) => {
                    app.launch_automation_run(&auto, &run_id, cx);
                    // Queued behind the run's own writes, so it shows them.
                    app.load_automation_runs(&auto.id, cx);
                }
                Err(err) => {
                    log::error!("create_automation_run failed: {err:#}");
                    app.automations.error = Some(format!("Could not start the run: {err}"));
                    cx.notify();
                }
            },
        );
    }

    /// Starts the pending run `run_id` of `auto` in a background thread.
    fn launch_automation_run(
        &mut self,
        auto: &AutomationRow,
        run_id: &str,
        cx: &mut Context<Self>,
    ) {
        if WorkingCopy::of(auto) == WorkingCopy::Current {
            let session_id = self
                .reusable_automation_thread(auto)
                .unwrap_or_else(|| self.new_automation_thread(auto, None, cx));
            self.begin_automation_run(auto, &session_id, run_id, cx);
            return;
        }
        // The worktree comes first: a run that cannot have one leaves no
        // empty thread behind.
        let (project, branch) = (auto.cwd.clone(), run_branch(run_id, now_ms()));
        let create = cx
            .background_executor()
            .spawn(async move { create_worktree(&project, &branch, "HEAD", false) });
        let (auto, run_id) = (auto.clone(), run_id.to_string());
        cx.spawn(async move |this, cx| {
            let created = create.await;
            let landed = this.update(cx, |app, cx| match created {
                Ok(tree) => {
                    let session_id = app.new_automation_thread(&auto, Some(tree), cx);
                    app.begin_automation_run(&auto, &session_id, &run_id, cx);
                }
                Err(err) => {
                    log::warn!("automation {}: no worktree: {err:#}", auto.id);
                    let reason = format!("Could not create the worktree: {err:#}");
                    app.db_write("close automation run", move |db| {
                        db.finish_automation_run(&run_id, RunStatus::Failed, Some(&reason))
                    });
                    app.refresh_automations(cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("automation worktree after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// "Continue last": the thread of the automation's last run, if it is
    /// still loaded and free.
    fn reusable_automation_thread(&self, auto: &AutomationRow) -> Option<String> {
        let id = auto
            .last_session_id
            .as_ref()
            .filter(|_| auto.reuse_session == Some(true))?;
        let free = self.sessions.iter().any(|s| &s.id == id)
            && !self.is_agent_running_in(id)
            && !self.worktree_removed(id);
        free.then(|| id.clone())
    }

    /// A thread for a run of `auto`: named after it, with its model and
    /// access, in `tree` (else the project folder) and filed in its
    /// sidebar folder.
    fn new_automation_thread(
        &mut self,
        auto: &AutomationRow,
        tree: Option<Worktree>,
        cx: &mut Context<Self>,
    ) -> String {
        let session_id = self.create_session_row(&auto.cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.title = auto.name.clone();
            // The worktree a run uses is the automation's, not the one in view.
            session.worktree_cwd = None;
            if let Some(tree) = tree {
                session.worktree_cwd = Some(tree.path);
                session.branch = tree.branch;
            }
            if let Some(option) = catalog::find(&auto.model) {
                session.model = option.key.to_string();
                session.harness = option.harness.id().to_string();
            }
            if let Some(mode) = auto
                .runtime_mode
                .as_deref()
                .and_then(PermissionMode::from_id)
            {
                session.runtime_mode = Some(mode.id().to_string());
            }
        }
        self.persist_session(&session_id);
        let folder = auto
            .session_folder_id
            .as_deref()
            .filter(|id| !id.is_empty());
        if let Some(folder) = folder
            && let Some(folders) = self.session_folders.get(&auto.cwd)
            && folders.iter().any(|f| f.id == folder)
        {
            let target = FolderTarget::Existing(folder.to_string());
            let next = place_session(folders, &session_id, &target);
            self.session_folders.insert(auto.cwd.clone(), next);
            self.save_settings(cx);
        }
        session_id
    }

    /// Sends the automation's instructions and marks the run started, or
    /// failed when the agent could not start.
    fn begin_automation_run(
        &mut self,
        auto: &AutomationRow,
        session_id: &str,
        run_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.send_prompt(session_id, &auto.prompt, cx);
        let (run, thread) = (run_id.to_string(), session_id.to_string());
        if !self.is_agent_running_in(session_id) {
            // send_prompt already explains the failure inside the thread.
            self.db_write("close automation run", move |db| {
                db.finish_automation_run(
                    &run,
                    RunStatus::Failed,
                    Some("The agent could not start."),
                )
            });
            return;
        }
        let now = now_ms();
        self.db_write("start automation run", move |db| {
            db.start_automation_run(&run, &thread, now)
        });
        self.attach_automation_run(session_id, run_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_run_gets_its_own_branch() {
        let first = run_branch("run-1", 1_000);
        assert!(first.starts_with("mc/"), "{first}");
        assert_ne!(first, run_branch("run-2", 1_000));
        assert_eq!(first, run_branch("run-1", 1_000));
    }
}
