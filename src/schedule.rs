//! Automation schedules, ported from MonoCode's `nextAutomationRunAt` and
//! `valid_time` so both apps agree on when a run is due.

use jiff::{Timestamp, ToSpan, Zoned, civil::Weekday, tz::TimeZone};

/// `HH:MM`, 24-hour, exactly as MonoCode's backend accepts it.
pub fn parse_time(value: &str) -> Option<(i8, i8)> {
    let (hour, minute) = value.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    let (hour, minute) = (hour.parse::<i8>().ok()?, minute.parse::<i8>().ok()?);
    ((0..24).contains(&hour) && (0..60).contains(&minute)).then_some((hour, minute))
}

fn is_weekend(day: &Zoned) -> bool {
    matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// The next run strictly after `after_ms`, in epoch ms, for MonoCode's
/// `scheduleKind` (`hourly`, `daily`, `weekdays`, `weekly`). `day_of_week`
/// counts from Sunday = 0, like JavaScript's `getDay`.
pub fn next_run_at(
    kind: &str,
    minute: i64,
    time: &str,
    day_of_week: i64,
    after_ms: i64,
    tz: &TimeZone,
) -> Option<i64> {
    let after = Timestamp::from_millisecond(after_ms)
        .ok()?
        .to_zoned(tz.clone());
    let start = after.with().second(0).subsec_nanosecond(0).build().ok()?;
    let candidate = if kind == "hourly" {
        let at = start
            .with()
            .minute(minute.clamp(0, 59) as i8)
            .build()
            .ok()?;
        if at <= after {
            at.checked_add(1.hour()).ok()?
        } else {
            at
        }
    } else {
        let (hour, min) = parse_time(time)?;
        let mut at = start.with().hour(hour).minute(min).build().ok()?;
        match kind {
            "daily" | "weekdays" => {
                if at <= after {
                    at = at.checked_add(1.day()).ok()?;
                }
                while kind == "weekdays" && is_weekend(&at) {
                    at = at.checked_add(1.day()).ok()?;
                }
                at
            }
            "weekly" => {
                let today = i64::from(at.weekday().to_sunday_zero_offset());
                let mut days = (day_of_week.clamp(0, 6) - today).rem_euclid(7);
                if days == 0 && at <= after {
                    days = 7;
                }
                at.checked_add(days.days()).ok()?
            }
            _ => return None,
        }
    };
    Some(candidate.timestamp().as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    fn next(kind: &str, minute: i64, time: &str, dow: i64, after: &str) -> i64 {
        next_run_at(kind, minute, time, dow, ms(after), &TimeZone::UTC).unwrap()
    }

    #[test]
    fn parse_time_matches_monocode_rules() {
        assert_eq!(parse_time("09:05"), Some((9, 5)));
        assert_eq!(parse_time("23:59"), Some((23, 59)));
        for bad in ["9:05", "24:00", "12:60", "9am", "", "12:5"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn hourly_daily_weekdays_and_weekly() {
        // 2026-10-02 is a Friday.
        assert_eq!(
            next("hourly", 30, "", 0, "2026-10-02T10:40:00Z"),
            ms("2026-10-02T11:30:00Z")
        );
        assert_eq!(
            next("daily", 0, "09:00", 0, "2026-10-02T10:00:00Z"),
            ms("2026-10-03T09:00:00Z")
        );
        assert_eq!(
            next("daily", 0, "17:00", 0, "2026-10-02T10:00:00Z"),
            ms("2026-10-02T17:00:00Z")
        );
        assert_eq!(
            next("weekdays", 0, "09:00", 0, "2026-10-02T10:00:00Z"),
            ms("2026-10-05T09:00:00Z")
        );
        assert_eq!(
            next("weekly", 0, "09:00", 1, "2026-10-02T10:00:00Z"),
            ms("2026-10-05T09:00:00Z")
        );
        assert_eq!(
            next("weekly", 0, "09:00", 5, "2026-10-02T10:00:00Z"),
            ms("2026-10-09T09:00:00Z")
        );
    }

    #[test]
    fn invalid_input_yields_none() {
        assert_eq!(next_run_at("daily", 0, "9am", 0, 0, &TimeZone::UTC), None);
        assert_eq!(
            next_run_at("monthly", 0, "09:00", 0, 0, &TimeZone::UTC),
            None
        );
    }
}
