//! MonoCode `features/monos/model/monoHabits.ts`, the model only: a habit is
//! something a Mono does on its own on a schedule, kept in its folder's
//! `habits.json`. MonoCode's agent adds them through its `app` CLI and its
//! scheduler runs them; BenCode has neither yet, so it reads the list for
//! the Overview's Habits tab and runs nothing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schedule::ScheduleKind;

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
}

/// MonoCode `Habit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Habit {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub schedule: HabitSchedule,
    pub enabled: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    /// "posted", "quiet" or "failed".
    pub last_outcome: Option<String>,
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn number(value: &Value, key: &str) -> Option<i64> {
    value.get(key)?.as_f64().filter(|n| n.is_finite()).map(|n| n as i64)
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
                next_run_at: number(raw, "nextRunAt").unwrap_or(0),
                last_run_at: number(raw, "lastRunAt"),
                last_outcome: string(raw, "lastOutcome")
                    .filter(|o| matches!(o.as_str(), "posted" | "quiet" | "failed")),
            })
        })
        .collect()
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

    #[test]
    fn habits_parse_as_monocode_writes_them() {
        let json = r#"[
            {"id":"h1","name":"CI","instructions":"Check CI","schedule":{"scheduleKind":"weekdays","minute":0,"time":"09:00","dayOfWeek":1},"enabled":true,"createdAt":1,"nextRunAt":20},
            {"id":"h2","name":"PRs","instructions":"Look","schedule":{"scheduleKind":"weekly","time":"16:30","dayOfWeek":5},"enabled":false,"nextRunAt":10,"lastOutcome":"quiet"},
            {"id":"bad","name":"No schedule","instructions":"x"},
            {"id":"odd","name":"Odd","instructions":"x","schedule":{"scheduleKind":"yearly"}}
        ]"#;
        let habits = parse_habits(json);
        assert_eq!(habits.len(), 2);
        assert_eq!(habits[0].schedule.label(), "Weekdays at 09:00");
        assert_eq!(habits[1].schedule.label(), "Friday at 16:30");
        assert_eq!(habits[1].last_outcome.as_deref(), Some("quiet"));
        let order: Vec<&str> = ordered(&habits).iter().map(|h| h.id.as_str()).collect();
        assert_eq!(order, ["h1", "h2"]);
        assert!(parse_habits("").is_empty());
        assert!(parse_habits("{}").is_empty());
    }
}
