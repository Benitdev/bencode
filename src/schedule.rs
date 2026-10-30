//! Automation schedules, ported from MonoCode's `nextAutomationRunAt` and
//! `valid_time` so both apps agree on when a run is due. BenCode adds
//! several days to a weekly trigger (`daysOfWeek`) and monthly triggers
//! (`dayOfMonth`), which MonoCode does not know.

use jiff::{Timestamp, ToSpan, Zoned, civil::Weekday, tz::TimeZone};
use serde_json::Value;

use crate::db::AutomationRow;

/// MonoCode `nextTriggersRunAt` with no time trigger: a year out.
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;
/// Day names by `dayOfWeek`, Sunday = 0.
pub const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// MonoCode `AutomationScheduleKind`, by its stored name, and BenCode's
/// `monthly`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleKind {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
    Monthly,
}

impl ScheduleKind {
    pub const ALL: [Self; 5] = [
        Self::Hourly,
        Self::Daily,
        Self::Weekdays,
        Self::Weekly,
        Self::Monthly,
    ];

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekdays => "weekdays",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    /// The kinds MonoCode also schedules: a Mono's habits keep to them.
    pub fn is_monocode(self) -> bool {
        self != Self::Monthly
    }

    /// The name in the Add Trigger menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hourly => "Hourly",
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
        }
    }
}

/// MonoCode `timeOptions`: every half hour, and `current` when it is not one.
pub fn time_options(current: &str) -> Vec<String> {
    let mut options: Vec<String> = (0..24)
        .flat_map(|hour| [format!("{hour:02}:00"), format!("{hour:02}:30")])
        .collect();
    if !current.is_empty() && !options.iter().any(|option| option == current) {
        options.insert(0, current.to_string());
    }
    options
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

/// When a time trigger fires: `scheduleKind`, `minute`, `time`,
/// `daysOfWeek` (else `dayOfWeek`) and `dayOfMonth`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeTrigger {
    /// `None` for a kind this version does not know: it never comes due.
    pub kind: Option<ScheduleKind>,
    pub minute: i64,
    pub time: String,
    /// The days a weekly trigger runs, Sunday = 0 as JavaScript's `getDay`:
    /// sorted, never empty.
    pub days_of_week: Vec<i64>,
    /// The day a monthly trigger runs, 1–31; a month without it runs on
    /// its last day.
    pub day_of_month: i64,
}

impl TimeTrigger {
    /// One kind at `time` (or `minute` past the hour), on Monday and the 1st.
    pub fn new(kind: Option<ScheduleKind>, minute: i64, time: &str, day_of_week: i64) -> Self {
        Self {
            kind,
            minute,
            time: time.to_string(),
            days_of_week: vec![day_of_week.clamp(0, 6)],
            day_of_month: 1,
        }
    }

    /// When this trigger next fires strictly after `after_ms`, in epoch ms.
    pub fn next_run(&self, after_ms: i64, tz: &TimeZone) -> Option<i64> {
        let kind = self.kind?;
        let after = Timestamp::from_millisecond(after_ms)
            .ok()?
            .to_zoned(tz.clone());
        let start = after.with().second(0).subsec_nanosecond(0).build().ok()?;
        // The run of the day: today at `time`.
        let today = || {
            let (hour, min) = parse_time(&self.time)?;
            start.with().hour(hour).minute(min).build().ok()
        };
        let candidate = match kind {
            ScheduleKind::Hourly => {
                let at = start
                    .with()
                    .minute(self.minute.clamp(0, 59) as i8)
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
                // The earliest of the days.
                let days = self
                    .days_of_week
                    .iter()
                    .map(|day| {
                        let days = (day.clamp(&0, &6) - weekday).rem_euclid(7);
                        if days == 0 && at <= after { 7 } else { days }
                    })
                    .min()?;
                at.checked_add(days.days()).ok()?
            }
            ScheduleKind::Monthly => {
                let (hour, min) = parse_time(&self.time)?;
                let first = start.with().day(1).hour(hour).minute(min).build().ok()?;
                let day = self.day_of_month.clamp(1, 31) as i8;
                // The day, or the month's last when it is shorter.
                let on = |month: &Zoned| month.with().day(day.min(month.days_in_month())).build();
                let this_month = on(&first).ok()?;
                if this_month > after {
                    this_month
                } else {
                    on(&first.checked_add(1.month()).ok()?).ok()?
                }
            }
        };
        Some(candidate.timestamp().as_millisecond())
    }
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
        days_of_week: days_of_week(fields),
        day_of_month: fields
            .get("dayOfMonth")
            .and_then(Value::as_i64)
            .map_or(1, |day| day.clamp(1, 31)),
    })
}

/// A weekly trigger's days: `daysOfWeek`, else the one `dayOfWeek`.
pub fn days_of_week(fields: &Trigger) -> Vec<i64> {
    let mut days: Vec<i64> = fields
        .get("daysOfWeek")
        .and_then(Value::as_array)
        .map(|days| {
            days.iter()
                .filter_map(Value::as_i64)
                .filter(|day| (0..7).contains(day))
                .collect()
        })
        .unwrap_or_default();
    days.sort_unstable();
    days.dedup();
    if days.is_empty() {
        let day = fields.get("dayOfWeek").and_then(Value::as_i64).unwrap_or(0);
        days.push(day.clamp(0, 6));
    }
    days
}

/// `days` with `day` switched on or off; the last day left stays on.
pub fn toggle_weekday(days: &[i64], day: i64) -> Vec<i64> {
    let mut next: Vec<i64> = days.iter().copied().filter(|d| *d != day).collect();
    if next.len() == days.len() {
        next.push(day);
    } else if next.is_empty() {
        return days.to_vec();
    }
    next.sort_unstable();
    next
}

/// Stores a weekly trigger's days: `daysOfWeek`, and the first of them as
/// `dayOfWeek` for readers that know only one.
pub fn set_weekdays(trigger: &mut Trigger, days: &[i64]) {
    let Some(first) = days.first() else {
        return;
    };
    trigger.insert("dayOfWeek".into(), Value::from(*first));
    trigger.insert("daysOfWeek".into(), Value::from(days.to_vec()));
}

/// Time triggers of `auto`: its `triggers` list, else the legacy fields
/// (MonoCode `hydrate_triggers`).
pub fn time_triggers(auto: &AutomationRow) -> Vec<TimeTrigger> {
    match &auto.triggers {
        Some(triggers) => triggers.iter().filter_map(time_trigger).collect(),
        None => vec![TimeTrigger::new(
            ScheduleKind::parse(&auto.schedule_kind),
            auto.minute,
            &auto.time,
            auto.day_of_week,
        )],
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

/// When `auto` runs next after `after_ms`, passing over `skipped`, the
/// occurrence the user chose to skip, while it is still the next one.
/// Returns the run and the skip still in force (`None` once the schedule
/// no longer has that occurrence next).
pub fn next_run_skipping(
    auto: &AutomationRow,
    skipped: Option<i64>,
    after_ms: i64,
    tz: &TimeZone,
) -> Option<(i64, Option<i64>)> {
    let next = next_automation_run_at(auto, after_ms, tz)?;
    match skipped {
        Some(skipped) if skipped == next => {
            Some((next_automation_run_at(auto, next, tz)?, Some(skipped)))
        }
        _ => Some((next, None)),
    }
}

/// When `trigger` next fires as its automation will run it: after Pause
/// all, which skips what falls before `paused_until`, and past `skipped`,
/// the occurrence the user chose to skip.
pub fn trigger_next_run(
    trigger: &TimeTrigger,
    now: i64,
    skipped: Option<i64>,
    paused_until: Option<i64>,
    tz: &TimeZone,
) -> Option<i64> {
    let after = paused_until.map_or(now, |until| (until - 1).max(now));
    let next = trigger.next_run(after, tz)?;
    match skipped {
        Some(skipped) if skipped == next => trigger.next_run(next, tz),
        _ => Some(next),
    }
}

/// "1st", "2nd", "23rd", "11th".
pub fn ordinal(day: i64) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

/// "Mon", by `dayOfWeek`.
pub fn short_weekday(day: i64) -> &'static str {
    usize::try_from(day)
        .ok()
        .and_then(|d| WEEKDAYS.get(d))
        .map_or("?", |name| &name[..3])
}

pub(crate) fn trigger_label(trigger: &TimeTrigger) -> String {
    let time = &trigger.time;
    match trigger.kind {
        Some(ScheduleKind::Hourly) => format!("Hourly at :{:02}", trigger.minute),
        Some(ScheduleKind::Daily) => format!("Daily at {time}"),
        Some(ScheduleKind::Weekdays) => format!("Weekdays at {time}"),
        Some(ScheduleKind::Monthly) => {
            format!("Monthly on the {} at {time}", ordinal(trigger.day_of_month))
        }
        Some(ScheduleKind::Weekly) | None => match trigger.days_of_week.as_slice() {
            [day] => {
                let name = usize::try_from(*day).ok().and_then(|d| WEEKDAYS.get(d));
                format!("{} at {time}", name.unwrap_or(&"Weekly"))
            }
            days => {
                // Monday first, as the day toggles read.
                let mut days = days.to_vec();
                days.sort_by_key(|day| (day + 6) % 7);
                let names: Vec<&str> = days.into_iter().map(short_weekday).collect();
                format!("{} at {time}", names.join(", "))
            }
        },
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
/// on the hour; a monthly one on the 1st.
pub fn new_time_trigger(id: String, kind: ScheduleKind) -> Trigger {
    let trigger = serde_json::json!({
        "id": id, "kind": "time", "event": kind.id(), "scheduleKind": kind.id(),
        "minute": 0, "time": "09:00", "dayOfWeek": 1,
        "repos": [], "repo": "", "branch": "", "actor": "anyone"
    });
    match trigger {
        Value::Object(mut fields) => {
            if kind == ScheduleKind::Monthly {
                fields.insert("dayOfMonth".into(), Value::from(1));
            }
            fields
        }
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
        let kind = ScheduleKind::parse(kind);
        TimeTrigger::new(kind, minute, time, dow)
            .next_run(ms(after), &TimeZone::UTC)
            .unwrap()
    }

    fn weekly(days: &[i64], time: &str) -> TimeTrigger {
        TimeTrigger {
            days_of_week: days.to_vec(),
            ..TimeTrigger::new(Some(ScheduleKind::Weekly), 0, time, 0)
        }
    }

    fn monthly(day: i64, time: &str) -> TimeTrigger {
        TimeTrigger {
            day_of_month: day,
            ..TimeTrigger::new(Some(ScheduleKind::Monthly), 0, time, 0)
        }
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
        let daily = TimeTrigger::new(Some(ScheduleKind::Daily), 0, "9am", 0);
        assert_eq!(daily.next_run(0, &TimeZone::UTC), None);
        assert_eq!(ScheduleKind::parse("yearly"), None);
        // A trigger of an unknown kind never comes due.
        let yearly = AutomationRow {
            schedule_kind: "yearly".into(),
            time: "09:00".into(),
            ..Default::default()
        };
        assert_eq!(next_automation_run_at(&yearly, 0, &TimeZone::UTC), None);
        for kind in ScheduleKind::ALL {
            assert_eq!(ScheduleKind::parse(kind.id()), Some(kind));
        }
    }

    #[test]
    fn times_offer_every_half_hour_and_an_odd_current_one() {
        let options = time_options("09:00");
        assert_eq!(options.len(), 48);
        assert_eq!(
            (options[0].as_str(), options[47].as_str()),
            ("00:00", "23:30")
        );
        assert_eq!(time_options("09:15")[0], "09:15");
        assert_eq!(time_options("09:15").len(), 49);
    }

    #[test]
    fn weekly_on_several_days_takes_the_earliest() {
        // 2026-10-02 is a Friday; Mon, Wed, Fri.
        let days = weekly(&[1, 3, 5], "09:00");
        let next = |after: &str| days.next_run(ms(after), &TimeZone::UTC).unwrap();
        assert_eq!(next("2026-10-02T08:00:00Z"), ms("2026-10-02T09:00:00Z"));
        // Past Friday's run: over the weekend to Monday.
        assert_eq!(next("2026-10-02T09:00:00Z"), ms("2026-10-05T09:00:00Z"));
        assert_eq!(next("2026-10-05T10:00:00Z"), ms("2026-10-07T09:00:00Z"));
        // Saturday and Sunday across the week's end.
        let weekend = weekly(&[0, 6], "18:30");
        let after = ms("2026-10-03T19:00:00Z");
        assert_eq!(
            weekend.next_run(after, &TimeZone::UTC),
            Some(ms("2026-10-04T18:30:00Z"))
        );
        assert_eq!(trigger_label(&days), "Mon, Wed, Fri at 09:00");
        assert_eq!(trigger_label(&weekend), "Sat, Sun at 18:30");
    }

    #[test]
    fn weekly_days_read_the_list_else_the_one_day() {
        let mut fields = trigger("time", "weekly", "09:00");
        assert_eq!(days_of_week(&fields), vec![1]);
        set_weekdays(&mut fields, &[3, 5]);
        assert_eq!(fields["dayOfWeek"], 3);
        assert_eq!(days_of_week(&fields), vec![3, 5]);
        // Out of range and repeated days are dropped.
        fields.insert("daysOfWeek".into(), serde_json::json!([5, 5, 9, 1]));
        assert_eq!(days_of_week(&fields), vec![1, 5]);
        assert_eq!(toggle_weekday(&[1, 5], 3), vec![1, 3, 5]);
        assert_eq!(toggle_weekday(&[1, 5], 5), vec![1]);
        assert_eq!(toggle_weekday(&[1], 1), vec![1], "one day stays on");
    }

    #[test]
    fn monthly_runs_on_the_day_or_the_months_last() {
        let on_15th = monthly(15, "09:00");
        let next = |trigger: &TimeTrigger, after: &str| {
            trigger.next_run(ms(after), &TimeZone::UTC).unwrap()
        };
        assert_eq!(
            next(&on_15th, "2026-10-02T10:00:00Z"),
            ms("2026-10-15T09:00:00Z")
        );
        assert_eq!(
            next(&on_15th, "2026-10-15T09:00:00Z"),
            ms("2026-11-15T09:00:00Z")
        );
        let on_31st = monthly(31, "09:00");
        assert_eq!(
            next(&on_31st, "2026-04-01T00:00:00Z"),
            ms("2026-04-30T09:00:00Z")
        );
        assert_eq!(
            next(&on_31st, "2026-04-30T09:00:00Z"),
            ms("2026-05-31T09:00:00Z")
        );
        // February: the 28th, the 29th in a leap year.
        assert_eq!(
            next(&on_31st, "2026-02-01T00:00:00Z"),
            ms("2026-02-28T09:00:00Z")
        );
        assert_eq!(
            next(&on_31st, "2028-02-01T00:00:00Z"),
            ms("2028-02-29T09:00:00Z")
        );
        let on_30th = monthly(30, "09:00");
        assert_eq!(
            next(&on_30th, "2026-01-30T10:00:00Z"),
            ms("2026-02-28T09:00:00Z")
        );
        // December to January.
        assert_eq!(
            next(&on_15th, "2026-12-20T00:00:00Z"),
            ms("2027-01-15T09:00:00Z")
        );
        assert_eq!(trigger_label(&on_15th), "Monthly on the 15th at 09:00");
        assert_eq!(
            trigger_label(&monthly(1, "08:00")),
            "Monthly on the 1st at 08:00"
        );
        assert_eq!(
            [2, 3, 11, 12, 13, 21, 22, 23, 31].map(ordinal),
            [
                "2nd", "3rd", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st"
            ]
        );
    }

    #[test]
    fn monthly_triggers_store_their_day() {
        let fields = new_time_trigger("m".into(), ScheduleKind::Monthly);
        assert_eq!(fields["dayOfMonth"], 1);
        assert!(!new_time_trigger("d".into(), ScheduleKind::Daily).contains_key("dayOfMonth"));
        let mut fields = fields;
        fields.insert("dayOfMonth".into(), Value::from(40));
        assert_eq!(time_trigger(&fields).unwrap().day_of_month, 31);
    }

    #[test]
    fn local_time_holds_across_daylight_saving() {
        // New York: clocks go back on 1 Nov 2026 and forward on 14 Mar 2027.
        let tz = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let daily = TimeTrigger::new(Some(ScheduleKind::Daily), 0, "09:00", 0);
        // 09:00 EDT is 13:00Z; the day after the change 09:00 EST is 14:00Z.
        assert_eq!(
            daily.next_run(ms("2026-10-31T14:00:00Z"), &tz),
            Some(ms("2026-11-01T14:00:00Z"))
        );
        let days = weekly(&[1, 5], "09:00");
        assert_eq!(
            days.next_run(ms("2026-10-30T14:00:00Z"), &tz),
            Some(ms("2026-11-02T14:00:00Z"))
        );
        let month = monthly(15, "09:00");
        assert_eq!(
            month.next_run(ms("2027-02-20T00:00:00Z"), &tz),
            Some(ms("2027-03-15T13:00:00Z"))
        );
    }

    #[test]
    fn a_skip_holds_while_its_occurrence_is_next() {
        let auto = AutomationRow {
            triggers: Some(vec![trigger("time", "daily", "09:00")]),
            ..Default::default()
        };
        let (after, tz) = (ms("2026-10-02T10:00:00Z"), TimeZone::UTC);
        let tomorrow = ms("2026-10-03T09:00:00Z");
        assert_eq!(
            next_run_skipping(&auto, Some(tomorrow), after, &tz),
            Some((ms("2026-10-04T09:00:00Z"), Some(tomorrow)))
        );
        // Not the next occurrence any more: the skip lapses.
        let stale = ms("2026-10-02T09:00:00Z");
        assert_eq!(
            next_run_skipping(&auto, Some(stale), after, &tz),
            Some((tomorrow, None))
        );
        assert_eq!(
            next_run_skipping(&auto, None, after, &tz),
            Some((tomorrow, None))
        );
    }

    #[test]
    fn a_trigger_row_skips_what_its_automation_skips() {
        let daily = TimeTrigger::new(Some(ScheduleKind::Daily), 0, "09:00", 1);
        let (now, tz) = (ms("2026-10-02T10:00:00Z"), TimeZone::UTC);
        let tomorrow = ms("2026-10-03T09:00:00Z");
        let after = ms("2026-10-04T09:00:00Z");
        assert_eq!(
            trigger_next_run(&daily, now, None, None, &tz),
            Some(tomorrow)
        );
        assert_eq!(
            trigger_next_run(&daily, now, Some(tomorrow), None, &tz),
            Some(after)
        );
        // Paused until the 4th at 09:00: that run goes ahead, the one before not.
        let until = Some(after);
        assert_eq!(trigger_next_run(&daily, now, None, until, &tz), Some(after));
        // A pause already over changes nothing.
        let over = Some(now - 1);
        assert_eq!(
            trigger_next_run(&daily, now, None, over, &tz),
            Some(tomorrow)
        );
    }
}
