//! Automation schedules, ported from MonoCode's `nextAutomationRunAt` and
//! `valid_time` so both apps agree on when a run is due.

use jiff::{Timestamp, ToSpan, Zoned, civil::Weekday, tz::TimeZone};
use serde_json::Value;

use crate::db::AutomationRow;

/// MonoCode `nextTriggersRunAt` with no time trigger: a year out.
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;
pub const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// MonoCode `AutomationScheduleKind`, by its stored name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleKind {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
}

impl ScheduleKind {
    pub const ALL: [Self; 4] = [Self::Hourly, Self::Daily, Self::Weekdays, Self::Weekly];

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekdays => "weekdays",
            Self::Weekly => "weekly",
        }
    }

    /// The name in the Add Trigger menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hourly => "Hourly",
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
        }
    }
}

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

/// The next run strictly after `after_ms`, in epoch ms. `day_of_week`
/// counts from Sunday = 0, like JavaScript's `getDay`.
pub fn next_run_at(
    kind: ScheduleKind,
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
    // The run of the day: today at `time`.
    let today = || {
        let (hour, min) = parse_time(time)?;
        start.with().hour(hour).minute(min).build().ok()
    };
    let candidate = match kind {
        ScheduleKind::Hourly => {
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
        }
        ScheduleKind::Daily | ScheduleKind::Weekdays => {
            let mut at = today()?;
            if at <= after {
                at = at.checked_add(1.day()).ok()?;
            }
            while kind == ScheduleKind::Weekdays && is_weekend(&at) {
                at = at.checked_add(1.day()).ok()?;
            }
            at
        }
        ScheduleKind::Weekly => {
            let at = today()?;
            let weekday = i64::from(at.weekday().to_sunday_zero_offset());
            let mut days = (day_of_week.clamp(0, 6) - weekday).rem_euclid(7);
            if days == 0 && at <= after {
                days = 7;
            }
            at.checked_add(days.days()).ok()?
        }
    };
    Some(candidate.timestamp().as_millisecond())
}

/// When a time trigger fires: `scheduleKind`, `minute`, `time`, `dayOfWeek`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeTrigger {
    /// `None` for a kind this version does not know: it never comes due.
    pub kind: Option<ScheduleKind>,
    pub minute: i64,
    pub time: String,
    pub day_of_week: i64,
}

/// One entry of an automation's `triggers` (MonoCode `AutomationTrigger`),
/// kept as JSON so keys BenCode does not model survive.
pub type Trigger = serde_json::Map<String, Value>;

/// `fields` as a time trigger; `None` for an event trigger.
pub fn time_trigger(fields: &Trigger) -> Option<TimeTrigger> {
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
        kind: ScheduleKind::parse(&text("scheduleKind")),
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
            kind: ScheduleKind::parse(&auto.schedule_kind),
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
        .map(|t| t.next_run(after_ms, tz))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .min()
}

impl TimeTrigger {
    /// When this trigger next fires after `after_ms`.
    pub fn next_run(&self, after_ms: i64, tz: &TimeZone) -> Option<i64> {
        next_run_at(
            self.kind?,
            self.minute,
            &self.time,
            self.day_of_week,
            after_ms,
            tz,
        )
    }
}

fn trigger_label(trigger: &TimeTrigger) -> String {
    let time = &trigger.time;
    match trigger.kind {
        Some(ScheduleKind::Hourly) => format!("Hourly at :{:02}", trigger.minute),
        Some(ScheduleKind::Daily) => format!("Daily at {time}"),
        Some(ScheduleKind::Weekdays) => format!("Weekdays at {time}"),
        Some(ScheduleKind::Weekly) | None => {
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
    if all == 0 {
        return "No trigger".to_string();
    }
    let Some(first) = time_triggers(auto).into_iter().next() else {
        return "On event".to_string();
    };
    match all {
        1 => trigger_label(&first),
        n => format!("{} +{}", trigger_label(&first), n - 1),
    }
}

/// MonoCode `createAutomationTrigger` for a schedule kind: 09:00, Monday,
/// on the hour.
pub fn new_time_trigger(id: String, kind: ScheduleKind) -> Trigger {
    let trigger = serde_json::json!({
        "id": id, "kind": "time", "event": kind.id(), "scheduleKind": kind.id(),
        "minute": 0, "time": "09:00", "dayOfWeek": 1,
        "repos": [], "repo": "", "branch": "", "actor": "anyone"
    });
    match trigger {
        Value::Object(fields) => fields,
        _ => Trigger::new(),
    }
}

/// MonoCode `automationTriggers`: the `triggers` list, else one trigger
/// built from the legacy fields.
pub fn triggers_of(auto: &AutomationRow) -> Vec<Trigger> {
    if let Some(triggers) = &auto.triggers {
        return triggers.clone();
    }
    let kind = ScheduleKind::parse(&auto.schedule_kind).unwrap_or(ScheduleKind::Weekdays);
    let mut legacy = new_time_trigger(format!("{}:legacy", auto.id), kind);
    legacy.insert("minute".into(), Value::from(auto.minute));
    legacy.insert("time".into(), Value::from(auto.time.clone()));
    legacy.insert("dayOfWeek".into(), Value::from(auto.day_of_week));
    vec![legacy]
}

/// MonoCode `applyTriggers`: stores `triggers` and mirrors the primary one
/// (the first time trigger, else the first) into the legacy fields.
pub fn apply_triggers(auto: &mut AutomationRow, triggers: Vec<Trigger>) {
    let primary = triggers
        .iter()
        .find(|t| time_trigger(t).is_some())
        .or(triggers.first());
    let text = |key: &str, fallback: &str| {
        primary
            .and_then(|t| t.get(key))
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_string()
    };
    let number = |key: &str, fallback: i64| {
        primary
            .and_then(|t| t.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(fallback)
    };
    auto.trigger_kind = Some(text("kind", "time"));
    auto.trigger_event = Some(text("event", ""));
    auto.schedule_kind = text("scheduleKind", "weekdays");
    auto.minute = number("minute", 0);
    auto.time = text("time", "09:00");
    auto.day_of_week = number("dayOfWeek", 1);
    auto.triggers = Some(triggers);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    fn next(kind: &str, minute: i64, time: &str, dow: i64, after: &str) -> i64 {
        let kind = ScheduleKind::parse(kind).unwrap();
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

    fn trigger(kind: &str, schedule: &str, time: &str) -> Trigger {
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
    }

    #[test]
    fn applying_triggers_mirrors_the_first_time_trigger() {
        let mut auto = AutomationRow::default();
        let mut weekly = new_time_trigger("b".into(), ScheduleKind::Weekly);
        weekly.insert("time".into(), Value::from("16:00"));
        weekly.insert("dayOfWeek".into(), Value::from(5));
        apply_triggers(&mut auto, vec![trigger("github", "", ""), weekly]);
        assert_eq!(auto.trigger_kind.as_deref(), Some("time"));
        assert_eq!(auto.trigger_event.as_deref(), Some("weekly"));
        assert_eq!(
            (auto.schedule_kind.as_str(), auto.day_of_week),
            ("weekly", 5)
        );
        assert_eq!(schedule_label(&auto), "Friday at 16:00 +1");
        // Unknown keys of a trigger are kept.
        assert_eq!(auto.triggers.as_ref().unwrap()[0]["customKey"], "kept");

        apply_triggers(&mut auto, Vec::new());
        assert_eq!(
            (auto.schedule_kind.as_str(), auto.time.as_str()),
            ("weekdays", "09:00")
        );
        assert_eq!(schedule_label(&auto), "No trigger");
    }

    #[test]
    fn legacy_rows_hydrate_to_one_trigger() {
        let legacy = AutomationRow {
            id: "a".into(),
            schedule_kind: "hourly".into(),
            minute: 15,
            ..Default::default()
        };
        let triggers = triggers_of(&legacy);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["id"], "a:legacy");
        assert_eq!(time_trigger(&triggers[0]).unwrap().minute, 15);
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
        let daily = ScheduleKind::Daily;
        assert_eq!(next_run_at(daily, 0, "9am", 0, 0, &TimeZone::UTC), None);
        assert_eq!(ScheduleKind::parse("monthly"), None);
        // A trigger of an unknown kind never comes due.
        let monthly = AutomationRow {
            schedule_kind: "monthly".into(),
            time: "09:00".into(),
            ..Default::default()
        };
        assert_eq!(next_automation_run_at(&monthly, 0, &TimeZone::UTC), None);
        for kind in ScheduleKind::ALL {
            assert_eq!(ScheduleKind::parse(kind.id()), Some(kind));
        }
    }
}
