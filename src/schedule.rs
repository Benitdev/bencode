//! Automation schedules, ported from MonoCode's `nextAutomationRunAt` and
//! `valid_time` so both apps agree on when a run is due.

use jiff::{Timestamp, ToSpan, Zoned, civil::Weekday, tz::TimeZone};
use serde_json::Value;

use crate::db::AutomationRow;

/// MonoCode `nextTriggersRunAt` with no time trigger: a year out.
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

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

/// When a time trigger fires: `scheduleKind`, `minute`, `time`, `dayOfWeek`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeTrigger {
    pub schedule_kind: String,
    pub minute: i64,
    pub time: String,
    pub day_of_week: i64,
}

fn time_trigger(fields: &serde_json::Map<String, Value>) -> Option<TimeTrigger> {
    if fields.get("kind").and_then(Value::as_str) != Some("time") {
        return None;
    }
    let text = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let number = |key: &str| fields.get(key).and_then(Value::as_i64).unwrap_or(0);
    Some(TimeTrigger {
        schedule_kind: text("scheduleKind"),
        minute: number("minute"),
        time: text("time"),
        day_of_week: number("dayOfWeek"),
    })
}

/// Time triggers of `auto`: its `triggers` list, else the legacy fields
/// (MonoCode `hydrate_triggers`).
pub fn time_triggers(auto: &AutomationRow) -> Vec<TimeTrigger> {
    match &auto.triggers {
        Some(triggers) => triggers.iter().filter_map(time_trigger).collect(),
        None => vec![TimeTrigger {
            schedule_kind: auto.schedule_kind.clone(),
            minute: auto.minute,
            time: auto.time.clone(),
            day_of_week: auto.day_of_week,
        }],
    }
}

/// The earliest next run across `auto`'s time triggers (MonoCode
/// `nextTriggersRunAt`); a year out when it has none. `None` only when a
/// time trigger is unreadable.
pub fn next_automation_run_at(auto: &AutomationRow, after_ms: i64, tz: &TimeZone) -> Option<i64> {
    let triggers = time_triggers(auto);
    if triggers.is_empty() {
        return Some(after_ms + YEAR_MS);
    }
    triggers
        .iter()
        .map(|t| {
            next_run_at(
                &t.schedule_kind,
                t.minute,
                &t.time,
                t.day_of_week,
                after_ms,
                tz,
            )
        })
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .min()
}

fn trigger_label(trigger: &TimeTrigger) -> String {
    let time = &trigger.time;
    match trigger.schedule_kind.as_str() {
        "hourly" => format!("Hourly at :{:02}", trigger.minute),
        "daily" => format!("Daily at {time}"),
        "weekdays" => format!("Weekdays at {time}"),
        _ => {
            let day = usize::try_from(trigger.day_of_week)
                .ok()
                .and_then(|d| WEEKDAYS.get(d));
            format!("{} at {time}", day.unwrap_or(&"Weekly"))
        }
    }
}

/// MonoCode `triggerLabel`: the first trigger, plus " +N" for the rest.
pub fn schedule_label(auto: &AutomationRow) -> String {
    let all = auto.triggers.as_ref().map_or(1, Vec::len);
    let Some(first) = time_triggers(auto).into_iter().next() else {
        return "On event".to_string();
    };
    match all {
        0 | 1 => trigger_label(&first),
        n => format!("{} +{}", trigger_label(&first), n - 1),
    }
}

/// `auto` with its first time trigger (and the legacy fields) set to run at
/// `time`; other triggers are kept as they are.
pub fn with_time(auto: &AutomationRow, time: &str) -> AutomationRow {
    let mut next = AutomationRow {
        time: time.to_string(),
        ..auto.clone()
    };
    if let Some(triggers) = next.triggers.as_mut()
        && let Some(first) = triggers.iter_mut().find(|t| time_trigger(t).is_some())
    {
        first.insert("time".into(), Value::from(time));
    }
    next
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

    fn trigger(kind: &str, schedule: &str, time: &str) -> serde_json::Map<String, Value> {
        let value = serde_json::json!({
            "id": "t", "kind": kind, "scheduleKind": schedule, "minute": 0,
            "time": time, "dayOfWeek": 1, "repos": [], "customKey": "kept"
        });
        value.as_object().unwrap().clone()
    }

    #[test]
    fn triggers_decide_the_next_run_and_label() {
        let auto = AutomationRow {
            schedule_kind: "daily".into(),
            time: "23:00".into(),
            triggers: Some(vec![
                trigger("time", "daily", "17:00"),
                trigger("time", "daily", "12:00"),
                trigger("github", "", ""),
            ]),
            ..Default::default()
        };
        let next = next_automation_run_at(&auto, ms("2026-10-02T10:00:00Z"), &TimeZone::UTC);
        assert_eq!(
            next,
            Some(ms("2026-10-02T12:00:00Z")),
            "earliest trigger wins"
        );
        assert_eq!(schedule_label(&auto), "Daily at 17:00 +2");

        let moved = with_time(&auto, "08:30");
        let first = &moved.triggers.as_ref().unwrap()[0];
        assert_eq!(first["time"], "08:30");
        assert_eq!(first["customKey"], "kept");
        assert_eq!(moved.triggers.as_ref().unwrap()[1]["time"], "12:00");
    }

    #[test]
    fn legacy_rows_and_event_only_rows() {
        let legacy = AutomationRow {
            schedule_kind: "weekdays".into(),
            time: "09:00".into(),
            ..Default::default()
        };
        assert_eq!(schedule_label(&legacy), "Weekdays at 09:00");
        let events = AutomationRow {
            triggers: Some(vec![trigger("github", "", "")]),
            ..Default::default()
        };
        let after = ms("2026-10-02T10:00:00Z");
        assert_eq!(
            next_automation_run_at(&events, after, &TimeZone::UTC),
            Some(after + YEAR_MS)
        );
        assert_eq!(schedule_label(&events), "On event");
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
