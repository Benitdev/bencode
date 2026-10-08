//! MonoCode's Run history page (`RunRow`, `RunStatusPill`): what started
//! each run, when, how it ended and how long it took. A row opens the
//! run's thread. Also the surface's error banner.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Palette};
use gpui::{AnyElement, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*};
use jiff::tz::TimeZone;

use super::format::{run_at, run_duration};
use crate::ui::page_parts::{panel, section_title, tint};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{AutomationRow, AutomationRunRow, RunStatus, RunTrigger};
use crate::schedule::schedule_label;
use crate::ui::scale::px;

/// MonoCode shows this many runs.
const RUNS_SHOWN: usize = 100;
// MonoCode `RUN_GRID`: 9.5rem, 6.75rem and 3.5rem after the trigger.
const TRIGGERED_WIDTH: f32 = 152.0;
const STATUS_WIDTH: f32 = 108.0;
const DURATION_WIDTH: f32 = 56.0;

/// MonoCode `runStatusLabel`.
fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "Pending",
        RunStatus::Running => "Running",
        RunStatus::Succeeded => "Succeeded",
        RunStatus::Failed => "Failed",
        RunStatus::Skipped => "Skipped",
        RunStatus::Cancelled => "Cancelled",
    }
}

/// MonoCode `runStatusTone`: the pill's text colour; its fill is the same
/// colour, tinted.
fn status_color(status: RunStatus, colors: &Palette) -> Hsla {
    match status {
        RunStatus::Succeeded => colors.success,
        RunStatus::Failed => colors.danger,
        RunStatus::Skipped => colors.warning,
        RunStatus::Cancelled => colors.fg_muted,
        RunStatus::Pending | RunStatus::Running => colors.accent,
    }
}

/// MonoCode `runTriggerMeta`.
fn trigger_label(run: &AutomationRunRow, auto: &AutomationRow) -> String {
    match run.trigger {
        RunTrigger::Manual => "Test run".to_string(),
        RunTrigger::Event => "Event".to_string(),
        RunTrigger::Scheduled => format!("Scheduled · {}", schedule_label(auto)),
    }
}

/// One line of the table: the trigger takes the room the fixed columns leave.
fn grid_row(trigger: impl IntoElement, triggered: impl IntoElement, status: impl IntoElement, duration: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .px_4()
        .child(div().flex_1().min_w_0().child(trigger))
        .child(div().flex_none().w(px(TRIGGERED_WIDTH)).truncate().child(triggered))
        .child(div().flex().flex_none().w(px(STATUS_WIDTH)).child(status))
        .child(div().flex().flex_none().justify_end().w(px(DURATION_WIDTH)).child(duration))
}

impl BenCodeApp {
    pub(super) fn render_automation_history(&self, draft: &AutomationRow, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let title = section_title("Run history", muted);
        let runs = &self.automations.runs;
        let table = if runs.is_empty() {
            panel(fg)
                .border_dashed()
                .flex()
                .justify_center()
                .px_4()
                .py_16()
                .text_size(px(12.0))
                .text_color(fg.opacity(tint::HINT))
                .child("This automation has not run yet.")
        } else {
            let (tz, now) = (TimeZone::system(), now_ms());
            let rows = runs
                .iter()
                .take(RUNS_SHOWN)
                .enumerate()
                .map(|(ix, run)| self.render_automation_run(ix, run, draft, &tz, now, cx));
            panel(fg)
                .overflow_hidden()
                .child(
                    grid_row("Trigger", "Triggered", "Status", "Duration")
                        .h(px(40.0))
                        .text_size(px(11.0))
                        .text_color(fg.opacity(tint::HINT)),
                )
                .children(rows)
        };
        div().flex().flex_col().gap_3().child(title).child(table).into_any_element()
    }

    fn render_automation_run(
        &self,
        ix: usize,
        run: &AutomationRunRow,
        draft: &AutomationRow,
        tz: &TimeZone,
        now: i64,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let tone = status_color(run.status, colors);
        let session = run.session_id.clone();
        let tip = match (&session, &run.error) {
            (Some(_), _) => "Open session".to_string(),
            (None, Some(error)) => error.clone(),
            (None, None) => "This run has no session yet".to_string(),
        };
        let trigger = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_color(fg)
            .child(Icon::new(IconName::Clock).size(IconSize::Sm).color(fg.opacity(tint::HINT)))
            .child(div().min_w_0().truncate().child(trigger_label(run, draft)));
        let pill = div()
            .flex()
            .items_center()
            .h(px(20.0))
            .px_2()
            .rounded_full()
            .bg(tone.opacity(tint::PILL))
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(tone)
            .child(status_label(run.status));
        let triggered = run_at(if run.scheduled_for > 0 { run.scheduled_for } else { run.created_at }, tz);
        div()
            .id(("automation-run", ix))
            .border_t_1()
            .border_color(fg.opacity(tint::RULE))
            .tooltip(Tooltip::text(tip))
            .when_some(session, |el, session| {
                el.cursor_pointer()
                    .hover(|style| style.bg(fg.opacity(tint::HOVER)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_automation_run(&session, cx)))
            })
            .child(
                grid_row(
                    trigger,
                    div().text_color(fg.opacity(tint::BODY)).child(triggered),
                    pill,
                    div().text_color(fg.opacity(tint::SOFT)).child(run_duration(run, now)),
                )
                .h(px(44.0))
                .text_size(px(13.0)),
            )
    }

    /// MonoCode's red banner over the page: the last thing that failed.
    pub(super) fn render_automation_error(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let error = self.automations.error.clone()?;
        let danger = cx.theme().colors.danger;
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .m_4()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(danger.opacity(tint::ALERT_STROKE))
                .bg(danger.opacity(tint::FILL))
                .text_size(px(12.0))
                .text_color(danger)
                .child(Icon::new(IconName::CircleAlert).size(IconSize::Sm).color(danger))
                .child(div().flex_1().min_w_0().child(error))
                .child(
                    IconButton::new("automation-error-dismiss", IconName::X)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.automations.error = None;
                            cx.notify();
                        })),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(trigger: RunTrigger) -> AutomationRunRow {
        AutomationRunRow {
            id: "r".into(),
            automation_id: "a".into(),
            trigger,
            scheduled_for: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
            status: RunStatus::Pending,
            session_id: None,
            error: None,
        }
    }

    #[test]
    fn runs_name_what_started_them() {
        let auto = AutomationRow {
            schedule_kind: "daily".into(),
            time: "09:00".into(),
            ..Default::default()
        };
        assert_eq!(trigger_label(&run(RunTrigger::Manual), &auto), "Test run");
        assert_eq!(
            trigger_label(&run(RunTrigger::Scheduled), &auto),
            "Scheduled · Daily at 09:00"
        );
        assert_eq!(trigger_label(&run(RunTrigger::Event), &auto), "Event");
    }
}
