//! MonoCode `features/monos/model/monoHabits.ts`: what a Mono does on its
//! own, on a schedule, kept in its folder's `habits.json`. Each run happens
//! in a hidden thread of its own (`app/habits.rs`), so the work never piles
//! up in the Mono's conversation; only what it decides to tell the user is
//! posted to the chat, and a run with nothing worth saying posts nothing.
//!
//! The file is MonoCode's: fields BenCode does not model round-trip, and
//! every change is made over the version read (`update_habits`).

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::files::{self, HABITS_FILE, SaveError};
use crate::schedule::ScheduleKind;

/// MonoCode `HABIT_RUNS_KEPT`.
pub const HABIT_RUNS_KEPT: usize = 20;
const REPORT_KEPT_CHARS: usize = 4_000;
/// MonoCode `QUIET_MARKER`: a run's whole reply when it has nothing worth
/// telling the user.
pub const QUIET_MARKER: &str = "NOTHING_TO_REPORT";
/// A run that was due longer ago than this is skipped, not caught up.
pub const MISSED_RUN_GRACE_MS: i64 = 2 * 60 * 60 * 1000;
/// One hour of work per run; time waiting on the user is not counted.
pub const HABIT_RUN_TIMEOUT_MS: i64 = 60 * 60 * 1000;
/// An approval the user has not answered in this long is turned down.
pub const HABIT_APPROVAL_WAIT_MS: i64 = 12 * 60 * 60 * 1000;
pub const HABITS_MAX: usize = 20;

/// MonoCode `HabitSchedule`: times are local.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitSchedule {
    pub schedule_kind: String,
    /// Minute past the hour, for hourly habits.
    pub minute: u8,
    /// "HH:MM".
    pub time: String,
    /// 0 is Sunday, for weekly habits.
    pub day_of_week: u8,
}

impl HabitSchedule {
    /// MonoCode `habitScheduleLabel` (`automationScheduleLabel`): "Weekdays
    /// at 09:00", "Friday at 16:30".
    pub fn label(&self) -> String {
        crate::schedule::trigger_label(&crate::schedule::TimeTrigger {
            kind: ScheduleKind::parse(&self.schedule_kind),
            minute: i64::from(self.minute),
            time: self.time.clone(),
            day_of_week: i64::from(self.day_of_week),
        })
    }

    /// MonoCode `nextHabitRunAt`: the next time strictly after `after`.
    pub fn next_run_at(&self, after: i64) -> i64 {
        let tz = jiff::tz::TimeZone::system();
        ScheduleKind::parse(&self.schedule_kind)
            .and_then(|kind| {
                crate::schedule::next_run_at(
                    kind,
                    i64::from(self.minute),
                    &self.time,
                    i64::from(self.day_of_week),
                    after,
                    &tz,
                )
            })
            // A kind this version cannot schedule waits a year.
            .unwrap_or(after + 365 * 24 * 60 * 60 * 1000)
    }
}

/// MonoCode `HabitOutcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HabitOutcome {
    /// It told the user something.
    Posted,
    /// It found nothing worth saying.
    Quiet,
    Failed,
}

/// MonoCode `HabitRun`: one run as the habit remembers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitRun {
    pub at: i64,
    #[serde(default)]
    pub duration_ms: i64,
    pub outcome: HabitOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// MonoCode `Habit`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Habit {
    pub id: String,
    pub name: String,
    /// What to do and when it is worth telling the user.
    pub instructions: String,
    pub schedule: HabitSchedule,
    pub enabled: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub next_run_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<HabitOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Recent runs, newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<HabitRun>>,
    /// Run once as soon as possible, paused or not, leaving the schedule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub run_requested: bool,
    /// The suggestion card in the chat it was started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card: Option<String>,
    /// What MonoCode writes that BenCode does not model.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn number(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)?
        .as_f64()
        .filter(|n| n.is_finite())
        .map(|n| n as i64)
}

const KNOWN: [&str; 12] = [
    "id",
    "name",
    "instructions",
    "schedule",
    "enabled",
    "createdAt",
    "nextRunAt",
    "lastRunAt",
    "lastOutcome",
    "lastError",
    "runs",
    "runRequested",
];

fn parse_runs(value: &Value) -> Vec<HabitRun> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|run| {
                    let outcome = serde_json::from_value(run.get("outcome")?.clone()).ok()?;
                    Some(HabitRun {
                        at: number(run, "at")?,
                        duration_ms: number(run, "durationMs").unwrap_or(0),
                        outcome,
                        report: string(run, "report"),
                        error: string(run, "error"),
                    })
                })
                .take(HABIT_RUNS_KEPT)
                .collect()
        })
        .unwrap_or_default()
}

/// MonoCode `parseHabits`: `habits.json` as the app wrote it; anything
/// malformed is left out.
pub fn parse_habits(text: &str) -> Vec<Habit> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|raw| {
            let schedule = raw.get("schedule")?;
            let kind = string(schedule, "scheduleKind")?;
            ScheduleKind::parse(&kind)?;
            let extra: Map<String, Value> = raw
                .as_object()?
                .iter()
                .filter(|(key, _)| !KNOWN.contains(&key.as_str()) && key.as_str() != "fromCard")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            Some(Habit {
                id: string(raw, "id")?,
                name: string(raw, "name")?,
                instructions: string(raw, "instructions")?,
                schedule: HabitSchedule {
                    schedule_kind: kind,
                    minute: number(schedule, "minute").unwrap_or(0).clamp(0, 59) as u8,
                    time: string(schedule, "time").unwrap_or_else(|| "09:00".to_string()),
                    day_of_week: number(schedule, "dayOfWeek").unwrap_or(1).clamp(0, 6) as u8,
                },
                enabled: raw.get("enabled").and_then(Value::as_bool) != Some(false),
                created_at: number(raw, "createdAt").unwrap_or(0),
                next_run_at: number(raw, "nextRunAt").unwrap_or(0),
                last_run_at: number(raw, "lastRunAt"),
                last_outcome: raw
                    .get("lastOutcome")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
                last_error: string(raw, "lastError"),
                runs: raw.get("runs").filter(|v| v.is_array()).map(parse_runs),
                run_requested: raw.get("runRequested").and_then(Value::as_bool) == Some(true),
                from_card: string(raw, "fromCard"),
                extra,
            })
        })
        .collect()
}

/// MonoCode `serializeHabits`.
pub fn serialize_habits(habits: &[Habit]) -> String {
    match serde_json::to_string_pretty(habits) {
        Ok(text) => format!("{text}\n"),
        Err(err) => {
            log::error!("habits did not serialize: {err}");
            "[]\n".to_string()
        }
    }
}

/// MonoCode `habitSchedule`: a schedule from the CLI (`{"kind":"weekdays",
/// "time":"09:00"}`); times are local.
pub fn habit_schedule(value: Option<&Value>) -> Result<HabitSchedule, String> {
    let input = value.and_then(Value::as_object).ok_or_else(|| {
        "schedule must be an object like {\"kind\":\"weekdays\",\"time\":\"09:00\"}".to_string()
    })?;
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ScheduleKind::parse)
        .ok_or_else(|| "schedule.kind must be hourly, daily, weekdays or weekly".to_string())?;
    let time = match input.get("time") {
        None => "09:00".to_string(),
        Some(value) => value
            .as_str()
            .filter(|time| crate::schedule::parse_time(time).is_some())
            .ok_or_else(|| "schedule.time must be 24-hour \"HH:MM\"".to_string())?
            .to_string(),
    };
    let int = |key: &str, max: u64, error: &str| -> Result<u8, String> {
        match input.get(key) {
            None => Ok(if key == "dayOfWeek" { 1 } else { 0 }),
            Some(value) => value
                .as_u64()
                .filter(|n| *n <= max)
                .map(|n| n as u8)
                .ok_or_else(|| error.to_string()),
        }
    };
    let minute = int("minute", 59, "schedule.minute must be 0-59")?;
    let day_of_week = int("dayOfWeek", 6, "schedule.dayOfWeek must be 0 (Sunday) to 6")?;
    let unknown: Vec<&str> = input
        .keys()
        .map(String::as_str)
        .filter(|key| !["kind", "time", "minute", "dayOfWeek"].contains(key))
        .collect();
    if !unknown.is_empty() {
        return Err(format!("Unknown schedule fields: {}", unknown.join(", ")));
    }
    Ok(HabitSchedule {
        schedule_kind: kind.id().to_string(),
        minute,
        time,
        day_of_week,
    })
}

/// MonoCode `newHabit`: enabled, first due at its next time after `now`.
pub fn new_habit(name: &str, instructions: &str, schedule: HabitSchedule, now: i64) -> Habit {
    Habit {
        id: crate::app::ids::unique_id("habit"),
        name: name.to_string(),
        instructions: instructions.to_string(),
        next_run_at: schedule.next_run_at(now),
        schedule,
        enabled: true,
        created_at: now,
        last_run_at: None,
        last_outcome: None,
        last_error: None,
        runs: None,
        run_requested: false,
        from_card: None,
        extra: Map::new(),
    }
}

/// MonoCode `dueHabits`: habits to run now, and ones that came due too
/// long ago (the app closed, the computer asleep) and only move on.
pub fn due_habits<'a>(
    habits: &'a [Habit],
    now: i64,
    running: &HashSet<String>,
) -> (Vec<&'a Habit>, Vec<&'a Habit>) {
    let (mut run, mut skipped) = (Vec::new(), Vec::new());
    for habit in habits {
        // One run of a habit at a time; it comes due again once it ends.
        if running.contains(&habit.id) {
            continue;
        }
        if habit.run_requested {
            run.push(habit);
            continue;
        }
        if !habit.enabled || habit.next_run_at > now {
            continue;
        }
        if now - habit.next_run_at <= MISSED_RUN_GRACE_MS {
            run.push(habit);
        } else {
            skipped.push(habit);
        }
    }
    (run, skipped)
}

/// How a run went, for `after_run`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub outcome: HabitOutcome,
    pub error: Option<String>,
    pub report: Option<String>,
    pub started_at: i64,
}

/// MonoCode `afterRun`: the habit after a run, or a skip (`None`), at `now`.
pub fn after_run(habit: &Habit, now: i64, result: Option<&RunResult>) -> Habit {
    let mut next = habit.clone();
    next.run_requested = false;
    // A requested run leaves the schedule where it was.
    if !(habit.run_requested && habit.next_run_at > now) {
        next.next_run_at = habit.schedule.next_run_at(now);
    }
    let Some(result) = result else {
        return next;
    };
    let run = HabitRun {
        at: result.started_at,
        duration_ms: (now - result.started_at).max(0),
        outcome: result.outcome,
        report: result
            .report
            .as_ref()
            .map(|report| report.chars().take(REPORT_KEPT_CHARS).collect()),
        error: result.error.clone(),
    };
    next.last_run_at = Some(now);
    next.last_outcome = Some(result.outcome);
    next.last_error = result.error.clone();
    let mut runs = vec![run];
    runs.extend(habit.runs.clone().unwrap_or_default());
    runs.truncate(HABIT_RUNS_KEPT);
    next.runs = Some(runs);
    next
}

/// MonoCode `updateHabits`: `habits.json` changed over the version read,
/// again from the top when the panel or a run saved in between.
pub fn update_habits<T>(
    dir: &Path,
    change: impl Fn(Vec<Habit>) -> Result<(Vec<Habit>, T), String>,
) -> Result<T, String> {
    for attempt in 0.. {
        let (text, hash) = files::read(dir, HABITS_FILE).map_err(|err| format!("{err:#}"))?;
        let (habits, result) = change(parse_habits(text.as_deref().unwrap_or("")))?;
        match files::save(dir, HABITS_FILE, &serialize_habits(&habits), Some(&hash)) {
            Ok(_) => return Ok(result),
            Err(SaveError::Conflict) if attempt < 2 => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    unreachable!("the loop returns")
}

/// MonoCode `loadHabits`.
pub fn load_habits(dir: &Path) -> Result<Vec<Habit>, String> {
    let (text, _) = files::read(dir, HABITS_FILE).map_err(|err| format!("{err:#}"))?;
    Ok(parse_habits(text.as_deref().unwrap_or("")))
}

/// MonoCode `habitStarting`: due and waiting for the scheduler to pick it up.
pub fn habit_starting(habit: &Habit, now: i64) -> bool {
    habit.run_requested || (habit.enabled && habit.next_run_at <= now)
}

/// "Oct 8, 2026, 09:00": MonoCode's `toLocaleString`.
pub fn local_time(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%b %-d, %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// MonoCode `habitView`: a habit as `habits.*` returns it.
pub fn habit_view(habit: &Habit, running: bool) -> Value {
    let mut out = json!({
        "id": habit.id,
        "name": habit.name,
        "instructions": habit.instructions,
        "schedule": habit.schedule.label(),
        "enabled": habit.enabled,
        "nextRun": habit.enabled.then(|| local_time(habit.next_run_at)),
    });
    if running {
        out["running"] = json!(true);
    }
    if let Some(at) = habit.last_run_at {
        out["lastRun"] = json!(local_time(at));
        out["lastOutcome"] = json!(habit.last_outcome);
        if let Some(error) = &habit.last_error {
            out["lastError"] = json!(error);
        }
    }
    out
}

/// MonoCode `habitRunPrompt`'s `<habit_run>`: what the hidden run is asked,
/// after the Mono's whole context.
pub fn habit_run_brief(habit: &Habit, now: i64) -> String {
    let last = match habit.last_run_at {
        None => " This is its first run.".to_string(),
        Some(at) => format!(
            " It last ran {}{}.",
            local_time(at),
            match habit.last_outcome {
                Some(HabitOutcome::Quiet) => " and found nothing worth telling the user",
                Some(HabitOutcome::Failed) => " and could not finish",
                _ => "",
            }
        ),
    };
    format!(
        "<habit_run>\nYou are running one of your habits on your own; the user did not just \
         message you, and they will not see this run. It is {}.{last}\n\nHabit: {}\n{}\n\nDo the \
         work now with your tools and the app CLI. Then reply with exactly the message you would \
         send the user in your chat: short, in your own voice, only what they would want to \
         know, with no preamble about running a habit. If there is nothing worth telling them, \
         reply with exactly {QUIET_MARKER} and nothing else; silence is the right answer more \
         often than not. Save anything worth remembering with the memory actions before you \
         reply.\n</habit_run>",
        local_time(now),
        habit.name,
        habit.instructions.trim()
    )
}

/// MonoCode `habitReport`: what to post, or nothing when the run chose to
/// stay quiet.
pub fn habit_report(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bare: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '`' | '*' | '.') && !c.is_whitespace())
        .collect();
    if bare == QUIET_MARKER {
        return None;
    }
    // A run that narrated before deciding to stay quiet ends on the marker.
    if trimmed.ends_with(QUIET_MARKER) && trimmed.len() < 400 {
        return None;
    }
    Some(
        trimmed
            .strip_suffix(QUIET_MARKER)
            .unwrap_or(trimmed)
            .trim()
            .to_string(),
    )
}

/// MonoCode `HabitToday`: a habit's place in today.
#[derive(Clone, Debug, PartialEq)]
pub struct HabitToday<'a> {
    pub habit: &'a Habit,
    pub state: TodayState,
    /// When it started, starts, or ran.
    pub at: i64,
    pub outcome: Option<HabitOutcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodayState {
    Running,
    Starting,
    Upcoming,
    Done,
}

fn same_day(a: i64, b: i64) -> bool {
    let day = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .ok()
            .map(|at| at.to_zoned(jiff::tz::TimeZone::system()).date())
    };
    day(a).is_some() && day(a) == day(b)
}

/// MonoCode `habitsToday`: one row per habit with something today, running
/// or starting first, then the day in order.
pub fn habits_today<'a>(
    habits: &'a [Habit],
    now: i64,
    running_since: impl Fn(&str) -> Option<i64>,
) -> Vec<HabitToday<'a>> {
    let mut items = Vec::new();
    for habit in habits {
        if let Some(since) = running_since(&habit.id) {
            items.push(HabitToday {
                habit,
                state: TodayState::Running,
                at: since,
                outcome: None,
            });
            continue;
        }
        if habit_starting(habit, now) {
            items.push(HabitToday {
                habit,
                state: TodayState::Starting,
                at: now,
                outcome: None,
            });
            continue;
        }
        if habit.enabled && habit.next_run_at > now && same_day(habit.next_run_at, now) {
            items.push(HabitToday {
                habit,
                state: TodayState::Upcoming,
                at: habit.next_run_at,
                outcome: None,
            });
            continue;
        }
        let last = habit.runs.as_ref().and_then(|runs| runs.first());
        if let Some(at) = last.map(|run| run.at).or(habit.last_run_at)
            && same_day(at, now)
        {
            items.push(HabitToday {
                habit,
                state: TodayState::Done,
                at,
                outcome: last.map(|run| run.outcome).or(habit.last_outcome),
            });
        }
    }
    let rank = |state: TodayState| match state {
        TodayState::Running => 0,
        TodayState::Starting => 1,
        TodayState::Done | TodayState::Upcoming => 2,
    };
    items.sort_by(|a, b| rank(a.state).cmp(&rank(b.state)).then(a.at.cmp(&b.at)));
    items
}

/// MonoCode `HabitsList`'s order: on ones first, soonest first.
pub fn ordered(habits: &[Habit]) -> Vec<&Habit> {
    let mut list: Vec<&Habit> = habits.iter().collect();
    list.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then(a.next_run_at.cmp(&b.next_run_at))
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"[
        {"id":"h1","name":"CI","instructions":"Check CI","schedule":{"scheduleKind":"weekdays","minute":0,"time":"09:00","dayOfWeek":1},"enabled":true,"createdAt":1,"nextRunAt":20,"futureField":{"a":1}},
        {"id":"h2","name":"PRs","instructions":"Look","schedule":{"scheduleKind":"weekly","time":"16:30","dayOfWeek":5},"enabled":false,"nextRunAt":10,"lastOutcome":"quiet","runs":[{"at":5,"durationMs":3,"outcome":"quiet"},{"at":4,"outcome":"odd"}]},
        {"id":"bad","name":"No schedule","instructions":"x"},
        {"id":"odd","name":"Odd","instructions":"x","schedule":{"scheduleKind":"yearly"}}
    ]"#;

    #[test]
    fn habits_parse_as_monocode_writes_them() {
        let habits = parse_habits(FILE);
        assert_eq!(habits.len(), 2);
        assert_eq!(habits[0].schedule.label(), "Weekdays at 09:00");
        assert_eq!(habits[1].schedule.label(), "Friday at 16:30");
        assert_eq!(habits[1].last_outcome, Some(HabitOutcome::Quiet));
        assert_eq!(habits[1].runs.as_ref().unwrap().len(), 1);
        let order: Vec<&str> = ordered(&habits).iter().map(|h| h.id.as_str()).collect();
        assert_eq!(order, ["h1", "h2"]);
        assert!(parse_habits("").is_empty());
        assert!(parse_habits("{}").is_empty());
        // What BenCode does not model survives a save.
        let saved: Value = serde_json::from_str(&serialize_habits(&habits)).unwrap();
        assert_eq!(saved[0]["futureField"], json!({ "a": 1 }));
        assert_eq!(saved[0]["schedule"]["scheduleKind"], "weekdays");
        assert!(saved[0].get("runRequested").is_none());
    }

    #[test]
    fn schedules_from_the_cli_are_checked() {
        let schedule = habit_schedule(Some(
            &json!({ "kind": "weekly", "time": "16:30", "dayOfWeek": 5 }),
        ))
        .unwrap();
        assert_eq!(schedule.label(), "Friday at 16:30");
        let hourly = habit_schedule(Some(&json!({ "kind": "hourly", "minute": 15 }))).unwrap();
        assert_eq!((hourly.minute, hourly.time.as_str()), (15, "09:00"));
        for bad in [
            json!({ "kind": "yearly" }),
            json!({ "kind": "daily", "time": "9:00" }),
            json!({ "kind": "daily", "minute": 60 }),
            json!({ "kind": "daily", "at": "x" }),
            json!("daily"),
        ] {
            assert!(habit_schedule(Some(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn due_runs_and_skips() {
        let mut habits = parse_habits(FILE);
        let now = 3 * 60 * 60 * 1000;
        habits[0].next_run_at = now - 1000;
        let (run, skipped) = due_habits(&habits, now, &HashSet::new());
        assert_eq!((run.len(), skipped.len()), (1, 0));
        habits[0].next_run_at = now - MISSED_RUN_GRACE_MS - 1;
        let (run, skipped) = due_habits(&habits, now, &HashSet::new());
        assert_eq!((run.len(), skipped.len()), (0, 1));
        habits[1].run_requested = true;
        let running: HashSet<String> = ["h1".to_string()].into();
        let (run, skipped) = due_habits(&habits, now, &running);
        assert_eq!(run[0].id, "h2");
        assert!(skipped.is_empty());
    }

    #[test]
    fn a_run_is_remembered() {
        let habits = parse_habits(FILE);
        let now = 1_760_000_000_000;
        let after = after_run(
            &habits[1],
            now,
            Some(&RunResult {
                outcome: HabitOutcome::Failed,
                error: Some("boom".into()),
                report: None,
                started_at: now - 500,
            }),
        );
        assert_eq!(after.last_outcome, Some(HabitOutcome::Failed));
        assert_eq!(after.last_error.as_deref(), Some("boom"));
        let runs = after.runs.unwrap();
        assert_eq!((runs.len(), runs[0].duration_ms), (2, 500));
        assert!(after.next_run_at > now);
        let skipped = after_run(&habits[0], now, None);
        assert_eq!(skipped.last_run_at, None);
    }

    #[test]
    fn a_quiet_run_posts_nothing() {
        assert_eq!(habit_report("NOTHING_TO_REPORT"), None);
        assert_eq!(habit_report(" `NOTHING_TO_REPORT`. "), None);
        assert_eq!(
            habit_report("Checked CI, all green.\n\nNOTHING_TO_REPORT"),
            None
        );
        assert_eq!(
            habit_report("CI is red on main."),
            Some("CI is red on main.".into())
        );
        assert_eq!(habit_report("   "), None);
    }

    #[test]
    fn the_file_changes_over_what_was_read() {
        let dir =
            std::env::temp_dir().join(format!("bencode-habits-{}", crate::app_cli::random_hex(6)));
        files::load(&dir).unwrap();
        let schedule = habit_schedule(Some(&json!({ "kind": "daily" }))).unwrap();
        let added = update_habits(&dir, |mut list| {
            let habit = new_habit("CI", "Check CI", schedule.clone(), 1);
            list.push(habit.clone());
            Ok((list, habit))
        })
        .unwrap();
        assert_eq!(load_habits(&dir).unwrap()[0].id, added.id);
        std::fs::remove_dir_all(&dir).ok();
    }
}
