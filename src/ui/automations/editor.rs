//! MonoCode `AutomationEditor`'s frame: the name, Save and Run now, the
//! Active switch with a summary of the project, model and next run,
//! Delete, and the Settings / Run history tabs.
//! The Settings page itself is in `settings.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::{Choice, Switch};
use ely_gpui_component::menus::{Menu, MenuItem, OverflowMenu};
use ely_gpui_component::navigation::Tabs;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*,
};
use jiff::tz::TimeZone;

use super::PAGE_WIDTH;
use super::format::next_run_preview;
use crate::app::automations::{EditorTab, draft_is_valid};
use crate::app::{BenCodeApp, now_ms};
use crate::db::AutomationRow;
use crate::harness::catalog;
use crate::schedule;
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::mascot::pixel_sprite;
use crate::ui::page_parts::tint;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;
use crate::ui::tokens::Text;

/// The automation's mark beside its name, as the New automation cards'.
const ICON_TILE: f32 = 40.0;

fn divider(fg: Hsla) -> Div {
    div()
        .flex_none()
        .w(px(1.0))
        .h(px(12.0))
        .bg(fg.opacity(tint::DIVIDER))
}

impl BenCodeApp {
    pub(super) fn render_automation_editor(&self, cx: &Context<Self>) -> AnyElement {
        let Some(draft) = self.editor_draft(cx) else {
            return div().into_any_element();
        };
        let stored = self.edited_automation().is_some();
        let history = stored && self.automations.tab == EditorTab::History;
        let page = if history {
            self.render_automation_history(&draft, cx)
        } else {
            self.render_automation_settings(&draft, cx)
        };
        let body = div()
            .id("automation-editor")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .px_8()
                    .pt_5()
                    .pb_10()
                    .child(page),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_automation_header(&draft, stored, history, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Scrolled::new("automation-editor-scrollbar", body)),
            )
            .into_any_element()
    }

    fn render_automation_header(
        &self,
        draft: &AutomationRow,
        stored: bool,
        history: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let dirty = self.automation_dirty(cx);
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .when(dirty, |el| {
                el.child(
                    Button::new(
                        "automation-discard",
                        if stored { "Reset" } else { "Cancel" },
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| this.discard_automation_edits(cx))),
                )
            })
            .when(stored, |el| {
                el.child(
                    Button::new("automation-run", "Run now")
                        .variant(ButtonVariant::Outline)
                        .size(ControlSize::Sm)
                        .icon(IconName::Play)
                        .on_click(cx.listener(|this, _, _, cx| this.run_automation_now(cx))),
                )
            })
            .child(
                Button::new("automation-save", if stored { "Save" } else { "Create" })
                    .primary()
                    .size(ControlSize::Sm)
                    .disabled(!draft_is_valid(draft) || !dirty)
                    .loading(self.automations.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.save_automation_draft(cx))),
            );
        let enable = cx.listener(|this, on: &bool, _, cx| {
            let on = *on;
            this.edit_automation(cx, |draft| draft.enabled = on);
        });
        let summary = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_1()
            .min_w_0()
            .text_size(Text::Base.px())
            .text_color(muted)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(
                        Switch::new("automation-enabled", draft.enabled)
                            .on_change(move |on, window, cx| enable(&on, window, cx)),
                    )
                    .child(if draft.enabled { "Active" } else { "Paused" }),
            )
            .child(divider(fg))
            .child(self.render_automation_summary(draft, cx));
        let actions = actions.when(stored, |el| {
            let id = draft.id.clone();
            let delete = app_callback(cx, move |this, cx| {
                this.automations.pending_delete = Some(id.clone());
                cx.notify();
            });
            let menu = Menu::new().item(
                MenuItem::new("Delete automation")
                    .icon(IconName::Trash2)
                    .on_click(delete),
            );
            el.child(OverflowMenu::new("automation-actions", menu).tooltip("Automation actions"))
        });
        let tabs = stored.then(|| {
            Tabs::new(
                "automation-tabs",
                [
                    Choice::new("settings", "Settings"),
                    Choice::new("history", "Run history"),
                ],
                if history { "history" } else { "settings" },
            )
            .on_change(on_value(cx, |this, tab, cx| {
                let tab = if tab == "history" {
                    EditorTab::History
                } else {
                    EditorTab::Settings
                };
                this.show_automation_tab(tab, cx)
            }))
        });
        div()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .mx_auto()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .px_8()
                    .pt_5()
                    .when(!stored, |el| el.pb_5())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(px(ICON_TILE))
                                    .rounded(px(10.0))
                                    .bg(fg.opacity(tint::FILL))
                                    .child(
                                        Icon::new(IconName::Zap)
                                            .size(IconSize::Md)
                                            .color(fg.opacity(tint::BODY)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.0))
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(Text::Title.px())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg)
                                            .child(self.automations.name_input.clone()),
                                    )
                                    .child(summary),
                            )
                            .child(actions),
                    )
                    .children(tabs),
            )
    }

    /// What the automation is set to, read at a glance: its project, its
    /// model and when it runs next.
    fn render_automation_summary(&self, draft: &AutomationRow, cx: &Context<Self>) -> Div {
        let fg = cx.theme().colors.fg;
        let item = |icon: AnyElement, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .child(icon)
                .child(div().min_w_0().truncate().child(text))
        };
        let icon = |name: IconName| {
            Icon::new(name)
                .size(IconSize::Xs)
                .color(fg.opacity(tint::QUIET))
                .into_any_element()
        };
        let project = if draft.cwd.is_empty() {
            item(icon(IconName::Folder), "No project".to_string())
        } else {
            item(
                pixel_sprite(
                    &self.project_mascot(&draft.cwd).rest,
                    px(12.0),
                    self.project_color(&draft.cwd),
                    false,
                )
                .into_any_element(),
                self.rail_project_label(&draft.cwd),
            )
        };
        let model = if draft.model.is_empty() {
            item(icon(IconName::Bot), "No model".to_string())
        } else {
            item(
                HarnessIcon::new(draft.harness.clone())
                    .size(px(14.0))
                    .into_any_element(),
                catalog::display_label(&draft.harness, &draft.model),
            )
        };
        let tz = TimeZone::system();
        let next = if !draft.enabled {
            "No runs while paused".to_string()
        } else if schedule::time_triggers(draft).is_empty() {
            "No schedule".to_string()
        } else {
            schedule::next_automation_run_at(draft, now_ms(), &tz)
                .map_or_else(|| "No schedule".to_string(), |at| next_run_preview(at, &tz))
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_1()
            .min_w_0()
            .child(project)
            .child(model)
            .child(item(icon(IconName::Clock), next))
    }
}
