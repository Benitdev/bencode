//! MonoCode `AutomationEditor`'s frame: the name, Save and Run now, the
//! Active switch, project and Delete, and the Settings / Run history tabs.
//! The Settings page itself is in `settings.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::Switch;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem, OverflowMenu};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Styled, div,
    prelude::*,
};

use super::PAGE_WIDTH;
use crate::ui::page_parts::{page_tab, tint};
use crate::app::automations::{EditorTab, draft_is_valid};
use crate::app::BenCodeApp;
use crate::db::AutomationRow;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;

fn divider(fg: Hsla) -> Div {
    div().flex_none().w(px(1.0)).h(px(12.0)).bg(fg.opacity(tint::DIVIDER))
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
                    Button::new("automation-discard", if stored { "Reset" } else { "Cancel" })
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
        let status = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_size(px(12.0))
            .text_color(muted)
            .child(
                Switch::new("automation-enabled", draft.enabled)
                    .on_change(move |on, window, cx| enable(&on, window, cx)),
            )
            .child(div().flex_none().child(if draft.enabled { "Active" } else { "Inactive" }))
            .child(divider(fg).ml_2())
            .child(self.render_automation_project(draft, cx))
            .when(stored, |el| {
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
                el.child(divider(fg)).child(
                    OverflowMenu::new("automation-actions", menu).tooltip("Automation actions"),
                )
            });
        let tabs = stored.then(|| {
            div()
                .flex()
                .gap_4()
                .child(
                    page_tab("automation-tab-settings", "Settings", !history, fg, muted).on_click(
                        cx.listener(|this, _, _, cx| this.show_automation_tab(EditorTab::Settings, cx)),
                    ),
                )
                .child(
                    page_tab("automation-tab-history", "Run history", history, fg, muted).on_click(
                        cx.listener(|this, _, _, cx| this.show_automation_tab(EditorTab::History, cx)),
                    ),
                )
        });
        div().flex_none().border_b_1().border_color(colors.border).child(
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
                        .items_center()
                        .gap_6()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(20.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child(self.automations.name_input.clone()),
                        )
                        .child(actions),
                )
                .child(status)
                .children(tabs),
        )
    }

    /// MonoCode `SearchableProjectPicker`: the project runs happen in.
    fn render_automation_project(&self, draft: &AutomationRow, cx: &Context<Self>) -> impl IntoElement {
        let mut projects = self.worktree_project_choices();
        if !draft.cwd.is_empty() && !projects.contains(&draft.cwd) {
            projects.insert(0, draft.cwd.clone());
        }
        let menu = projects.into_iter().fold(Menu::new(), |menu, path| {
            let pick = app_callback(cx, {
                let path = path.clone();
                move |this, cx| {
                    let path = path.clone();
                    // Folders belong to a project.
                    this.edit_automation(cx, |draft| {
                        draft.cwd = path;
                        draft.session_folder_id = Some(String::new());
                    });
                }
            });
            menu.item(
                MenuItem::radio(self.rail_project_label(&path), path == draft.cwd).on_click(pick),
            )
        });
        let label = if draft.cwd.is_empty() {
            "Choose a project".to_string()
        } else {
            self.rail_project_label(&draft.cwd)
        };
        DropdownMenu::new("automation-project", label, menu)
            .icon(IconName::Folder)
            .variant(ButtonVariant::Ghost)
    }
}
