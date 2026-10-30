//! MonoCode `AutomationEditor`'s frame: the name, Save and Run now, the
//! Active switch with a summary of the project, model and next run,
//! Delete, and the Settings / Run history tabs.
//! The Settings page itself is in `settings.rs`.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::forms::{Choice, Switch};
use bencode_ui::menus::{Menu, MenuItem, OverflowMenu};
use bencode_ui::navigation::Tabs;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*,
};
use jiff::tz::TimeZone;

use super::PAGE_WIDTH;
use super::format::{in_text, when_text};
use crate::app::automation_pauses::active_skip;
use crate::app::automations::{EditorTab, draft_is_valid};
use crate::app::{BenCodeApp, now_ms};
use crate::db::AutomationRow;
use crate::harness::catalog;
use crate::schedule;
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::mascot::pixel_sprite;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

/// The automation's mark beside its name, as the New automation cards'.
const ICON_TILE: f32 = 40.0;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The quiet dot between a header line's parts.
fn separator(fg: Hsla) -> Div {
    div().flex_none().text_color(fg.ink(Ink::Faint)).child("·")
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
        let valid = draft_is_valid(draft);
        let enable = cx.listener(|this, on: &bool, _, cx| this.set_editor_enabled(*on, cx));
        // One line each, never wrapped: the switch with the project and
        // model, then when it runs.
        let summary = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .min_w_0()
            .text_size(Text::Base.px())
            .text_color(muted)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
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
                    .child(separator(fg))
                    .children(self.render_automation_summary(draft, cx)),
            )
            .child(self.render_next_run(draft, cx));
        let actions = if stored {
            self.render_stored_actions(draft, dirty, valid, cx)
        } else {
            self.render_new_actions(valid, cx)
        };
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
                                    .bg(fg.fill(Fill::Selected))
                                    .child(
                                        Icon::new(IconName::Zap)
                                            .size(IconSize::Md)
                                            .color(fg.ink(Ink::Body)),
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

    /// A stored automation's actions: "Unsaved changes", Reset and Save
    /// while there are edits (their room kept while there are none, so
    /// nothing moves), Run now or Test changes, and the menu.
    fn render_stored_actions(
        &self,
        draft: &AutomationRow,
        dirty: bool,
        valid: bool,
        cx: &Context<Self>,
    ) -> Div {
        let fg = cx.theme().colors.fg;
        let edits = div()
            .flex()
            .items_center()
            .gap_2()
            .when(!dirty, |el| el.invisible())
            .child(
                div()
                    .id("automation-unsaved")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(unsaved_dot(cx))
                    .child("Unsaved changes"),
            )
            .child(
                Button::new("automation-discard", "Reset")
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .disabled(!dirty)
                    .when(dirty, |button| {
                        button.on_click(
                            cx.listener(|this, _, _, cx| this.discard_automation_edits(cx)),
                        )
                    }),
            )
            .child(
                Button::new("automation-save", "Save")
                    .variant(if dirty && valid {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ControlSize::Sm)
                    .disabled(!dirty || !valid)
                    .loading(self.automations.saving)
                    .when(dirty && valid, |button| {
                        button
                            .on_click(cx.listener(|this, _, _, cx| this.save_automation_draft(cx)))
                    }),
            );
        // With edits, Run now tries them unsaved.
        let test = dirty && valid;
        let run = div()
            .id("automation-run-tip")
            .tooltip(Tooltip::text(if test {
                "Runs what is on screen without saving it"
            } else {
                "Runs the saved automation now"
            }))
            .child(
                Button::new(
                    "automation-run",
                    if test { "Test changes" } else { "Run now" },
                )
                .variant(ButtonVariant::Outline)
                .size(ControlSize::Sm)
                .icon(IconName::Play)
                .on_click(cx.listener(|this, _, _, cx| this.run_automation_now(cx))),
            );
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(edits)
            .child(run)
            .child(self.render_automation_menu(draft, cx))
    }

    /// A new automation's actions: Cancel and Create.
    fn render_new_actions(&self, valid: bool, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(
                Button::new("automation-discard", "Cancel")
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| this.discard_automation_edits(cx))),
            )
            .child(
                Button::new("automation-save", "Create")
                    .variant(if valid {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ControlSize::Sm)
                    .disabled(!valid)
                    .loading(self.automations.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.save_automation_draft(cx))),
            )
    }

    /// Skip next run, Duplicate and Delete.
    fn render_automation_menu(&self, draft: &AutomationRow, cx: &Context<Self>) -> OverflowMenu {
        let id = draft.id.clone();
        let stored = self.edited_automation();
        let now = now_ms();
        let skipped = stored.and_then(|auto| active_skip(auto, now)).is_some();
        let skippable = stored.is_some_and(|auto| {
            auto.enabled && auto.next_run_at > now && !schedule::time_triggers(auto).is_empty()
        });
        let skip = {
            let id = id.clone();
            app_callback(cx, move |this, cx| {
                this.skip_automation_run(&id, skipped, cx)
            })
        };
        let delete = app_callback(cx, move |this, cx| {
            this.automations.pending_delete = Some(id.clone());
            cx.notify();
        });
        let menu = Menu::new()
            .item(
                MenuItem::new(if skipped {
                    "Undo skip"
                } else {
                    "Skip next run"
                })
                .icon(if skipped {
                    IconName::Undo2
                } else {
                    IconName::Ban
                })
                .disabled(!skipped && !skippable)
                .on_click(skip),
            )
            .item(
                MenuItem::new("Duplicate")
                    .icon(IconName::Copy)
                    .on_click(app_callback(cx, |this, cx| this.duplicate_automation(cx))),
            )
            .separator()
            .item(
                MenuItem::new("Delete automation")
                    .icon(IconName::Trash2)
                    .on_click(delete),
            );
        OverflowMenu::new("automation-actions", menu).tooltip("Automation actions")
    }

    /// What the automation is set to, read at a glance: its project, then
    /// its model, a dot between them.
    fn render_automation_summary(&self, draft: &AutomationRow, cx: &Context<Self>) -> [Div; 3] {
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
                .color(fg.ink(Ink::Muted))
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
            .flex_none()
        };
        [project, separator(fg), model]
    }

    /// When it runs next, and what holds it back: the automation paused,
    /// Pause all, or a skipped run with its Undo. Times read as on the
    /// cards ("tomorrow 08:00"); the trigger rows give the full date.
    fn render_next_run(&self, draft: &AutomationRow, cx: &Context<Self>) -> Div {
        let fg = cx.theme().colors.fg;
        let (tz, now) = (TimeZone::system(), now_ms());
        let amber = Status::Attention.color(fg);
        let line = div().flex().items_center().gap_2().min_w_0().child(
            Icon::new(IconName::Clock)
                .size(IconSize::Xs)
                .color(fg.ink(Ink::Muted)),
        );
        let text = |text: String| div().min_w_0().truncate().child(text);
        let next = |at: Option<i64>| {
            at.map_or_else(
                || "No schedule".to_string(),
                |at| {
                    let when = when_text(at, now, &tz);
                    // "in 3h" only while it says more than the day does.
                    if at - now < DAY_MS {
                        format!("Next run {when} · {}", in_text(at, now))
                    } else {
                        format!("Next run {when}")
                    }
                },
            )
        };
        if !draft.enabled {
            return line.child(text("No runs while paused".to_string()));
        }
        if schedule::time_triggers(draft).is_empty() {
            return line.child(text("No schedule".to_string()));
        }
        let skipped = self
            .edited_automation()
            .and_then(|auto| active_skip(auto, now));
        if let Some(until) = self.automations_paused_until() {
            // What falls in the pause is skipped; a run at its end goes
            // ahead, unless it is the one the user skipped.
            let at = schedule::next_run_skipping(draft, skipped, until - 1, &tz).map(|(at, _)| at);
            return line.child(text(next(at))).child(separator(fg)).child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .text_color(amber)
                    .child(Icon::new(IconName::Pause).size(IconSize::Xs).color(amber))
                    .child(format!("All paused until {}", when_text(until, now, &tz))),
            );
        }
        let Some((stored, skipped)) = self.edited_automation().zip(skipped) else {
            return line.child(text(next(schedule::next_automation_run_at(
                draft, now, &tz,
            ))));
        };
        let id = stored.id.clone();
        line.child(text(next(Some(stored.next_run_at))))
            .child(separator(fg))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .text_color(amber)
                    .child(Icon::new(IconName::Ban).size(IconSize::Xs).color(amber))
                    .child(format!("{} skipped", when_text(skipped, now, &tz))),
            )
            .child(
                Button::new("automation-skip-undo", "Undo")
                    .variant(ButtonVariant::Link)
                    .size(ControlSize::Sm)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.skip_automation_run(&id, true, cx)),
                    ),
            )
    }
}

/// The small mark of unsaved edits, on a list card and in the header.
pub(super) fn unsaved_dot(cx: &gpui::App) -> Div {
    div()
        .flex_none()
        .size(px(6.0))
        .rounded_full()
        .bg(cx.theme().colors.accent)
}
