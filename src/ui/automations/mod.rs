//! Automations: scheduled prompt runners stored in MonoCode's automations table.

mod editor;
mod list;
mod scheduler;
mod templates;

use ely_gpui_component::data_display::Tone;
use ely_gpui_component::layout::MasterDetail;
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use jiff::tz::TimeZone;

use crate::app::{BenCodeApp, Surface, ViewMode, now_ms};
use crate::db::{AutomationRow, AutomationRunRow};
use crate::schedule;
use crate::ui::app_callback::app_callback;
use templates::{AutomationTemplate, BLANK_AUTOMATION};

/// MonoCode `harness:model` key used for newly created automations.
const DEFAULT_AUTOMATION_MODEL: &str = "claude:sonnet";
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A prompt to run in a fresh thread.
pub struct ThreadRequest {
    pub title: String,
    pub prompt: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub pinned: bool,
}

/// MonoCode's schedule wording, e.g. "Weekdays at 09:00" or "Hourly at :05".
fn schedule_label(auto: &AutomationRow) -> String {
    let time = &auto.time;
    match auto.schedule_kind.as_str() {
        "hourly" => format!("Hourly at :{:02}", auto.minute),
        "daily" => format!("Daily at {time}"),
        "weekdays" => format!("Weekdays at {time}"),
        _ => {
            let day = usize::try_from(auto.day_of_week)
                .ok()
                .and_then(|d| WEEKDAYS.get(d));
            format!("{} at {time}", day.unwrap_or(&"Weekly"))
        }
    }
}

/// Tone for a MonoCode run status (pending, running, succeeded, failed, skipped, cancelled).
fn run_status_tone(status: &str) -> Tone {
    match status {
        "succeeded" => Tone::Success,
        "failed" => Tone::Danger,
        "pending" | "running" => Tone::Info,
        _ => Tone::Neutral,
    }
}

fn new_automation(draft: &AutomationTemplate, cwd: String, now: i64) -> AutomationRow {
    let delay = if draft.schedule == "hourly" {
        HOUR_MS
    } else {
        DAY_MS
    };
    let next_run_at =
        schedule::next_run_at(draft.schedule, 0, draft.time, 1, now, &TimeZone::system())
            .unwrap_or(now + delay);
    AutomationRow {
        id: format!("auto-{now}"),
        name: draft.name.to_string(),
        prompt: draft.prompt.to_string(),
        harness: "claude".to_string(),
        model: DEFAULT_AUTOMATION_MODEL.to_string(),
        cwd,
        schedule_kind: draft.schedule.to_string(),
        time: draft.time.to_string(),
        day_of_week: 1,
        enabled: true,
        next_run_at,
        created_at: now,
        updated_at: now,
        ..Default::default()
    }
}

/// `fallback` when the field was cleared.
fn non_empty(text: &str, fallback: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        fallback.to_string()
    } else {
        text.to_string()
    }
}

impl BenCodeApp {
    pub fn open_automations(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Automations, cx);
        self.refresh_automations(cx);
        cx.notify();
    }

    fn close_automations(&mut self, cx: &mut Context<Self>) {
        if self.surface_open(Surface::Automations) {
            self.close_surface(cx);
        }
    }

    fn refresh_automations(&mut self, cx: &mut Context<Self>) {
        match self.db.list_automations() {
            Ok(automations) => self.automations = automations,
            Err(err) => log::error!("list_automations failed: {err:#}"),
        }
        let selected_exists = self.selected_automation().is_some();
        match self.automations.first().map(|a| a.id.clone()) {
            Some(first) if !selected_exists => self.select_automation(&first, cx),
            Some(_) => {}
            None => self.selected_automation_id = None,
        }
    }

    fn selected_automation(&self) -> Option<&AutomationRow> {
        let id = self.selected_automation_id.as_deref()?;
        self.automations.iter().find(|a| a.id == id)
    }

    fn select_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected_automation_id = Some(id.to_string());
        self.automation_time_error = None;
        if let Some(auto) = self.selected_automation() {
            let (name, prompt, time) = (auto.name.clone(), auto.prompt.clone(), auto.time.clone());
            self.automation_name_input
                .update(cx, |input, cx| input.set_text(name, cx));
            self.automation_prompt_input
                .update(cx, |input, cx| input.set_text(prompt, cx));
            self.automation_time_input
                .update(cx, |input, cx| input.set_text(time, cx));
        }
        self.load_automation_runs(id);
        cx.notify();
    }

    fn load_automation_runs(&mut self, id: &str) {
        match self.db.list_automation_runs(id) {
            Ok(runs) => self.automation_runs = runs,
            Err(err) => {
                log::error!("list_automation_runs failed: {err:#}");
                self.automation_runs.clear();
            }
        }
    }

    fn insert_automation(&mut self, draft: &AutomationTemplate, cx: &mut Context<Self>) {
        let cwd = self
            .selected_session()
            .map_or_else(|| self.current_cwd.clone(), |s| s.cwd.clone());
        let auto = new_automation(draft, cwd, now_ms());
        if let Err(err) = self.db.save_automation(&auto) {
            log::error!("save_automation failed: {err:#}");
            return;
        }
        self.refresh_automations(cx);
        self.select_automation(&auto.id, cx);
    }

    fn create_new_automation(&mut self, cx: &mut Context<Self>) {
        self.insert_automation(&BLANK_AUTOMATION, cx);
    }

    fn save_selected_automation(&mut self, cx: &mut Context<Self>) {
        let Some(auto) = self.selected_automation() else {
            return;
        };
        let time = non_empty(self.automation_time_input.read(cx).text(), &auto.time);
        let now = now_ms();
        let Some(next_run_at) = schedule::next_run_at(
            &auto.schedule_kind,
            auto.minute,
            &time,
            auto.day_of_week,
            now,
            &TimeZone::system(),
        ) else {
            self.automation_time_error = Some(format!(
                "“{time}” is not a valid 24-hour time such as 09:00."
            ));
            cx.notify();
            return;
        };
        let updated = AutomationRow {
            name: non_empty(self.automation_name_input.read(cx).text(), &auto.name),
            prompt: self.automation_prompt_input.read(cx).text().to_string(),
            time,
            next_run_at,
            updated_at: now,
            ..auto.clone()
        };
        self.automation_time_error = None;
        if let Err(err) = self.db.save_automation(&updated) {
            log::error!("save_automation failed: {err:#}");
        }
        self.refresh_automations(cx);
    }

    fn delete_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(err) = self.db.delete_automation(id) {
            log::error!("delete_automation failed: {err:#}");
            return;
        }
        if self.selected_automation_id.as_deref() == Some(id) {
            self.selected_automation_id = None;
            self.automation_runs.clear();
        }
        self.refresh_automations(cx);
        cx.notify();
    }

    fn set_automation_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        if let Err(err) = self.db.toggle_automation(id, enabled) {
            log::error!("toggle_automation failed: {err:#}");
        }
        self.refresh_automations(cx);
        cx.notify();
    }

    /// Runs the automation's prompt in a new thread and records a manual run linked to it.
    fn run_selected_automation_now(&mut self, cx: &mut Context<Self>) {
        let Some(auto) = self.selected_automation().cloned() else {
            return;
        };
        let request = ThreadRequest {
            title: auto.name.clone(),
            prompt: auto.prompt.clone(),
            cwd: Some(auto.cwd.clone()),
            model: Some(auto.model.clone()),
            pinned: false,
        };
        let Some(session_id) = self.run_in_new_thread(request, cx) else {
            return;
        };
        let now = now_ms();
        let run = AutomationRunRow {
            id: format!("run-{now}"),
            automation_id: auto.id.clone(),
            trigger: "manual".to_string(),
            scheduled_for: now,
            created_at: now,
            started_at: Some(now),
            completed_at: None,
            status: "running".to_string(),
            session_id: Some(session_id.clone()),
            error: None,
        };
        match self.db.create_automation_run(&run) {
            Ok(()) => self.attach_automation_run(&session_id, run.id),
            Err(err) => log::error!("create_automation_run failed: {err:#}"),
        }
        self.close_automations(cx);
    }

    /// Opens a new thread and starts `request.prompt` in it, alongside any
    /// other running threads. `None` if the thread could not be created.
    pub fn run_in_new_thread(
        &mut self,
        request: ThreadRequest,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        self.create_new_session(cx);
        if let Some(model) = &request.model {
            self.set_session_model(model, cx);
        }
        let session = self.selected_session_mut()?;
        session.title = request.title;
        session.pinned = request.pinned;
        if let Some(cwd) = request.cwd {
            session.cwd = cwd;
        }
        let id = session.id.clone();
        self.selected_diff_path = None;
        self.refresh_workspace_if_moved(cx);
        self.active_view_mode = ViewMode::Chat;
        self.send_prompt(&id, &request.prompt, cx);
        Some(id)
    }

    pub(crate) fn render_automations_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let min = theme.pane_min().to_pixels(theme.base_rem());
        div()
            .size_full()
            .child(MasterDetail::new(
                "automations-split",
                self.render_automation_master(cx),
                self.render_automation_detail(cx),
                min,
            ))
            .children(self.render_automation_delete_confirm(cx))
            .into_any_element()
    }

    fn render_automation_delete_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let id = self.automation_pending_delete.clone()?;
        let name = self
            .automations
            .iter()
            .find(|a| a.id == id)
            .map_or("this automation", |a| a.name.as_str());
        let close = app_callback(cx, |this, cx| {
            this.automation_pending_delete = None;
            cx.notify();
        });
        let delete = app_callback(cx, move |this, cx| this.delete_automation(&id, cx));
        Some(
            ConfirmDialog::new(
                "automation-delete-confirm",
                "Delete automation?",
                format!("“{name}” and its schedule will be removed."),
                close,
            )
            .confirm("Delete")
            .destructive()
            .on_confirm(delete),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, time: &str, minute: i64, day: i64) -> AutomationRow {
        AutomationRow {
            schedule_kind: kind.into(),
            time: time.into(),
            minute,
            day_of_week: day,
            ..Default::default()
        }
    }

    #[test]
    fn schedule_labels_follow_monocode() {
        assert_eq!(schedule_label(&row("hourly", "", 5, 0)), "Hourly at :05");
        assert_eq!(
            schedule_label(&row("daily", "09:00", 0, 0)),
            "Daily at 09:00"
        );
        assert_eq!(
            schedule_label(&row("weekdays", "08:30", 0, 0)),
            "Weekdays at 08:30"
        );
        assert_eq!(
            schedule_label(&row("weekly", "10:00", 0, 1)),
            "Monday at 10:00"
        );
        assert_eq!(
            schedule_label(&row("weekly", "10:00", 0, 9)),
            "Weekly at 10:00"
        );
    }

    #[test]
    fn run_status_tones() {
        assert_eq!(run_status_tone("succeeded"), Tone::Success);
        assert_eq!(run_status_tone("failed"), Tone::Danger);
        assert_eq!(run_status_tone("running"), Tone::Info);
        assert_eq!(run_status_tone("cancelled"), Tone::Neutral);
    }

    #[test]
    fn new_automation_uses_draft_and_next_scheduled_run() {
        let hourly = &templates::BUILTIN_TEMPLATES[4];
        let auto = new_automation(hourly, "/repo".into(), 1_000);
        assert_eq!(auto.id, "auto-1000");
        assert_eq!(auto.schedule_kind, "hourly");
        // Hourly at :00 → the top of the next hour.
        assert_eq!(auto.next_run_at, HOUR_MS);
        let daily = new_automation(&BLANK_AUTOMATION, "/r".into(), 0).next_run_at;
        assert!(
            daily > 0 && daily <= DAY_MS,
            "next daily run within a day: {daily}"
        );
        assert!(auto.enabled && auto.cwd == "/repo");
    }

    #[test]
    fn non_empty_falls_back_on_blank() {
        assert_eq!(non_empty("  ", "keep"), "keep");
        assert_eq!(non_empty(" new ", "keep"), "new");
    }
}
