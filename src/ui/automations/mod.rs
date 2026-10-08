//! MonoCode `AutomationsView.tsx`: the automations list beside either the
//! New automation page or the editor of one automation (Settings and Run
//! history). State and logic: `app/automations.rs`, `app/automation_runs.rs`.

mod editor;
mod format;
mod history;
mod list;
mod picker;
mod settings;
pub mod templates;

use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// MonoCode `<aside className="w-[280px]">`.
const LIST_WIDTH: f32 = 280.0;
/// MonoCode `max-w-5xl` with `px-8`: the column the pages are laid out in.
const PAGE_WIDTH: f32 = 1024.0;

impl BenCodeApp {
    pub(crate) fn render_automations_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let main = if self.automations.picker_open || self.automations.draft.is_none() {
            self.render_automation_picker(cx)
        } else {
            self.render_automation_editor(cx)
        };
        div()
            .flex()
            .size_full()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(LIST_WIDTH))
                    .h_full()
                    .border_r_1()
                    .border_color(colors.border)
                    .child(self.render_automation_list(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(self.render_automation_error(cx))
                    .child(main),
            )
            .children(self.render_automation_delete_confirm(cx))
            .into_any_element()
    }

    fn render_automation_delete_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let id = self.automations.pending_delete.clone()?;
        let name = self
            .automations
            .items
            .iter()
            .find(|a| a.id == id)
            .map_or("this automation", |a| a.name.as_str());
        let close = app_callback(cx, |this, cx| {
            this.automations.pending_delete = None;
            cx.notify();
        });
        let delete = app_callback(cx, move |this, cx| this.delete_automation(&id, cx));
        Some(
            ConfirmDialog::new(
                "automation-delete-confirm",
                "Delete automation?",
                format!("“{name}” and its run history will be removed."),
                close,
            )
            .confirm("Delete")
            .destructive()
            .on_confirm(delete),
        )
    }
}
