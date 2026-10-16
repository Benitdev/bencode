//! A Mono's habits on the app (MonoCode `useMonoHabits.ts` and
//! `App.tsx`'s habit host, `agentApp.ts` `handleHabits`): the `habits.*`
//! calls, the scheduler that runs them while BenCode is open, each run in
//! a hidden thread of its own with full access, an approval it still asks
//! for put to the user in the Mono's chat, and what it decides to say
//! posted there. One run at a time across all Monos, so waking the laptop
//! never starts several agents at once.
//!
//! A run's thread is never saved, listed or opened (`is_habit_run`): it
//! goes once the run ends, and a crash leaves nothing behind.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use gpui::Context;
use serde_json::{Map, Value, json};

use crate::app::BenCodeApp;
use crate::app::agent::TurnInput;
use crate::app::app_calls::Caller;
use crate::app_cli::artifacts::ArtifactCard;
use crate::app_cli::input::required;
use crate::app_cli::server::AppCall;
use crate::db::Block;
use crate::monos::habits::{self, Habit, HabitOutcome, RunResult};

/// MonoCode `TICK_MS`.
const TICK: Duration = Duration::from_secs(60);
/// MonoCode `POLL_MS`: how often a run is checked for approvals and time.
const POLL: Duration = Duration::from_secs(2);

/// A habit's run in flight.
pub struct HabitRunState {
    pub mono: String,
    pub mono_session: String,
    pub habit: Habit,
    pub started_at: i64,
    working_ms: i64,
    waiting_ms: i64,
    /// Why the run was stopped, when BenCode stopped it.
    failure: Option<String>,
    /// Sessions it started, for its report (MonoCode `habitRunLaunches`).
    pub launches: Vec<Value>,
}

#[derive(Default)]
pub struct HabitsState {
    /// Habits running now, by id, with when each started.
    running: HashMap<String, i64>,
    /// Runs in flight, by their hidden thread.
    pub runs: HashMap<String, HabitRunState>,
    /// Due habits waiting their turn: (Mono, habit id).
    queue: VecDeque<(String, String)>,
    /// A check or a run is under way.
    busy: bool,
    /// A check asked for meanwhile happens as soon as it ends.
    check_again: bool,
}

impl BenCodeApp {
    /// MonoCode `useMonoHabits`: checks for due habits now and every minute.
    pub fn start_habit_scheduler(&mut self, cx: &mut Context<Self>) {
        self.check_habits(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |app, cx| app.check_habits(cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// MonoCode `isHabitRun`.
    pub fn is_habit_run(&self, session_id: &str) -> bool {
        self.habits.runs.contains_key(session_id)
    }

    /// MonoCode `habitRunMono`: the Mono a run works for.
    pub(crate) fn habit_run_mono(&self, session_id: &str) -> Option<String> {
        self.habits.runs.get(session_id).map(|run| run.mono.clone())
    }

    /// MonoCode `habitRunningSince`.
    pub fn habit_running_since(&self, habit_id: &str) -> Option<i64> {
        self.habits.running.get(habit_id).copied()
    }

    /// The runs of `mono` waiting on an approval, for its chat: (run
    /// thread, habit name).
    pub fn habit_runs_asking(&self, mono_session: &str) -> Vec<(String, String)> {
        let mut asking: Vec<(String, String)> = self
            .habits
            .runs
            .iter()
            .filter(|(id, run)| run.mono_session == mono_session && self.pending_permission_for(id).is_some())
            .map(|(id, run)| (id.clone(), run.habit.name.clone()))
            .collect();
        asking.sort();
        asking
    }

    /// MonoCode `checkHabitsNow` and the tick: due habits queued, stale
    /// ones moved on, then the queue run one at a time.
    pub(crate) fn check_habits(&mut self, cx: &mut Context<Self>) {
        if self.habits.busy {
            self.habits.check_again = true;
            return;
        }
        if self.settings.monos_off {
            return;
        }
        // Habits are set up in its chat, so a Mono never opened has none.
        let monos: Vec<(String, std::path::PathBuf)> = self
            .settings
            .monos
            .iter()
            .filter(|mono| mono.session_id.is_some())
            .filter_map(|mono| {
                crate::app::mono_files::mono_files_dir(&mono.id).map(|dir| (mono.id.clone(), dir))
            })
            .collect();
        if monos.is_empty() {
            return;
        }
        self.habits.busy = true;
        self.habits.check_again = false;
        let running: HashSet<String> = self.habits.running.keys().cloned().collect();
        let task = cx.background_executor().spawn(async move {
            let now = crate::app::now_ms();
            let mut due = Vec::new();
            for (mono, dir) in monos {
                let habits = match habits::load_habits(&dir) {
                    Ok(habits) => habits,
                    Err(err) => {
                        log::warn!("could not read a Mono's habits: {err}");
                        continue;
                    }
                };
                let (run, skipped) = habits::due_habits(&habits, now, &running);
                if !skipped.is_empty() {
                    let moved = habits::update_habits(&dir, |list| {
                        let now = crate::app::now_ms();
                        let stale: HashSet<String> = habits::due_habits(&list, now, &running)
                            .1
                            .iter()
                            .map(|habit| habit.id.clone())
                            .collect();
                        let next = list
                            .iter()
                            .map(|habit| {
                                if stale.contains(&habit.id) {
                                    habits::after_run(habit, now, None)
                                } else {
                                    habit.clone()
                                }
                            })
                            .collect();
                        Ok((next, ()))
                    });
                    if let Err(err) = moved {
                        log::warn!("could not move missed habits on: {err}");
                    }
                }
                due.extend(run.into_iter().map(|habit| (mono.clone(), habit.id.clone())));
            }
            due
        });
        cx.spawn(async move |this, cx| {
            let due = task.await;
            let landed = this.update(cx, |this, cx| {
                this.habits.queue.extend(due);
                this.start_next_habit(cx);
            });
            if let Err(err) = landed {
                log::debug!("habits checked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The next queued habit, claimed from the latest file; when the queue
    /// is empty the scheduler is free again.
    fn start_next_habit(&mut self, cx: &mut Context<Self>) {
        let Some((mono, habit_id)) = self.habits.queue.pop_front() else {
            self.habits.busy = false;
            if self.habits.check_again {
                self.check_habits(cx);
            }
            return;
        };
        let Some(mono_session) = self.mono(&mono).and_then(|m| m.session_id.clone()) else {
            self.start_next_habit(cx);
            return;
        };
        // Its conversation is where the run reports: in memory first.
        if !self.sessions.iter().any(|s| s.id == mono_session) {
            self.prepare_mono_session(&mono, cx);
            self.habits.queue.push_front((mono, habit_id));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let landed = this.update(cx, |this, cx| {
                    let loaded = this.habits.queue.front().is_some_and(|(mono, _)| {
                        this.mono(mono)
                            .and_then(|m| m.session_id.as_deref())
                            .is_some_and(|id| this.sessions.iter().any(|s| s.id == id))
                    });
                    if !loaded {
                        // Give it up for this tick; the next one tries again.
                        log::warn!("a Mono's conversation did not load; its habit waits");
                        this.habits.queue.pop_front();
                    }
                    this.start_next_habit(cx);
                });
                if let Err(err) = landed {
                    log::debug!("habit after app drop: {err:#}");
                }
            })
            .detach();
            return;
        }
        let Some(dir) = crate::app::mono_files::mono_files_dir(&mono) else {
            self.start_next_habit(cx);
            return;
        };
        let running: HashSet<String> = self.habits.running.keys().cloned().collect();
        // MonoCode `claimHabit`: from the latest definition, as it may
        // have been removed, paused or rescheduled since.
        let task = cx.background_executor().spawn(async move {
            let files = crate::monos::files::load(&dir).map_err(|err| format!("{err:#}"))?;
            let now = crate::app::now_ms();
            let claimed = habits::update_habits(&dir, |list| {
                let only: Vec<Habit> = list.iter().filter(|h| h.id == habit_id).cloned().collect();
                let claimed = habits::due_habits(&only, now, &running).0.first().map(|h| (*h).clone());
                let next = list
                    .iter()
                    .map(|habit| match &claimed {
                        Some(claimed) if claimed.id == habit.id => habits::after_run(habit, now, None),
                        _ => habit.clone(),
                    })
                    .collect();
                Ok((next, claimed))
            })?;
            Ok::<_, String>((files, claimed, now))
        });
        cx.spawn(async move |this, cx| {
            let claimed = task.await;
            let landed = this.update(cx, |this, cx| match claimed {
                Ok((files, Some(habit), now)) => {
                    this.launch_habit_run(&mono, &mono_session, &files, habit, now, cx)
                }
                Ok((_, None, _)) => this.start_next_habit(cx),
                Err(err) => {
                    log::warn!("could not claim a habit: {err}");
                    this.start_next_habit(cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("habit claimed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `runHabit`: the hidden thread, with the Mono's model and
    /// full access, sent its whole context and the habit.
    fn launch_habit_run(
        &mut self,
        mono_id: &str,
        mono_session: &str,
        files: &crate::monos::files::MonoFiles,
        habit: Habit,
        now: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(mono) = self.mono(mono_id).cloned() else {
            self.start_next_habit(cx);
            return;
        };
        let Some(source) = self.sessions.iter().find(|s| s.id == mono_session).cloned() else {
            self.start_next_habit(cx);
            return;
        };
        let run_id = format!("habit-run-{}", crate::app::ids::unique_id(&habit.id));
        let mut row = crate::db::SessionRow {
            id: run_id.clone(),
            title: format!("{} · {}", mono.display_name(), habit.name),
            cwd: source.cwd.clone(),
            harness: source.harness.clone(),
            model: source.model.clone(),
            created_at: now,
            updated_at: now,
            // Nobody is watching a habit's run, so it cannot stop to ask
            // before each step; what the harness still asks goes to the chat.
            runtime_mode: Some(crate::app::PermissionMode::FullAccess.id().to_string()),
            model_settings: source.model_settings.clone(),
            provider_account_id: source.provider_account_id.clone(),
            ..Default::default()
        };
        row.blocks.clear();
        self.habits.runs.insert(
            run_id.clone(),
            HabitRunState {
                mono: mono_id.to_string(),
                mono_session: mono_session.to_string(),
                habit: habit.clone(),
                started_at: now,
                working_ms: 0,
                waiting_ms: 0,
                failure: None,
                launches: Vec::new(),
            },
        );
        self.habits.running.insert(habit.id.clone(), now);
        self.sessions.push(row);
        let (soul, memory) = self.mono_context_parts(&mono, files, self.app_access.socket.is_some());
        let prompt = format!("{soul}\n\n{memory}\n\n{}", habits::habit_run_brief(&habit, now));
        self.send_turn(
            &run_id,
            TurnInput {
                text: habit.name.clone(),
                agent_prompt: Some(prompt),
                ..Default::default()
            },
            cx,
        );
        if !self.is_agent_running_in(&run_id) {
            if let Some(run) = self.habits.runs.get_mut(&run_id) {
                run.failure = Some("The habit's run could not start.".into());
            }
            self.habit_run_settled(&run_id, false, cx);
            return;
        }
        self.watch_habit_run(run_id, cx);
        self.refresh_mono_overview(mono_id, cx);
        cx.notify();
    }

    /// MonoCode `watchRun`: relays an approval to the Mono's chat, and
    /// stops a run that worked an hour or waited half a day on the user.
    fn watch_habit_run(&mut self, run_id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let more = this.update(cx, |this, cx| this.poll_habit_run(&run_id, cx));
                if !matches!(more, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    fn poll_habit_run(&mut self, run_id: &str, cx: &mut Context<Self>) -> bool {
        let asking = self.pending_permission_for(run_id).cloned();
        let Some(run) = self.habits.runs.get_mut(run_id) else {
            return false;
        };
        let step = POLL.as_millis() as i64;
        if asking.is_some() {
            run.waiting_ms += step;
        } else {
            run.working_ms += step;
        }
        let failure = if run.waiting_ms >= habits::HABIT_APPROVAL_WAIT_MS {
            Some("Nobody answered its approval request.")
        } else if run.working_ms >= habits::HABIT_RUN_TIMEOUT_MS {
            Some("It took too long and was stopped.")
        } else {
            None
        };
        if let Some(failure) = failure {
            run.failure = Some(failure.to_string());
            self.stop_agent(run_id, cx);
            return false;
        }
        true
    }

    /// A run's turn ended (`completed`) or was stopped: its report posted,
    /// the habit's record updated, its thread dropped, and the next one
    /// started (MonoCode `runHabit`'s end).
    pub(crate) fn habit_run_settled(&mut self, run_id: &str, completed: bool, cx: &mut Context<Self>) {
        let Some(run) = self.habits.runs.remove(run_id) else {
            return;
        };
        self.habits.running.remove(&run.habit.id);
        let blocks = self
            .sessions
            .iter()
            .find(|s| s.id == run_id)
            .map(|s| s.blocks.clone())
            .unwrap_or_default();
        self.sessions.retain(|s| s.id != run_id);
        self.transcripts.remove(run_id);
        self.threads.remove(run_id);
        let completed = completed && run.failure.is_none();
        let report = completed
            .then(|| habits::habit_report(&final_reply(&blocks)))
            .flatten();
        let cards = crate::app_cli::artifacts::chat_cards(&blocks);
        // Sessions it started are listed only in the Mono's chat, so a run
        // that started one posts even with nothing else to say.
        let text = report.clone().or_else(|| match run.launches.len() {
            0 => None,
            1 => Some("Started a session.".to_string()),
            n => Some(format!("Started {n} sessions.")),
        });
        if let Some(text) = text {
            self.post_habit_message(&run, &text, cards, cx);
        }
        let outcome = if !completed {
            HabitOutcome::Failed
        } else if report.is_some() {
            HabitOutcome::Posted
        } else {
            HabitOutcome::Quiet
        };
        let result = RunResult {
            outcome,
            error: (!completed).then(|| {
                run.failure
                    .clone()
                    .unwrap_or_else(|| "The run did not finish.".to_string())
            }),
            report,
            started_at: run.started_at,
        };
        if let Some(dir) = crate::app::mono_files::mono_files_dir(&run.mono) {
            let habit_id = run.habit.id.clone();
            let mono = run.mono.clone();
            let task = cx.background_executor().spawn(async move {
                habits::update_habits(&dir, |list| {
                    let now = crate::app::now_ms();
                    let next = list
                        .iter()
                        .map(|habit| {
                            if habit.id == habit_id {
                                habits::after_run(habit, now, Some(&result))
                            } else {
                                habit.clone()
                            }
                        })
                        .collect();
                    Ok((next, ()))
                })
            });
            cx.spawn(async move |this, cx| {
                if let Err(err) = task.await {
                    log::warn!("could not record a habit's run: {err}");
                }
                let landed = this.update(cx, |this, cx| this.refresh_mono_overview(&mono, cx));
                if let Err(err) = landed {
                    log::debug!("habit recorded after app drop: {err:#}");
                }
            })
            .detach();
        }
        self.start_next_habit(cx);
        cx.notify();
    }

    /// MonoCode's habit `post`: a message in the Mono's chat, outside any
    /// turn, with the cards and sessions the run made; the user hears of it.
    fn post_habit_message(
        &mut self,
        run: &HabitRunState,
        text: &str,
        cards: Vec<ArtifactCard>,
        cx: &mut Context<Self>,
    ) {
        let launches = &run.launches;
        let now = crate::app::now_ms();
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == run.mono_session) else {
            return;
        };
        let mut block = Block::new(crate::app::ids::unique_id("habit-post"), "assistant", text);
        block.extra.insert(
            "monoHabit".into(),
            json!({ "id": run.habit.id, "name": run.habit.name, "at": now }),
        );
        if !cards.is_empty()
            && let Ok(value) = serde_json::to_value(&cards)
        {
            block.extra.insert("artifactCards".into(), value);
        }
        if !launches.is_empty() {
            block
                .extra
                .insert("monoSpawnedSessions".into(), Value::Array(launches.to_vec()));
        }
        session.blocks.push(block);
        session.updated_at = now;
        let id = run.mono_session.clone();
        self.persist_session(&id);
        self.note_mono_reply(&id, cx);
        self.announce_turn_finished(&id, cx);
        cx.notify();
    }

    /// The Overview's habits and board read again, for the Mono in view.
    pub(crate) fn refresh_mono_overview(&mut self, mono: &str, cx: &mut Context<Self>) {
        if self.monos.view.as_deref() == Some(mono) {
            self.load_mono_overview(mono, cx);
        }
    }

    /// `habits.*` from a Mono's own conversation.
    pub(crate) fn habits_call(
        &mut self,
        caller: Caller,
        action: &str,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = crate::app::mono_files::mono_files_dir(&caller.mono) else {
            call.fail("This Mono's folder is unavailable", true);
            return;
        };
        let running: HashMap<String, i64> = self.habits.running.clone();
        let action = action.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { habits_action(&dir, &action, &input, &running, crate::app::now_ms()) });
        let mono = caller.mono;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                match result {
                    Ok((value, run_now)) => {
                        call.answer(value);
                        if run_now {
                            this.check_habits(cx);
                        }
                    }
                    Err(error) => call.fail(error, true),
                }
                this.refresh_mono_overview(&mono, cx);
            });
            if let Err(err) = landed {
                log::debug!("habit call after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The Habits panel's actions on the user's side (MonoCode
    /// `MonoHabits`, `HabitPage`, `NewHabitPage`): add, change, pause,
    /// run now, remove, all through `habits.json`.
    pub(crate) fn edit_mono_habits(
        &mut self,
        mono: &str,
        change: impl Fn(Vec<Habit>, i64) -> Result<Vec<Habit>, String> + Send + 'static,
        run_now: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = crate::app::mono_files::mono_files_dir(mono) else {
            return;
        };
        let task = cx.background_executor().spawn(async move {
            habits::update_habits(&dir, |list| {
                let next = change(list, crate::app::now_ms())?;
                Ok((next, ()))
            })
        });
        let mono = mono.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                if let Err(err) = &result {
                    log::warn!("could not change the Mono's habits: {err}");
                    this.monos.overview.habit_error = Some(err.clone());
                } else {
                    this.monos.overview.habit_error = None;
                }
                if run_now && result.is_ok() {
                    this.check_habits(cx);
                }
                this.refresh_mono_overview(&mono, cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("habits changed after app drop: {err:#}");
            }
        })
        .detach();
    }
}

/// MonoCode `finalReply`: the last thing the run said.
fn final_reply(blocks: &[Block]) -> String {
    for block in blocks.iter().rev() {
        if block.role == "user" {
            break;
        }
        let text = block.text.as_deref().unwrap_or("");
        if block.role == "assistant" && block.tool.is_none() && !text.trim().is_empty() {
            return text.to_string();
        }
    }
    String::new()
}

/// MonoCode `handleHabits`, on the Mono's folder: the answer, and whether
/// the scheduler should look now (`habits.run`).
pub fn habits_action(
    dir: &std::path::Path,
    action: &str,
    input: &Map<String, Value>,
    running: &HashMap<String, i64>,
    now: i64,
) -> Result<(Value, bool), String> {
    let view = |habit: &Habit| habits::habit_view(habit, running.contains_key(&habit.id));
    let find = |list: &[Habit]| -> Result<Habit, String> {
        let id = required(input.get("id"), "id", 128)?;
        list.iter()
            .find(|habit| habit.id == id)
            .cloned()
            .ok_or_else(|| "No habit with that id; run habits.list".to_string())
    };
    match action {
        "habits.list" => {
            let list = habits::load_habits(dir)?;
            Ok((json!({ "habits": list.iter().map(view).collect::<Vec<_>>() }), false))
        }
        "habits.add" => {
            let name = required(input.get("name"), "name", 80)?;
            let instructions = required(input.get("instructions"), "instructions", 4_000)?;
            let schedule = habits::habit_schedule(input.get("schedule"))?;
            let habit = habits::update_habits(dir, |mut list| {
                if list.len() >= habits::HABITS_MAX {
                    return Err(format!("A Mono can have at most {} habits", habits::HABITS_MAX));
                }
                let created = habits::new_habit(&name, &instructions, schedule.clone(), now);
                list.push(created.clone());
                Ok((list, created))
            })?;
            Ok((json!({ "habit": view(&habit) }), false))
        }
        "habits.update" => {
            let name = input
                .get("name")
                .map(|v| required(Some(v), "name", 80))
                .transpose()?;
            let instructions = input
                .get("instructions")
                .map(|v| required(Some(v), "instructions", 4_000))
                .transpose()?;
            let schedule = input
                .get("schedule")
                .map(|v| habits::habit_schedule(Some(v)))
                .transpose()?;
            let enabled = match input.get("enabled") {
                None => None,
                Some(Value::Bool(on)) => Some(*on),
                Some(_) => return Err("enabled must be true or false".into()),
            };
            let habit = habits::update_habits(dir, |list| {
                let current = find(&list)?;
                let mut next = current.clone();
                if let Some(name) = &name {
                    next.name = name.clone();
                }
                if let Some(instructions) = &instructions {
                    next.instructions = instructions.clone();
                }
                if let Some(schedule) = &schedule {
                    next.schedule = schedule.clone();
                }
                if let Some(on) = enabled {
                    next.enabled = on;
                }
                // A new schedule, or turning it back on, counts from now.
                if schedule.is_some() || (next.enabled && !current.enabled) {
                    next.next_run_at = next.schedule.next_run_at(now);
                }
                let list = list
                    .into_iter()
                    .map(|habit| if habit.id == next.id { next.clone() } else { habit })
                    .collect();
                Ok((list, next))
            })?;
            Ok((json!({ "habit": view(&habit) }), false))
        }
        "habits.run" => {
            let id = required(input.get("id"), "id", 128)?;
            if let Some(since) = running.get(&id) {
                return Err(format!(
                    "That habit is already running (started {}); its result will be posted here when it ends",
                    habits::local_time(*since)
                ));
            }
            let habit = habits::update_habits(dir, |list| {
                let mut next = find(&list)?;
                next.run_requested = true;
                let list = list
                    .into_iter()
                    .map(|habit| if habit.id == next.id { next.clone() } else { habit })
                    .collect();
                Ok((list, next))
            })?;
            Ok((
                json!({
                    "habit": view(&habit),
                    "note": "It starts now, on its own. Whatever it finds is posted to this chat, or nothing if there is nothing to say.",
                }),
                true,
            ))
        }
        "habits.remove" => {
            let removed = habits::update_habits(dir, |list| {
                let habit = find(&list)?;
                let list = list.into_iter().filter(|h| h.id != habit.id).collect();
                Ok((list, habit))
            })?;
            Ok((json!({ "removed": removed.name }), false))
        }
        _ => Err(format!("Unknown app action: {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn habits_are_managed_through_the_cli() {
        let dir = std::env::temp_dir().join(format!("bencode-habit-calls-{}", crate::app_cli::random_hex(6)));
        crate::monos::files::load(&dir).unwrap();
        let none = HashMap::new();
        let now = 1_760_000_000_000;
        let (added, _) = habits_action(
            &dir,
            "habits.add",
            &input(json!({ "name": "CI", "instructions": "Check CI", "schedule": { "kind": "weekdays", "time": "09:00" } })),
            &none,
            now,
        )
        .unwrap();
        let id = added["habit"]["id"].as_str().unwrap().to_string();
        assert_eq!(added["habit"]["schedule"], "Weekdays at 09:00");
        let (paused, _) = habits_action(&dir, "habits.update", &input(json!({ "id": id, "enabled": false })), &none, now).unwrap();
        assert_eq!(paused["habit"]["enabled"], false);
        assert_eq!(paused["habit"]["nextRun"], Value::Null);
        let (_, run_now) = habits_action(&dir, "habits.run", &input(json!({ "id": id })), &none, now).unwrap();
        assert!(run_now);
        assert!(habits::load_habits(&dir).unwrap()[0].run_requested);
        let busy: HashMap<String, i64> = [(id.clone(), now)].into();
        assert!(habits_action(&dir, "habits.run", &input(json!({ "id": id })), &busy, now).is_err());
        let (listed, _) = habits_action(&dir, "habits.list", &input(json!({})), &busy, now).unwrap();
        assert_eq!(listed["habits"][0]["running"], true);
        assert!(habits_action(&dir, "habits.remove", &input(json!({ "id": "nope" })), &none, now).is_err());
        let (removed, _) = habits_action(&dir, "habits.remove", &input(json!({ "id": id })), &none, now).unwrap();
        assert_eq!(removed["removed"], "CI");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_report_is_the_last_reply() {
        let mut tool = Block::new("t", "assistant", "");
        tool.tool = Some(json!({ "kind": "read" }));
        let blocks = vec![
            Block::new("u", "user", "go"),
            Block::new("a1", "assistant", "Looking"),
            tool,
            Block::new("a2", "assistant", "CI is red."),
        ];
        assert_eq!(final_reply(&blocks), "CI is red.");
        assert_eq!(final_reply(&blocks[..1]), "");
    }
}
