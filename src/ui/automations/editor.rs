//! Right pane: the selected automation's fields, actions and run history.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tag};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::{FormField, Input, Switch};
use ely_gpui_component::layout::{ScrollArea, Section};
use ely_gpui_component::lists::ListItem;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::typography::Caption;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};
use jiff::Timestamp;

use super::{run_status_tone, schedule_label};
use crate::app::BenCodeApp;
use crate::db::{AutomationRow, AutomationRunRow};

fn run_time(millis: i64) -> String {
    Timestamp::from_millisecond(millis).map_or_else(
        |_| "unknown time".to_string(),
        |at| at.strftime("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

impl BenCodeApp {
    pub(super) fn render_automation_detail(&self, cx: &Context<Self>) -> AnyElement {
        let Some(auto) = self.selected_automation() else {
            return EmptyState::new("automation-none", IconName::Zap, "No automation selected")
                .body("Select one on the left, or start from a template.")
                .into_any_element();
        };
        ScrollArea::new("automation-detail")
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .pl_4()
                    .child(self.render_automation_actions(auto, cx))
                    .child(self.render_automation_fields(auto))
                    .child(self.render_automation_runs()),
            )
            .into_any_element()
    }

    fn render_automation_actions(
        &self,
        auto: &AutomationRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (toggle_id, delete_id) = (auto.id.clone(), auto.id.clone());
        let busy = self.is_agent_running();
        let toggle = cx.listener(move |this, on: &bool, _, cx| {
            this.set_automation_enabled(&toggle_id, *on, cx)
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                Switch::new("automation-enabled", auto.enabled)
                    .label("Enabled")
                    .on_change(move |on, window, cx| toggle(&on, window, cx)),
            )
            .child(div().flex_1())
            .child(
                Button::new(
                    "automation-run",
                    if busy { "Agent busy" } else { "Run now" },
                )
                .primary()
                .icon(IconName::Play)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.run_selected_automation_now(cx))),
            )
            .child(
                Button::new("automation-save", "Save")
                    .variant(ButtonVariant::Secondary)
                    .icon(IconName::Save)
                    .on_click(cx.listener(|this, _, _, cx| this.save_selected_automation(cx))),
            )
            .child(
                IconButton::new("automation-delete", IconName::Trash2)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Delete automation")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.automation_pending_delete = Some(delete_id.clone());
                        cx.notify();
                    })),
            )
    }

    fn render_automation_fields(&self, auto: &AutomationRow) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                FormField::new("automation-name", "Name")
                    .child(Input::new(&self.automation_name_input)),
            )
            .child({
                let field = FormField::new("automation-time", "Time (HH:MM, 24-hour)")
                    .description(schedule_label(auto));
                match self.automation_time_error.clone() {
                    Some(error) => field.error(error),
                    None => field,
                }
                .child(Input::new(&self.automation_time_input))
            })
            .child(
                FormField::new("automation-workspace", "Workspace").child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Caption::new(auto.cwd.clone()))
                        .child(
                            Tag::new("automation-harness", auto.harness.clone())
                                .icon(IconName::Terminal),
                        ),
                ),
            )
            .child(
                FormField::new("automation-prompt", "Prompt")
                    .child(Input::new(&self.automation_prompt_input)),
            )
    }

    fn render_automation_runs(&self) -> impl IntoElement {
        let runs: AnyElement = if self.automation_runs.is_empty() {
            Caption::new("No runs recorded yet.").into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(
                    self.automation_runs
                        .iter()
                        .enumerate()
                        .map(|(ix, run)| render_run(ix, run)),
                )
                .into_any_element()
        };
        Section::new("Recent runs")
            .description(format!("{} recorded", self.automation_runs.len()))
            .child(runs)
    }
}

fn render_run(ix: usize, run: &AutomationRunRow) -> ListItem {
    let detail = run
        .error
        .clone()
        .unwrap_or_else(|| run_time(run.started_at.unwrap_or(run.created_at)));
    ListItem::new(("automation-run", ix), format!("Trigger: {}", run.trigger))
        .description(detail)
        .trailing(Badge::new(run.status.clone()).tone(run_status_tone(&run.status)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_time_formats_utc() {
        assert_eq!(run_time(0), "1970-01-01 00:00 UTC");
    }
}
