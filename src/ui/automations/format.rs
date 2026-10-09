//! How automation times read: MonoCode `formatAutomationRunAt`,
//! `formatAutomationRunDuration`, `gmtOffsetLabel` and `nextRunPreview`
//! (`features/automations/model/automations.ts`).

use jiff::{Timestamp, Zoned, tz::TimeZone};

use crate::db::{AutomationRunRow, RunStatus};

fn zoned(at_ms: i64, tz: &TimeZone) -> Option<Zoned> {
    Some(
        Timestamp::from_millisecond(at_ms)
            .ok()?
            .to_zoned(tz.clone()),
    )
}

/// "GMT+7", "GMT-3:30".
pub fn gmt_offset_label(offset_seconds: i32) -> String {
    let minutes = offset_seconds / 60;
    let sign = if minutes >= 0 { '+' } else { '-' };
    let (hours, rest) = (minutes.abs() / 60, minutes.abs() % 60);
    if rest == 0 {
        format!("GMT{sign}{hours}")
    } else {
        format!("GMT{sign}{hours}:{rest:02}")
    }
}

/// The offset in force at `at_ms`, as `gmt_offset_label`.
pub fn gmt_offset_at(at_ms: i64, tz: &TimeZone) -> String {
    zoned(at_ms, tz).map_or_else(String::new, |at| gmt_offset_label(at.offset().seconds()))
}

/// "Next run Mon 12 Oct, 09:00 GMT+7".
pub fn next_run_preview(at_ms: i64, tz: &TimeZone) -> String {
    zoned(at_ms, tz).map_or_else(String::new, |at| {
        format!(
            "Next run {} {}",
            at.strftime("%a %-d %b, %H:%M"),
            gmt_offset_label(at.offset().seconds())
        )
    })
}

/// "8 Oct, 09:00"; a dash for a missing time.
pub fn run_at(at_ms: i64, tz: &TimeZone) -> String {
    match zoned(at_ms, tz) {
        Some(at) if at_ms > 0 => at.strftime("%-d %b, %H:%M").to_string(),
        _ => "—".to_string(),
    }
}

/// How long `run` took, or has taken so far: "< 1m", "12m", "1h 5m".
pub fn run_duration(run: &AutomationRunRow, now: i64) -> String {
    let start = run
        .started_at
        .or((run.status != RunStatus::Pending).then_some(run.created_at));
    let end = run
        .completed_at
        .or(start.filter(|_| run.status.is_live()).map(|_| now));
    let (Some(start), Some(end)) = (start, end) else {
        return "—".to_string();
    };
    if start <= 0 || end < start {
        return "—".to_string();
    }
    let minutes = (end - start) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "< 1m".to_string(),
        (0, rest) => format!("{rest}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, rest) => format!("{hours}h {rest}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: RunStatus, started: Option<i64>, completed: Option<i64>) -> AutomationRunRow {
        AutomationRunRow {
            id: "r".into(),
            automation_id: "a".into(),
            trigger: crate::db::RunTrigger::Manual,
            scheduled_for: 1_000,
            created_at: 1_000,
            started_at: started,
            completed_at: completed,
            status,
            session_id: None,
            error: None,
        }
    }

    #[test]
    fn offsets_read_like_monocode() {
        assert_eq!(gmt_offset_label(7 * 3600), "GMT+7");
        assert_eq!(gmt_offset_label(0), "GMT+0");
        assert_eq!(gmt_offset_label(-(3 * 3600 + 1800)), "GMT-3:30");
    }

    #[test]
    fn times_are_short_and_local() {
        // 2026-10-12 is a Monday.
        let at = "2026-10-12T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(
            next_run_preview(at, &TimeZone::UTC),
            "Next run Mon 12 Oct, 09:00 GMT+0"
        );
        assert_eq!(run_at(at, &TimeZone::UTC), "12 Oct, 09:00");
        assert_eq!(run_at(0, &TimeZone::UTC), "—");
    }

    #[test]
    fn durations_follow_the_run_state() {
        const MINUTE: i64 = 60_000;
        assert_eq!(
            run_duration(&run(RunStatus::Pending, None, None), 9 * MINUTE),
            "—"
        );
        assert_eq!(
            run_duration(&run(RunStatus::Running, Some(MINUTE), None), MINUTE + 5_000),
            "< 1m"
        );
        assert_eq!(
            run_duration(&run(RunStatus::Running, Some(MINUTE), None), 13 * MINUTE),
            "12m"
        );
        assert_eq!(
            run_duration(
                &run(RunStatus::Succeeded, Some(MINUTE), Some(66 * MINUTE)),
                0
            ),
            "1h 5m"
        );
        assert_eq!(
            run_duration(
                &run(RunStatus::Succeeded, Some(MINUTE), Some(121 * MINUTE)),
                0
            ),
            "2h"
        );
        // A skipped run never started; it is timed from its creation.
        assert_eq!(
            run_duration(&run(RunStatus::Skipped, None, Some(1_000)), 0),
            "< 1m"
        );
    }
}
