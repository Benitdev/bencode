//! Holding automations back without editing them: "Skip next run" passes
//! over one occurrence of one automation (a holiday) and can be undone;
//! "Pause all" skips every due run until a date (a vacation), kept in
//! `settings.json` as `automationsPausedUntil`. The scheduler records what
//! falls in a pause as skipped (`automation_runs.rs`), so nothing piles up
//! for the resume. BenCode's own.

use gpui::Context;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

use super::{BenCodeApp, now_ms};
use crate::db::AutomationRow;
use crate::ui::region::Region;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const MINUTE_MS: i64 = 60_000;

/// Pause all's lengths, as its menu offers them.
pub const PAUSE_LENGTHS: [(&str, i64); 3] = [("1 day", 1), ("3 days", 3), ("1 week", 7)];

/// Pause all › Until…: the date and time being picked, and why the last
/// one was refused.
#[derive(Clone, Debug)]
pub struct PausePick {
    pub value: DateTime,
    pub error: Option<&'static str>,
}

/// Local wall time of `ms`.
pub fn local_datetime(ms: i64) -> Option<DateTime> {
    let at = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(at.to_zoned(TimeZone::system()).datetime())
}

/// `value` read as local time, in ms.
fn local_ms(value: DateTime) -> Option<i64> {
    let zoned = value.to_zoned(TimeZone::system()).ok()?;
    Some(zoned.timestamp().as_millisecond())
}

/// The run `auto` skips, while it is still ahead of `now` and of nothing
/// else: the header shows it with its Undo.
pub fn active_skip(auto: &AutomationRow, now: i64) -> Option<i64> {
    auto.skipped_run_at
        .filter(|skipped| auto.enabled && *skipped > now && *skipped < auto.next_run_at)
}

/// The pause in force at `now`: its end, while it is still ahead.
pub fn pause_in_force(until: Option<i64>, now: i64) -> Option<i64> {
    until.filter(|until| *until > now)
}

impl BenCodeApp {
    /// When Pause all ends, while it holds.
    pub(crate) fn automations_paused_until(&self) -> Option<i64> {
        pause_in_force(self.settings.automations_paused_until, now_ms())
    }

    /// Pause all for `days` from now.
    pub(crate) fn pause_automations_for(&mut self, days: i64, cx: &mut Context<Self>) {
        self.pause_automations_until(Some(now_ms() + days * DAY_MS), cx);
    }

    /// Pause all until `until` (epoch ms); `None` resumes.
    pub(crate) fn pause_automations_until(&mut self, until: Option<i64>, cx: &mut Context<Self>) {
        self.automations.pause_pick = None;
        self.set_automations_paused_until(until, cx);
    }

    /// Pause all › Until…: starts at the pause in force, else a day from
    /// now, to the minute.
    pub(crate) fn open_pause_pick(&mut self, cx: &mut Context<Self>) {
        let start = self.automations_paused_until().unwrap_or(now_ms() + DAY_MS);
        let rounded = (start + MINUTE_MS - 1) / MINUTE_MS * MINUTE_MS;
        if let Some(value) = local_datetime(rounded) {
            self.automations.pause_pick = Some(PausePick { value, error: None });
        }
        self.redraw_regions(&[Region::Surface], cx);
    }

    pub(crate) fn set_pause_pick(&mut self, value: DateTime, cx: &mut Context<Self>) {
        if let Some(pick) = self.automations.pause_pick.as_mut() {
            pick.value = value;
            pick.error = None;
        }
        self.redraw_regions(&[Region::Surface], cx);
    }

    pub(crate) fn close_pause_pick(&mut self, cx: &mut Context<Self>) {
        self.automations.pause_pick = None;
        self.redraw_regions(&[Region::Surface], cx);
    }

    /// Until…'s Pause: the time picked, if it is ahead.
    pub(crate) fn submit_pause_pick(&mut self, cx: &mut Context<Self>) {
        let Some(pick) = self.automations.pause_pick.as_mut() else {
            return;
        };
        match local_ms(pick.value) {
            Some(until) if until > now_ms() => self.pause_automations_until(Some(until), cx),
            Some(_) => pick.error = Some("Choose a time in the future."),
            None => pick.error = Some("Choose a valid date and time."),
        }
        self.redraw_regions(&[Region::Surface], cx);
    }

    /// Skip next run of automation `id`, or `undo` the skip.
    pub(crate) fn skip_automation_run(&mut self, id: &str, undo: bool, cx: &mut Context<Self>) {
        let (request, id) = (self.next_automation_list(), id.to_string());
        self.db_then(
            cx,
            move |db| {
                let now = now_ms();
                let done = if undo {
                    db.unskip_automation_run(&id, now)
                } else {
                    db.skip_next_automation_run(&id, now, &TimeZone::system())
                };
                db.list_automations().map(|items| (items, done))
            },
            move |this, listed, cx| match listed {
                Ok((items, done)) => {
                    this.show_automations(request, items, cx);
                    if let Err(err) = done {
                        this.automation_failed("skip the next run", &err, cx);
                    }
                }
                Err(err) => this.automation_failed("skip the next run", &err, cx),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pause_holds_only_until_its_end() {
        assert_eq!(pause_in_force(Some(2_000), 1_000), Some(2_000));
        assert_eq!(pause_in_force(Some(2_000), 2_000), None);
        assert_eq!(pause_in_force(None, 1_000), None);
    }

    #[test]
    fn a_skip_shows_while_it_is_ahead() {
        let auto = AutomationRow {
            enabled: true,
            next_run_at: 3_000,
            skipped_run_at: Some(2_000),
            ..Default::default()
        };
        assert_eq!(active_skip(&auto, 1_000), Some(2_000));
        assert_eq!(active_skip(&auto, 2_000), None, "passed");
        let paused = AutomationRow {
            enabled: false,
            ..auto.clone()
        };
        assert_eq!(active_skip(&paused, 1_000), None);
    }
}
