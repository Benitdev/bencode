//! Runs scheduled automations, as MonoCode's App does: every 30 seconds it
//! claims each due occurrence (`claimDueAutomations`) and starts the prompt
//! in a new thread without leaving the current view. Runs a previous
//! BenCode left open are closed at startup (`automation_runs_recover`).

use std::time::Duration;

use gpui::Context;
use jiff::tz::TimeZone;

use crate::app::{BenCodeApp, now_ms};
use crate::db::{AutomationRow, AutomationRunRow, DEFAULT_GRACE_MINUTES};
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

impl BenCodeApp {
    /// Closes stale runs, then checks for due automations now and every
    /// `POLL_INTERVAL`.
    pub fn start_automation_scheduler(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        match self.db.recover_automation_runs(now, now) {
            Ok(0) => {}
            Ok(closed) => log::info!("closed {closed} automation runs left open"),
            Err(err) => log::error!("could not recover automation runs: {err:#}"),
        }
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

    fn run_due_automations(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        // Read the table each time: MonoCode may have edited it.
        let automations = match self.db.list_automations() {
            Ok(list) => list,
            Err(err) => {
                log::error!("could not list automations: {err:#}");
                return;
            }
        };
        let mut claimed_any = false;
        for auto in automations
            .iter()
            .filter(|a| a.enabled && a.next_run_at > 0 && a.next_run_at <= now)
        {
            claimed_any |= self.claim_and_launch(auto, now, cx);
        }
        if claimed_any {
            self.refresh_automations(cx);
        }
    }

    /// Claims `auto`'s due occurrence; starts it unless it was skipped.
    fn claim_and_launch(&mut self, auto: &AutomationRow, now: i64, cx: &mut Context<Self>) -> bool {
        let tz = TimeZone::system();
        let Some(next) = schedule::next_automation_run_at(auto, now, &tz) else {
            log::warn!("automation {} has an unreadable schedule", auto.id);
            return false;
        };
        let grace = grace_minutes(auto);
        let claim = self
            .db
            .claim_due_automation(&auto.id, auto.next_run_at, next, now, grace);
        match claim {
            Ok(Some(run)) => {
                if run.status == "pending" {
                    self.launch_scheduled_run(auto, &run, cx);
                }
                true
            }
            Ok(None) => false,
            Err(err) => {
                log::error!("could not claim automation {}: {err:#}", auto.id);
                false
            }
        }
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
        if !self.is_agent_running_in(&session_id) {
            // send_prompt already explains the failure inside the thread.
            let error = Some("The agent could not start.");
            if let Err(err) = self.db.finish_automation_run(&run.id, "failed", error) {
                log::error!("could not close automation run {}: {err:#}", run.id);
            }
            return;
        }
        if let Err(err) = self.db.start_automation_run(&run.id, &session_id, now_ms()) {
            log::error!("could not mark automation run {} started: {err:#}", run.id);
        }
        self.attach_automation_run(&session_id, run.id.clone());
    }
}
