//! How automation times read: MonoCode `formatAutomationRunAt`,
//! `formatAutomationRunDuration`, `gmtOffsetLabel` and `nextRunPreview`
//! (`features/automations/model/automations.ts`), and BenCode's short
//! forms for the cards and the overview ("next in 18h", "5h ago").

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

/// "Fri 30 Oct, 08:00": a skipped run, the end of a pause.
pub fn day_and_time(at_ms: i64, tz: &TimeZone) -> String {
    zoned(at_ms, tz).map_or_else(String::new, |at| {
        at.strftime("%a %-d %b, %H:%M").to_string()
    })
}

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

/// "Mon 2 Nov": the day a pause ends.
pub fn short_day(at_ms: i64, tz: &TimeZone) -> String {
    zoned(at_ms, tz).map_or_else(String::new, |at| at.strftime("%a %-d %b").to_string())
}

/// "08:00".
pub fn clock(at_ms: i64, tz: &TimeZone) -> String {
    zoned(at_ms, tz).map_or_else(String::new, |at| at.strftime("%H:%M").to_string())
}

/// How far ahead `at` is: "in 25m", "in 3h", "in 1h 30m" under three hours.
pub fn in_text(at: i64, now: i64) -> String {
    let minutes = ((at - now).max(0) + MINUTE_MS - 1) / MINUTE_MS;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "now".to_string(),
        (0, rest) => format!("in {rest}m"),
        (hours @ 1..=2, rest) if rest > 0 => format!("in {hours}h {rest}m"),
        (hours, _) => format!("in {hours}h"),
    }
}

/// A time ahead, by its day: "today 14:00", "tomorrow 08:00", "Fri 08:00"
/// within the week, else "15 Nov, 09:30".
pub fn when_text(at: i64, now: i64, tz: &TimeZone) -> String {
    let (Some(when), Some(today)) = (zoned(at, tz), zoned(now, tz)) else {
        return String::new();
    };
    let days = (when.date() - today.date()).get_days();
    match days {
        0 => format!("today {}", when.strftime("%H:%M")),
        1 => format!("tomorrow {}", when.strftime("%H:%M")),
        2..=6 => when.strftime("%a %H:%M").to_string(),
        _ => when.strftime("%-d %b, %H:%M").to_string(),
    }
}

/// A card's next run: "next in 18h" within a day, else "next Fri 08:00".
pub fn next_text(at: i64, now: i64, tz: &TimeZone) -> String {
    if at - now < DAY_MS {
        return format!("next {}", in_text(at, now));
    }
    let (Some(when), Some(today)) = (zoned(at, tz), zoned(now, tz)) else {
        return String::new();
    };
    if (when.date() - today.date()).get_days() < 7 {
        format!("next {}", when.strftime("%a %H:%M"))
    } else {
        format!("next {}", when.strftime("%-d %b, %H:%M"))
    }
}

/// How long ago, short: "now", "12m ago", "5h ago", "3d ago", "2w ago".
pub fn ago_text(then: i64, now: i64) -> String {
    let minutes = (now - then).max(0) / MINUTE_MS;
    match minutes {
        0 => "now".to_string(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        1440..10_080 => format!("{}d ago", minutes / 1440),
        _ => format!("{}w ago", minutes / 10_080),
    }
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
    elapsed(start, end)
}

/// The time from `start` to `end`: "< 1m", "12m", "1h 5m".
pub fn elapsed(start: i64, end: i64) -> String {
    let minutes = (end - start).max(0) / 60_000;
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
            dismissed_at: None,
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
        assert_eq!(day_and_time(at, &TimeZone::UTC), "Mon 12 Oct, 09:00");
        assert_eq!(run_at(0, &TimeZone::UTC), "—");
    }

    #[test]
    fn times_ahead_read_short() {
        const MINUTE: i64 = 60_000;
        const HOUR: i64 = 60 * MINUTE;
        let tz = TimeZone::UTC;
        // 2026-10-30 12:00 UTC, a Friday.
        let now = "2026-10-30T12:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(in_text(now + 25 * MINUTE, now), "in 25m");
        assert_eq!(in_text(now + 30_000, now), "in 1m");
        assert_eq!(in_text(now + 90 * MINUTE, now), "in 1h 30m");
        assert_eq!(in_text(now + 18 * HOUR + 5 * MINUTE, now), "in 18h");
        assert_eq!(next_text(now + 18 * HOUR, now, &tz), "next in 18h");
        assert_eq!(next_text(now + 68 * HOUR, now, &tz), "next Mon 08:00");
        assert_eq!(
            next_text(now + 9 * 24 * HOUR, now, &tz),
            "next 8 Nov, 12:00"
        );
        assert_eq!(when_text(now + 2 * HOUR, now, &tz), "today 14:00");
        assert_eq!(when_text(now + 20 * HOUR, now, &tz), "tomorrow 08:00");
        assert_eq!(when_text(now + 68 * HOUR, now, &tz), "Mon 08:00");
        assert_eq!(short_day(now + 3 * 24 * HOUR, &tz), "Mon 2 Nov");
        assert_eq!(ago_text(now - 30_000, now), "now");
        assert_eq!(ago_text(now - 12 * MINUTE, now), "12m ago");
        assert_eq!(ago_text(now - 5 * HOUR, now), "5h ago");
        assert_eq!(ago_text(now - 50 * HOUR, now), "2d ago");
        assert_eq!(ago_text(now - 15 * 24 * HOUR, now), "2w ago");
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
