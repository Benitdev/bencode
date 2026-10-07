//! Runs scheduled automations, as MonoCode's App does: every 30 seconds it
//! claims each due occurrence (`claimDueAutomations`) and starts the prompt
//! in a new thread without leaving the current view. Runs a previous
//! BenCode left open are closed at startup (`automation_runs_recover`).

use std::time::Duration;

use gpui::Context;
use jiff::tz::TimeZone;

use crate::app::{BenCodeApp, now_ms};
use crate::db::{AppDb, AutomationRow, AutomationRunRow, DEFAULT_GRACE_MINUTES};
use crate::harness::catalog;
use crate::schedule;

/// MonoCode polls due automations this often.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

fn grace_minutes(auto: &AutomationRow) -> i64 {
    auto.extra
        .get("missedRunGraceMinutes")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_GRACE_MINUTES)
}

/// MonoCode `claimDueAutomations`: claims every due occurrence and returns
/// what to start. Runs on the database writer, not the UI thread.
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
        let claimed = self.db_read(move |db| claim_due(db, now));
        cx.spawn(async move |this, cx| {
            let claimed = match claimed.await {
                Ok(Ok(claimed)) => claimed,
                Ok(Err(err)) => {
                    log::error!("could not claim due automations: {err:#}");
                    return;
                }
                Err(_) => {
                    log::error!("the database writer stopped");
                    return;
                }
            };
            if claimed.is_empty() {
                return;
            }
            let landed = this.update(cx, |app, cx| {
                for (auto, run) in &claimed {
                    if run.status == "pending" {
                        app.launch_scheduled_run(auto, run, cx);
                    }
                }
                // The list below reads `db`; let the runs' queued rows land.
                app.settle_db_writes();
                app.refresh_automations(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("automations claimed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Starts `auto` in a new thread that stays in the background.
    fn launch_scheduled_run(
        &mut self,
        auto: &AutomationRow,
        run: &AutomationRunRow,
        cx: &mut Context<Self>,
    ) {
        let session_id = self.create_session_row(&auto.cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.title = auto.name.clone();
            if let Some(option) = catalog::find(&auto.model) {
                session.model = option.key.to_string();
                session.harness = option.harness.id().to_string();
            }
        }
        self.persist_session(&session_id);
        self.send_prompt(&session_id, &auto.prompt, cx);
        let run_id = run.id.clone();
        if !self.is_agent_running_in(&session_id) {
            // send_prompt already explains the failure inside the thread.
            self.db_write("close automation run", move |db| {
                db.finish_automation_run(&run_id, "failed", Some("The agent could not start."))
            });
            return;
        }
        let (thread, now) = (session_id.clone(), now_ms());
        self.db_write("start automation run", move |db| {
            db.start_automation_run(&run_id, &thread, now)
        });
        self.attach_automation_run(&session_id, run.id.clone());
    }
}
