//! MonoCode session reminders on the app side (`sessionReminders.ts`,
//! `sessionReminderPresets.ts`, `useSessionReminders`): the "Remind me"
//! presets, loading and scheduling, claiming due reminders for a desktop
//! alert, and the due list the reminder panel shows.

use std::time::Duration;

use gpui::Context;
use jiff::Zoned;
use jiff::civil::Time;
use jiff::tz::TimeZone;

use crate::app::{BenCodeApp, now_ms};
use crate::db::Reminder;

/// MonoCode's backend claims due reminders every 5s.
const CLAIM_EVERY: Duration = Duration::from_secs(5);
/// MonoCode re-reads the list every 30s.
const REFRESH_EVERY: Duration = Duration::from_secs(30);
const HOUR_MS: i64 = 3_600_000;

/// MonoCode's preset ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    OneHour,
    ThreeHours,
    Evening,
    Tomorrow,
    NextWeek,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::OneHour,
        Preset::ThreeHours,
        Preset::Evening,
        Preset::Tomorrow,
        Preset::NextWeek,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Preset::OneHour => "reminder:1h",
            Preset::ThreeHours => "reminder:3h",
            Preset::Evening => "reminder:evening",
            Preset::Tomorrow => "reminder:tomorrow",
            Preset::NextWeek => "reminder:next-week",
        }
    }

    pub fn from_id(id: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.id() == id)
    }
}

fn zoned(ms: i64) -> Option<Zoned> {
    Some(jiff::Timestamp::from_millisecond(ms).ok()?.to_zoned(TimeZone::system()))
}

/// `day` at `hour`:00 local, in ms.
fn at_hour(day: &Zoned, hour: i8) -> Option<i64> {
    let time = Time::new(hour, 0, 0, 0).ok()?;
    let at = day.date().to_datetime(time).to_zoned(day.time_zone().clone()).ok()?;
    Some(at.timestamp().as_millisecond())
}

/// MonoCode `reminderTime`: when a preset fires, from `now` (ms). Hour
/// presets add time; the others land on 18:00 today, 9:00 tomorrow, or
/// 9:00 next Monday, and are `None` once that has passed.
pub fn reminder_time(preset: Preset, now: i64) -> Option<i64> {
    match preset {
        Preset::OneHour => return Some(now + HOUR_MS),
        Preset::ThreeHours => return Some(now + 3 * HOUR_MS),
        _ => {}
    }
    let today = zoned(now)?;
    let due = match preset {
        Preset::Evening => at_hour(&today, 18)?,
        Preset::Tomorrow => at_hour(&today.tomorrow().ok()?, 9)?,
        Preset::NextWeek => {
            // `(8 - day) % 7 || 7`: always the coming Monday.
            let from_sunday = today.weekday().to_sunday_zero_offset() as i64;
            let ahead = match (8 - from_sunday) % 7 {
                0 => 7,
                days => days,
            };
            let day = today.checked_add(jiff::Span::new().days(ahead)).ok()?;
            at_hour(&day, 9)?
        }
        Preset::OneHour | Preset::ThreeHours => unreachable!("handled above"),
    };
    (due > now).then_some(due)
}

/// MonoCode's `H:MM`: 24-hour, unpadded hour.
fn clock(ms: i64) -> String {
    zoned(ms).map_or_else(String::new, |z| format!("{}:{:02}", z.hour(), z.minute()))
}

/// MonoCode `sessionReminderPresets`: the labels, built when shown, and
/// whether each can still be picked.
pub fn preset_rows(now: i64) -> Vec<(Preset, String, bool)> {
    Preset::ALL
        .into_iter()
        .map(|preset| {
            let label = match preset {
                Preset::OneHour => format!("In 1 hour ({})", clock(now + HOUR_MS)),
                Preset::ThreeHours => format!("In 3 hours ({})", clock(now + 3 * HOUR_MS)),
                Preset::Evening => "This evening (18:00)".into(),
                Preset::Tomorrow => "Tomorrow (9:00)".into(),
                Preset::NextWeek => "Next week (Mon 9:00)".into(),
            };
            (preset, label, reminder_time(preset, now).is_some())
        })
        .collect()
}

/// MonoCode `formatReminderTime`: "Mon, Oct 6, 9:00 AM".
pub fn format_reminder_time(due_at: i64) -> String {
    zoned(due_at).map_or_else(String::new, |z| z.strftime("%a, %b %-d, %-I:%M %p").to_string())
}

impl BenCodeApp {
    /// MonoCode `useSessionReminders` polling: claims due reminders every
    /// 5s (alerting each once) and re-reads the list every 30s.
    pub fn start_reminder_poll(&mut self, cx: &mut Context<Self>) {
        self.refresh_reminders(cx);
        cx.spawn(async move |this, cx| {
            let mut since_refresh = Duration::ZERO;
            loop {
                cx.background_executor().timer(CLAIM_EVERY).await;
                since_refresh += CLAIM_EVERY;
                let refresh = since_refresh >= REFRESH_EVERY;
                if refresh {
                    since_refresh = Duration::ZERO;
                }
                let alive = this.update(cx, |app, cx| {
                    app.claim_due_reminders(cx);
                    if refresh {
                        app.refresh_reminders(cx);
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Re-reads the list off the UI thread; a newer reload wins.
    pub fn refresh_reminders(&mut self, cx: &mut Context<Self>) {
        self.reminder_generation += 1;
        let generation = self.reminder_generation;
        let list = self.db_read(|db| db.list_reminders());
        cx.spawn(async move |this, cx| {
            let Ok(list) = list.await else {
                log::error!("the database writer stopped");
                return;
            };
            let landed = this.update(cx, |app, cx| {
                if app.reminder_generation != generation {
                    return; // a newer reload superseded this one
                }
                match list {
                    Ok(list) => {
                        if list != app.reminders || app.reminder_error.is_some() {
                            app.reminders = list;
                            app.reminder_error = None;
                            cx.notify();
                        }
                    }
                    Err(err) => {
                        log::error!("could not load reminders: {err:#}");
                        app.reminder_error = Some(err.to_string());
                        cx.notify();
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("reminders loaded after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Claims what came due and raises a desktop alert for each.
    fn claim_due_reminders(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let claimed = self.db_read(move |db| db.take_due_reminders(now));
        cx.spawn(async move |this, cx| {
            let claimed = match claimed.await {
                Ok(Ok(claimed)) => claimed,
                Ok(Err(err)) => {
                    log::error!("could not claim due reminders: {err:#}");
                    return;
                }
                Err(_) => {
                    log::error!("the database writer stopped");
                    return;
                }
            };
            if claimed.is_empty() {
                return;
            }
            let landed = this.update(cx, |app, cx| {
                for reminder in &claimed {
                    let title = crate::app::session_list::display_title(&reminder.title, &reminder.harness);
                    cx.background_executor()
                        .spawn(async move { desktop_alert(&title) })
                        .detach();
                }
                app.refresh_reminders(cx);
            });
            if let Err(err) = landed {
                log::debug!("reminders claimed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `due`: reminders whose time has come, soonest first.
    pub fn due_reminders(&self) -> Vec<&Reminder> {
        let now = now_ms();
        self.reminders.iter().filter(|r| r.due_at <= now).collect()
    }

    pub fn reminder_for(&self, session_id: &str) -> Option<&Reminder> {
        self.reminders.iter().find(|r| r.session_id == session_id)
    }

    /// MonoCode `schedule`: sets (or moves) the threads' reminder and opens
    /// the Reminders group.
    pub fn schedule_reminders(&mut self, session_ids: &[String], due_at: i64, cx: &mut Context<Self>) {
        let ids = session_ids.to_vec();
        // On the writer, so a thread saved a moment ago is in the table first.
        self.db_then(
            cx,
            move |db| db.set_reminders(&ids, due_at, now_ms()),
            |this, set, cx| {
                if let Err(err) = set {
                    this.reminder_failure = Some(err.to_string());
                    log::warn!("could not set reminder: {err:#}");
                } else {
                    let project = this.current_cwd.clone();
                    if this.sessions_ui.reminders_collapsed.remove(&project).is_some() {
                        this.save_settings(cx);
                    }
                }
                this.refresh_reminders(cx);
            },
        );
    }

    /// MonoCode `cancel`.
    pub fn cancel_reminders(&mut self, session_ids: &[String], expected_due_at: Option<i64>, cx: &mut Context<Self>) {
        let ids = session_ids.to_vec();
        self.db_then(
            cx,
            move |db| db.clear_reminders(&ids, expected_due_at),
            |this, cleared, cx| {
                if let Err(err) = cleared {
                    this.reminder_failure = Some(err.to_string());
                    log::warn!("could not cancel reminder: {err:#}");
                }
                this.refresh_reminders(cx);
            },
        );
    }

    /// MonoCode `dismissDue`: continuing a thread clears its due reminder
    /// (a later one stays).
    pub fn dismiss_due_reminder(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let due = self
            .reminder_for(session_id)
            .filter(|r| r.due_at <= now_ms())
            .map(|r| r.due_at);
        if let Some(due_at) = due {
            self.cancel_reminders(&[session_id.to_string()], Some(due_at), cx);
        }
    }

    /// MonoCode `openHere`: opens the thread (switching project) and lets
    /// that occurrence go.
    pub fn open_reminder(&mut self, session_id: &str, due_at: i64, cx: &mut Context<Self>) {
        let cwd = self
            .reminders
            .iter()
            .find(|r| r.session_id == session_id)
            .map(|r| r.cwd.clone());
        if let Some(cwd) = cwd
            && !crate::app::same_project_path(&cwd, &self.current_cwd)
        {
            self.switch_project(cwd, cx);
        }
        if self.sessions.iter().any(|s| s.id == session_id) {
            return self.show_reminder_thread(session_id, due_at, cx);
        }
        // A thread of another project is not in memory yet.
        let (id, row) = (session_id.to_string(), session_id.to_string());
        self.db_then(
            cx,
            move |db| db.get_session(&row),
            move |this, loaded, cx| match loaded {
                Ok(Some(row)) => {
                    if !this.sessions.iter().any(|s| s.id == id) {
                        this.sessions.push(row);
                    }
                    this.show_reminder_thread(&id, due_at, cx);
                }
                Ok(None) => {
                    this.reminder_failure = Some("This conversation is no longer available.".into());
                    this.cancel_reminders(&[id], Some(due_at), cx);
                }
                Err(err) => {
                    this.reminder_failure = Some(err.to_string());
                    cx.notify();
                }
            },
        );
    }

    /// Opens a reminder's thread, now in `self.sessions`, and clears it.
    fn show_reminder_thread(&mut self, session_id: &str, due_at: i64, cx: &mut Context<Self>) {
        self.sidebar_mode = crate::app::SidebarMode::Sessions;
        self.open_session(session_id.to_string(), cx);
        self.cancel_reminders(&[session_id.to_string()], Some(due_at), cx);
    }
}

/// MonoCode's macOS alert, under BenCode's name: the thread as subtitle,
/// "Reminder: continue this conversation."
fn desktop_alert(session_title: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"Reminder: continue this conversation.\" with title \"BenCode\" subtitle \"{}\"",
        quote(session_title)
    );
    match std::process::Command::new("osascript").arg("-e").arg(script).output() {
        Ok(out) if !out.status.success() => {
            log::warn!("reminder alert failed: {}", String::from_utf8_lossy(&out.stderr))
        }
        Ok(_) => {}
        Err(err) => log::warn!("reminder alert failed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> i64 {
        jiff::civil::date(y, mo, d)
            .at(h, mi, 0, 0)
            .to_zoned(TimeZone::system())
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn presets_land_where_monocode_puts_them() {
        // Wednesday 2026-10-07 10:05 local.
        let now = local(2026, 10, 7, 10, 5);
        assert_eq!(reminder_time(Preset::OneHour, now), Some(now + HOUR_MS));
        assert_eq!(reminder_time(Preset::Evening, now), Some(local(2026, 10, 7, 18, 0)));
        assert_eq!(reminder_time(Preset::Tomorrow, now), Some(local(2026, 10, 8, 9, 0)));
        assert_eq!(reminder_time(Preset::NextWeek, now), Some(local(2026, 10, 12, 9, 0)));
        let late = local(2026, 10, 7, 19, 0);
        assert_eq!(reminder_time(Preset::Evening, late), None, "evening has passed");
        let monday = local(2026, 10, 12, 8, 0);
        assert_eq!(reminder_time(Preset::NextWeek, monday), Some(local(2026, 10, 19, 9, 0)));
    }

    #[test]
    fn labels_use_an_unpadded_24_hour_clock() {
        let now = local(2026, 10, 7, 8, 5);
        let rows = preset_rows(now);
        assert_eq!(rows[0].1, "In 1 hour (9:05)");
        assert_eq!(rows[1].1, "In 3 hours (11:05)");
        assert!(rows.iter().all(|(_, _, enabled)| *enabled));
        assert_eq!(format_reminder_time(local(2026, 10, 12, 9, 0)), "Mon, Oct 12, 9:00 AM");
    }
}
