//! What the Automations overview and the cards read from the runs
//! `refresh_automations` loads: what needs the user (a failed or missed
//! last run, a prompt waiting), what ran lately, what runs next, each
//! card's status and run strip, and the rail's badge. Kept pure and
//! tested; the views are in `ui/automations/overview.rs` and `list.rs`.
//! BenCode's own: MonoCode lands on the New automation page.

use std::collections::HashMap;

use gpui::Context;
use jiff::tz::TimeZone;

use super::automation_pauses::active_skip;
use super::automations::AutomationPage;
use super::surfaces::Surface;
use super::{AutomationLink, BenCodeApp, now_ms};
use crate::db::{
    ALL_PAUSED, AutomationRow, AutomationRunRow, INTERRUPTED, MISSED, NOT_STARTED, RunStatus,
};
use crate::schedule::{self, ScheduleKind};

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// Runs loaded per automation, whatever their age: the card's strip, and
/// the latest finished one behind a few that are still going.
pub const RUNS_PER_AUTOMATION: i64 = 10;
/// Recent results reach back this far.
pub const RECENT_DAYS: i64 = 7;
/// Most recent results shown.
pub const RECENT_SHOWN: usize = 30;
/// Runs on a card's strip.
pub const STRIP_RUNS: usize = 7;
/// Upcoming runs reach this far ahead.
pub const UPCOMING_MS: i64 = DAY_MS;
/// Most occurrences of one automation looked at for Upcoming.
const UPCOMING_PER_AUTOMATION: usize = 48;

/// Why an automation is under Needs you.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    /// A run waits on a permission prompt.
    Approval,
    /// Its latest run failed.
    Failed,
    /// Its latest run was missed (late beyond its grace, or never started).
    Missed,
    /// Its latest run was cut off when BenCode stopped.
    Interrupted,
}

impl Attention {
    /// The rail icon's tooltip while it shows the badge.
    pub fn rail_tip(self) -> &'static str {
        match self {
            Self::Approval => "Automations, a run needs your approval",
            Self::Failed => "Automations, a run failed",
            Self::Missed => "Automations, a run was missed",
            Self::Interrupted => "Automations, a run was interrupted",
        }
    }
}

/// One entry of Needs you.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsYou {
    pub automation_id: String,
    pub kind: Attention,
    /// The finished run it is about; `None` for a prompt waiting.
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub text: String,
    /// When it happened (epoch ms): newer than the last look badges the rail.
    pub at: i64,
}

/// When `run` ended, else when it was made.
pub fn finished_at(run: &AutomationRunRow) -> i64 {
    run.completed_at.unwrap_or(run.created_at)
}

/// A run Pause all skipped: it says nothing of how the automation does.
fn paused_skip(run: &AutomationRunRow) -> bool {
    run.status == RunStatus::Skipped && run.error.as_deref() == Some(ALL_PAUSED)
}

/// A run that should have run and did not (not a pause, not a user skip).
pub fn missed(run: &AutomationRunRow) -> bool {
    run.status == RunStatus::Skipped && matches!(run.error.as_deref(), Some(MISSED | NOT_STARTED))
}

/// A run BenCode's quitting cut off, unlike one the user stopped.
pub fn interrupted(run: &AutomationRunRow) -> bool {
    run.status == RunStatus::Cancelled && run.error.as_deref() == Some(INTERRUPTED)
}

/// The automation's latest finished run, Pause all's skips aside.
fn latest_finished<'a>(
    recent: &'a [AutomationRunRow],
    automation_id: &str,
) -> Option<&'a AutomationRunRow> {
    recent
        .iter()
        .filter(|run| run.automation_id == automation_id)
        .filter(|run| !run.status.is_live() && !paused_skip(run))
        .max_by_key(|run| run.created_at)
}

/// The automations whose latest finished run failed, was missed or was
/// interrupted, and was not dismissed, newest first. A later run that goes well clears it.
pub fn finished_needs(items: &[AutomationRow], recent: &[AutomationRunRow]) -> Vec<NeedsYou> {
    let mut needs: Vec<NeedsYou> = items
        .iter()
        .filter_map(|auto| {
            let run = latest_finished(recent, &auto.id)?;
            let kind = match run.status {
                RunStatus::Failed => Attention::Failed,
                _ if missed(run) => Attention::Missed,
                _ if interrupted(run) => Attention::Interrupted,
                _ => return None,
            };
            if run.dismissed_at.is_some() {
                return None;
            }
            let text = run.error.clone().unwrap_or_else(|| match kind {
                Attention::Missed => "The run was missed.".to_string(),
                _ => "The run failed.".to_string(),
            });
            Some(NeedsYou {
                automation_id: auto.id.clone(),
                kind,
                run_id: Some(run.id.clone()),
                session_id: run.session_id.clone(),
                text,
                at: finished_at(run),
            })
        })
        .collect();
    needs.sort_by_key(|need| std::cmp::Reverse(need.at));
    needs
}

/// The rail's badge: a prompt waiting, else something that went wrong
/// since the user last looked (`seen_at`).
pub fn badge(needs: &[NeedsYou], seen_at: Option<i64>) -> Option<Attention> {
    if needs.iter().any(|need| need.kind == Attention::Approval) {
        return Some(Attention::Approval);
    }
    needs
        .iter()
        .find(|need| need.at > seen_at.unwrap_or(0))
        .map(|need| need.kind)
}

/// Finished runs of the last `RECENT_DAYS`, newest first, Pause all's
/// skips left out.
pub fn recent_results(recent: &[AutomationRunRow], now: i64) -> Vec<&AutomationRunRow> {
    let since = now - RECENT_DAYS * DAY_MS;
    let mut runs: Vec<&AutomationRunRow> = recent
        .iter()
        .filter(|run| !run.status.is_live() && !paused_skip(run))
        .filter(|run| finished_at(run) >= since)
        .collect();
    runs.sort_by_key(|run| std::cmp::Reverse(finished_at(run)));
    runs
}

/// One automation's runs on one day, as Recent results shows them: one
/// row, read from the latest.
#[derive(Clone, Debug)]
pub struct ResultGroup<'a> {
    /// Newest first.
    pub runs: Vec<&'a AutomationRunRow>,
}

impl<'a> ResultGroup<'a> {
    pub fn latest(&self) -> &'a AutomationRunRow {
        self.runs[0]
    }

    pub fn failed(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| run.status == RunStatus::Failed)
            .count()
    }

    /// The mark the row shows: failed when any run that day failed.
    pub fn status(&self) -> RunStatus {
        if self.failed() > 0 {
            RunStatus::Failed
        } else {
            self.latest().status
        }
    }

    /// "5 runs", "1 failed of 5"; nothing for one run.
    pub fn count_text(&self) -> Option<String> {
        let (all, failed) = (self.runs.len(), self.failed());
        match (all, failed) {
            (1, _) => None,
            (all, 0) => Some(format!("{all} runs")),
            (all, failed) => Some(format!("{failed} failed of {all}")),
        }
    }

    /// Any of its runs finished after `seen`.
    pub fn new_since(&self, seen: i64) -> bool {
        self.runs.iter().any(|run| finished_at(run) > seen)
    }
}

/// `runs` (newest first) as one row per automation per day, under the
/// days' labels, at most `RECENT_SHOWN` rows.
pub fn group_results<'a>(
    runs: &[&'a AutomationRunRow],
    now: i64,
    tz: &TimeZone,
) -> Vec<(String, Vec<ResultGroup<'a>>)> {
    let mut shown = 0;
    let mut days = Vec::new();
    for (label, runs) in by_day(runs, now, tz) {
        let mut groups: Vec<ResultGroup> = Vec::new();
        for run in runs {
            match groups
                .iter_mut()
                .find(|g| g.latest().automation_id == run.automation_id)
            {
                Some(group) => group.runs.push(run),
                None if shown < RECENT_SHOWN => {
                    shown += 1;
                    groups.push(ResultGroup { runs: vec![run] });
                }
                None => {}
            }
        }
        if !groups.is_empty() {
            days.push((label, groups));
        }
    }
    days
}

/// "Today", "Yesterday", then "Mon 26 Oct", in local time.
pub fn day_label(at: i64, now: i64, tz: &TimeZone) -> String {
    let day = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(tz.clone()))
    };
    let (Some(at), Some(today)) = (day(at), day(now)) else {
        return String::new();
    };
    if at.date() == today.date() {
        return "Today".to_string();
    }
    if today.date().yesterday().ok() == Some(at.date()) {
        return "Yesterday".to_string();
    }
    at.strftime("%a %-d %b").to_string()
}

/// `runs` (newest first) under their days' labels, in order.
pub fn by_day<'a>(
    runs: &[&'a AutomationRunRow],
    now: i64,
    tz: &TimeZone,
) -> Vec<(String, Vec<&'a AutomationRunRow>)> {
    let mut days: Vec<(String, Vec<&AutomationRunRow>)> = Vec::new();
    for run in runs {
        let label = day_label(finished_at(run), now, tz);
        match days.last_mut() {
            Some((last, group)) if *last == label => group.push(run),
            _ => days.push((label, vec![run])),
        }
    }
    days
}

/// The statuses of each automation's last `STRIP_RUNS` runs, oldest
/// first, by automation id.
pub fn run_strips(recent: &[AutomationRunRow]) -> HashMap<String, Vec<RunStatus>> {
    let mut by_automation: HashMap<&str, Vec<&AutomationRunRow>> = HashMap::new();
    for run in recent {
        by_automation
            .entry(run.automation_id.as_str())
            .or_default()
            .push(run);
    }
    by_automation
        .into_iter()
        .map(|(id, mut runs)| {
            runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
            runs.truncate(STRIP_RUNS);
            let strip = runs.iter().rev().map(|run| run.status).collect();
            (id.to_string(), strip)
        })
        .collect()
}

/// "Last 7 runs: 6 succeeded, 1 failed".
pub fn strip_summary(strip: &[RunStatus]) -> String {
    let count = |status: &[RunStatus]| strip.iter().filter(|s| status.contains(s)).count();
    let parts: Vec<String> = [
        (count(&[RunStatus::Succeeded]), "succeeded"),
        (count(&[RunStatus::Failed]), "failed"),
        (count(&[RunStatus::Skipped]), "skipped"),
        (count(&[RunStatus::Cancelled]), "cancelled"),
        (count(&[RunStatus::Pending, RunStatus::Running]), "running"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    match strip.len() {
        1 => format!("Last run: {}", parts.join(", ")),
        n => format!("Last {n} runs: {}", parts.join(", ")),
    }
}

/// One row of Upcoming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upcoming {
    pub automation_id: String,
    pub at: i64,
    /// The user skipped this run (Undo brings it back).
    pub skipped: bool,
    /// It falls inside Pause all.
    pub paused: bool,
    /// The automation's other runs in the window: an hourly one shows as
    /// one row.
    pub more: usize,
}

/// Runs due within `UPCOMING_MS` of `now` across enabled automations,
/// soonest first.
pub fn upcoming(
    items: &[AutomationRow],
    now: i64,
    paused_until: Option<i64>,
    tz: &TimeZone,
) -> Vec<Upcoming> {
    let horizon = now + UPCOMING_MS;
    let mut rows = Vec::new();
    for auto in items.iter().filter(|auto| auto.enabled) {
        let triggers = schedule::time_triggers(auto);
        if triggers.is_empty() {
            continue;
        }
        let mut times = Vec::new();
        let mut after = now;
        while times.len() < UPCOMING_PER_AUTOMATION {
            match schedule::next_automation_run_at(auto, after, tz) {
                Some(at) if at <= horizon => {
                    times.push(at);
                    after = at;
                }
                _ => break,
            }
        }
        let skip = active_skip(auto, now);
        let row = |at: i64, more: usize| Upcoming {
            automation_id: auto.id.clone(),
            at,
            skipped: skip == Some(at),
            paused: paused_until.is_some_and(|until| at < until),
            more,
        };
        let hourly = triggers
            .iter()
            .any(|t| t.kind == Some(ScheduleKind::Hourly));
        match times.split_first() {
            Some((first, rest)) if hourly => rows.push(row(*first, rest.len())),
            _ => rows.extend(times.into_iter().map(|at| row(at, 0))),
        }
    }
    rows.sort_by_key(|row| row.at);
    rows
}

/// What a card's second line says, most pressing first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardStatus {
    /// A run started at this time is going.
    Running(i64),
    /// The automation is switched off.
    Off,
    /// Pause all holds until then.
    PausedUntil(i64),
    /// The run at `skipped` is skipped; the one after runs.
    Skipping { skipped: i64, next: i64 },
    /// Its schedule, and its next run if it has one.
    Scheduled(Option<i64>),
}

pub fn card_status(
    auto: &AutomationRow,
    running_since: Option<i64>,
    paused_until: Option<i64>,
    now: i64,
) -> CardStatus {
    if let Some(since) = running_since {
        return CardStatus::Running(since);
    }
    if !auto.enabled {
        return CardStatus::Off;
    }
    if let Some(until) = paused_until {
        return CardStatus::PausedUntil(until);
    }
    if let Some(skipped) = active_skip(auto, now) {
        return CardStatus::Skipping {
            skipped,
            next: auto.next_run_at,
        };
    }
    let scheduled = !schedule::time_triggers(auto).is_empty();
    CardStatus::Scheduled((scheduled && auto.next_run_at > 0).then_some(auto.next_run_at))
}

impl BenCodeApp {
    /// The automation runs going now: the thread each runs in, and its link.
    pub(crate) fn running_automations(&self) -> Vec<(&str, &AutomationLink)> {
        let mut running: Vec<(&str, &AutomationLink)> = self
            .runs
            .iter()
            .filter_map(|(session, run)| Some((session.as_str(), run.automation.as_ref()?)))
            .collect();
        running.sort_by_key(|(_, link)| link.started_at);
        running
    }

    /// When automation `id`'s run started, while one is going.
    pub(crate) fn automation_running_since(&self, id: &str) -> Option<i64> {
        self.runs
            .values()
            .filter_map(|run| run.automation.as_ref())
            .filter(|link| link.automation_id == id)
            .map(|link| link.started_at)
            .min()
    }

    /// Needs you: prompts waiting first, then what went wrong.
    pub(crate) fn automations_needing_you(&self) -> Vec<NeedsYou> {
        let mut needs: Vec<NeedsYou> = self
            .runs
            .iter()
            .filter_map(|(session, run)| {
                let link = run.automation.as_ref()?;
                let request = run.pending_permission.as_ref()?;
                Some(NeedsYou {
                    automation_id: link.automation_id.clone(),
                    kind: Attention::Approval,
                    run_id: None,
                    session_id: Some(session.clone()),
                    text: format!("Waiting for approval: {}", request.description),
                    at: link.started_at,
                })
            })
            .collect();
        needs.sort_by_key(|need| need.at);
        needs.extend(self.automations.needs.iter().cloned());
        needs
    }

    /// The rail's Automations badge, if it shows.
    pub(crate) fn automation_badge(&self) -> Option<Attention> {
        let waiting = self
            .runs
            .values()
            .any(|run| run.automation.is_some() && run.pending_permission.is_some());
        if waiting {
            return Some(Attention::Approval);
        }
        badge(&self.automations.needs, self.settings.automations_seen_at)
    }

    /// The overview is on screen.
    pub(crate) fn automation_overview_shown(&self) -> bool {
        self.surface_open(Surface::Automations) && self.automations.page == AutomationPage::Overview
    }

    /// Shows the overview; the editor's edits are kept. What finished
    /// since the last look reads as new, and the look is recorded.
    pub(crate) fn show_automation_overview(&mut self, cx: &mut Context<Self>) {
        self.leave_automation_editor(AutomationPage::Overview, cx);
        let now = now_ms();
        // Never looked (the first load has not landed): nothing is new.
        self.automations.seen_before = Some(self.settings.automations_seen_at.unwrap_or(now));
        self.set_automations_seen_at(now, cx);
        cx.notify();
    }

    /// Loads the runs the overview, the cards and the badge read; a later
    /// load wins.
    pub(crate) fn load_recent_automation_runs(&mut self, cx: &mut Context<Self>) {
        self.automations.recent_requests += 1;
        let request = self.automations.recent_requests;
        let since = now_ms() - RECENT_DAYS * DAY_MS;
        self.db_then(
            cx,
            move |db| db.list_recent_automation_runs(RUNS_PER_AUTOMATION, since),
            move |this, runs, cx| {
                if request != this.automations.recent_requests {
                    return;
                }
                match runs {
                    Ok(runs) => this.show_recent_runs(runs, cx),
                    Err(err) => log::error!("list_recent_automation_runs failed: {err:#}"),
                }
            },
        );
    }

    pub(super) fn show_recent_runs(&mut self, runs: Vec<AutomationRunRow>, cx: &mut Context<Self>) {
        self.automations.recent = runs;
        self.automations.view_cache.invalidate();
        self.update_automation_needs();
        // The first load after the update: what came before is not new.
        if self.settings.automations_seen_at.is_none() {
            self.set_automations_seen_at(now_ms(), cx);
        }
        // Seen as it lands while the overview shows: no badge for it.
        if self.automation_overview_shown()
            && badge(&self.automations.needs, self.settings.automations_seen_at).is_some()
        {
            self.set_automations_seen_at(now_ms(), cx);
        }
        // The rail's badge and the surface both read them.
        cx.notify();
    }

    /// Needs you's finished entries follow the list and its runs.
    pub(super) fn update_automation_needs(&mut self) {
        self.automations.needs = finished_needs(&self.automations.items, &self.automations.recent);
    }

    /// Needs you › Dismiss: the run leaves at once, and is stored so.
    pub(crate) fn dismiss_automation_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        let now = now_ms();
        if let Some(run) = self.automations.recent.iter_mut().find(|r| r.id == run_id) {
            run.dismissed_at = Some(now);
        }
        self.update_automation_needs();
        let id = run_id.to_string();
        self.db_write("dismiss automation run", move |db| {
            db.dismiss_automation_run(&id, now)
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::RunTrigger;

    const NOW: i64 = 1_800_000_000_000;

    fn run(id: &str, auto: &str, created: i64, status: RunStatus) -> AutomationRunRow {
        AutomationRunRow {
            id: id.into(),
            automation_id: auto.into(),
            trigger: RunTrigger::Scheduled,
            scheduled_for: created,
            created_at: created,
            started_at: Some(created),
            completed_at: (!status.is_live()).then_some(created + 60_000),
            status,
            session_id: Some(format!("s-{id}")),
            error: None,
            dismissed_at: None,
        }
    }

    fn with_error(mut run: AutomationRunRow, error: &str) -> AutomationRunRow {
        run.error = Some(error.into());
        run
    }

    fn auto(id: &str) -> AutomationRow {
        AutomationRow {
            id: id.into(),
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn needs_you_reads_each_latest_finished_run() {
        let items = ["a", "b", "c", "d", "e", "f", "g"].map(auto);
        let recent = vec![
            // a: failed, then a run still going: the failure stands.
            run("a2", "a", NOW - 1_000, RunStatus::Running),
            with_error(run("a1", "a", NOW - 9_000, RunStatus::Failed), "boom"),
            // b: failed, then succeeded: cleared.
            run("b2", "b", NOW - 2_000, RunStatus::Succeeded),
            run("b1", "b", NOW - 8_000, RunStatus::Failed),
            // c: missed, then skipped under Pause all: still missed.
            with_error(run("c2", "c", NOW - 3_000, RunStatus::Skipped), ALL_PAUSED),
            with_error(run("c1", "c", NOW - 7_000, RunStatus::Skipped), MISSED),
            // d: not started before BenCode stopped, but dismissed.
            AutomationRunRow {
                dismissed_at: Some(NOW),
                ..with_error(run("d1", "d", NOW - 6_000, RunStatus::Skipped), NOT_STARTED)
            },
            // e: a skip under Pause all alone is no failure.
            with_error(run("e1", "e", NOW - 5_000, RunStatus::Skipped), ALL_PAUSED),
            // f: cut off when BenCode stopped; g: stopped by the user.
            with_error(
                run("f1", "f", NOW - 4_000, RunStatus::Cancelled),
                INTERRUPTED,
            ),
            run("g1", "g", NOW - 4_500, RunStatus::Cancelled),
        ];
        let needs = finished_needs(&items, &recent);
        let picked: Vec<(&str, Attention)> = needs
            .iter()
            .map(|n| (n.automation_id.as_str(), n.kind))
            .collect();
        assert_eq!(
            picked,
            [
                ("f", Attention::Interrupted),
                ("c", Attention::Missed),
                ("a", Attention::Failed)
            ],
            "newest first"
        );
        assert_eq!(needs[2].text, "boom");
        assert_eq!(needs[2].session_id.as_deref(), Some("s-a1"));
    }

    #[test]
    fn the_badge_shows_what_is_new_or_waiting() {
        let failed = NeedsYou {
            automation_id: "a".into(),
            kind: Attention::Failed,
            run_id: Some("r".into()),
            session_id: None,
            text: String::new(),
            at: 5_000,
        };
        assert_eq!(
            badge(std::slice::from_ref(&failed), None),
            Some(Attention::Failed)
        );
        assert_eq!(
            badge(std::slice::from_ref(&failed), Some(4_000)),
            Some(Attention::Failed)
        );
        assert_eq!(
            badge(std::slice::from_ref(&failed), Some(5_000)),
            None,
            "seen"
        );
        let waiting = NeedsYou {
            kind: Attention::Approval,
            at: 1,
            ..failed.clone()
        };
        assert_eq!(
            badge(&[failed, waiting], Some(9_000)),
            Some(Attention::Approval)
        );
    }

    #[test]
    fn recent_results_are_a_week_newest_first() {
        let recent = vec![
            run("live", "a", NOW - 1_000, RunStatus::Running),
            run("old", "a", NOW - 8 * DAY_MS, RunStatus::Succeeded),
            run("r1", "a", NOW - 3 * HOUR_MS, RunStatus::Succeeded),
            run("r2", "b", NOW - HOUR_MS, RunStatus::Failed),
            with_error(
                run("p", "b", NOW - 2 * HOUR_MS, RunStatus::Skipped),
                ALL_PAUSED,
            ),
        ];
        let ids: Vec<&str> = recent_results(&recent, NOW)
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, ["r2", "r1"]);
        let many: Vec<AutomationRunRow> = (0..40)
            .map(|n| {
                run(
                    &format!("x{n}"),
                    &format!("a{n}"),
                    NOW - 1_000 * n,
                    RunStatus::Succeeded,
                )
            })
            .collect();
        let runs = recent_results(&many, NOW);
        let rows: usize = group_results(&runs, NOW, &TimeZone::UTC)
            .iter()
            .map(|(_, groups)| groups.len())
            .sum();
        assert_eq!(rows, RECENT_SHOWN);
    }

    #[test]
    fn days_read_today_yesterday_then_a_date() {
        let tz = TimeZone::UTC;
        // 2026-10-30 12:00 UTC, a Friday.
        let now = "2026-10-30T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(day_label(now - HOUR_MS, now, &tz), "Today");
        assert_eq!(day_label(now - 13 * HOUR_MS, now, &tz), "Yesterday");
        assert_eq!(day_label(now - 4 * DAY_MS, now, &tz), "Mon 26 Oct");
        let runs = [
            run("t1", "a", now - HOUR_MS, RunStatus::Succeeded),
            run("t2", "a", now - 2 * HOUR_MS, RunStatus::Succeeded),
            run("y1", "a", now - 20 * HOUR_MS, RunStatus::Failed),
            run("m1", "a", now - 4 * DAY_MS, RunStatus::Succeeded),
        ];
        let refs: Vec<&AutomationRunRow> = runs.iter().collect();
        let days: Vec<(String, usize)> = by_day(&refs, now, &tz)
            .into_iter()
            .map(|(label, runs)| (label, runs.len()))
            .collect();
        assert_eq!(
            days,
            [
                ("Today".to_string(), 2),
                ("Yesterday".to_string(), 1),
                ("Mon 26 Oct".to_string(), 1)
            ]
        );
    }

    #[test]
    fn results_fold_to_one_row_per_automation_per_day() {
        let tz = TimeZone::UTC;
        // 2026-10-30 12:00 UTC.
        let now = "2026-10-30T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let mut recent: Vec<AutomationRunRow> = (1..6)
            .map(|h| {
                run(
                    &format!("h{h}"),
                    "hourly",
                    now - h * HOUR_MS,
                    RunStatus::Succeeded,
                )
            })
            .collect();
        recent[2].status = RunStatus::Failed;
        recent.push(run("d1", "daily", now - 90 * 60_000, RunStatus::Succeeded));
        recent.push(run(
            "y1",
            "hourly",
            now - 20 * HOUR_MS,
            RunStatus::Succeeded,
        ));
        let runs = recent_results(&recent, now);
        let days = group_results(&runs, now, &tz);
        // "day: latest run, status, count" per row.
        let read: Vec<String> = days
            .iter()
            .flat_map(|(day, groups)| {
                groups.iter().map(move |g| {
                    let count = g.count_text().unwrap_or_default();
                    format!("{day}: {} {:?} {count}", g.latest().id, g.status())
                })
            })
            .collect();
        assert_eq!(
            read,
            [
                "Today: h1 Failed 1 failed of 5",
                "Today: d1 Succeeded ",
                "Yesterday: y1 Succeeded ",
            ]
        );
        let hourly = &days[0].1[0];
        // New when any of its runs finished since the last look.
        assert!(hourly.new_since(now - 2 * HOUR_MS));
        assert!(!hourly.new_since(now));
    }

    #[test]
    fn the_strip_runs_oldest_to_newest() {
        let mut recent: Vec<AutomationRunRow> = (0..9)
            .map(|n| {
                run(
                    &format!("r{n}"),
                    "a",
                    NOW - (9 - n) * HOUR_MS,
                    RunStatus::Succeeded,
                )
            })
            .collect();
        recent[7].status = RunStatus::Failed;
        recent[8].status = RunStatus::Running;
        recent.push(run("other", "b", NOW, RunStatus::Failed));
        // Newest first, as loaded.
        recent.reverse();
        let strips = run_strips(&recent);
        let strip = &strips["a"];
        assert_eq!(strip.len(), STRIP_RUNS);
        assert_eq!(strip[0], RunStatus::Succeeded);
        assert_eq!(&strip[5..], [RunStatus::Failed, RunStatus::Running]);
        assert_eq!(
            strip_summary(strip),
            "Last 7 runs: 5 succeeded, 1 failed, 1 running"
        );
        assert_eq!(strip_summary(&[RunStatus::Skipped]), "Last run: 1 skipped");
        assert!(!strips.contains_key("none"));
    }

    fn scheduled(
        id: &str,
        kind: ScheduleKind,
        time: &str,
        now: i64,
        tz: &TimeZone,
    ) -> AutomationRow {
        let mut auto = auto(id);
        let mut trigger = schedule::new_time_trigger(format!("t-{id}"), kind);
        trigger.insert("time".into(), serde_json::Value::from(time));
        trigger.insert("minute".into(), serde_json::Value::from(15));
        schedule::apply_triggers(&mut auto, vec![trigger]);
        auto.next_run_at = schedule::next_automation_run_at(&auto, now, tz).unwrap();
        auto
    }

    #[test]
    fn upcoming_runs_skip_pause_and_fold_hourly() {
        let tz = TimeZone::UTC;
        // 2026-10-30 12:00 UTC, a Friday.
        let now = "2026-10-30T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let daily = scheduled("daily", ScheduleKind::Daily, "18:00", now, &tz);
        let hourly = scheduled("hourly", ScheduleKind::Hourly, "09:00", now, &tz);
        let weekdays = scheduled("weekdays", ScheduleKind::Weekdays, "08:00", now, &tz);
        // Its next run (Monday) is past the day; nothing shows.
        let mut off = scheduled("off", ScheduleKind::Daily, "13:00", now, &tz);
        off.enabled = false;
        // Skipped at 18:00 today, as Skip next run leaves it.
        let mut skipping = scheduled("skip", ScheduleKind::Daily, "18:00", now, &tz);
        let skipped = skipping.next_run_at;
        skipping.skipped_run_at = Some(skipped);
        skipping.next_run_at = schedule::next_automation_run_at(&skipping, skipped, &tz).unwrap();
        let items = [daily, hourly, weekdays, off, skipping];
        let rows = upcoming(&items, now, None, &tz);
        let read: Vec<(&str, bool, usize)> = rows
            .iter()
            .map(|r| (r.automation_id.as_str(), r.skipped, r.more))
            .collect();
        assert_eq!(
            read,
            [
                ("hourly", false, 23),
                ("daily", false, 0),
                ("skip", true, 0),
            ]
        );
        assert_eq!(rows[0].at, now + 15 * 60_000);
        // Pause all until 17:00: what falls before it reads paused.
        let paused = upcoming(&items, now, Some(now + 5 * HOUR_MS), &tz);
        assert!(paused[0].paused);
        assert!(!paused[1].paused, "the run at 18:00 goes ahead");
    }

    #[test]
    fn a_card_says_the_most_pressing_thing() {
        let mut a = auto("a");
        a.next_run_at = NOW + HOUR_MS;
        let trigger = schedule::new_time_trigger("t".into(), ScheduleKind::Daily);
        schedule::apply_triggers(&mut a, vec![trigger]);
        assert_eq!(
            card_status(&a, Some(NOW - 1), Some(NOW + 1), NOW),
            CardStatus::Running(NOW - 1)
        );
        assert_eq!(
            card_status(&a, None, Some(NOW + 9), NOW),
            CardStatus::PausedUntil(NOW + 9)
        );
        assert_eq!(
            card_status(&a, None, None, NOW),
            CardStatus::Scheduled(Some(NOW + HOUR_MS))
        );
        a.skipped_run_at = Some(NOW + 10);
        assert_eq!(
            card_status(&a, None, None, NOW),
            CardStatus::Skipping {
                skipped: NOW + 10,
                next: NOW + HOUR_MS
            }
        );
        a.enabled = false;
        assert_eq!(card_status(&a, None, Some(NOW + 9), NOW), CardStatus::Off);
        let mut none = auto("n");
        none.triggers = Some(Vec::new());
        assert_eq!(
            card_status(&none, None, None, NOW),
            CardStatus::Scheduled(None)
        );
    }
}
