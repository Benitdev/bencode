//! MonoCode `AutomationsView.tsx`: the automations list beside the
//! overview (BenCode's landing), the New automation page, or the editor of
//! one automation (Settings and Run history). State and logic:
//! `app/automations.rs`, `app/automation_runs.rs`,
//! `app/automation_overview.rs`.

mod editor;
mod format;
mod history;
mod list;
mod overview;
mod picker;
mod settings;
pub mod templates;

use bencode_ui::overlays::ConfirmDialog;
use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::app::automation_drafts::{Discard, NEW_DRAFT};
use crate::app::automations::AutomationPage;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// MonoCode `<aside className="w-[280px]">`.
const LIST_WIDTH: f32 = 280.0;
/// MonoCode `max-w-5xl` with `px-8`: the column the pages are laid out in.
const PAGE_WIDTH: f32 = 1024.0;

impl BenCodeApp {
    pub(crate) fn render_automations_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let main = match self.automations.page {
            AutomationPage::Overview => self.render_automation_overview(cx),
            AutomationPage::Editor if self.automations.draft.is_some() => {
                self.render_automation_editor(cx)
            }
            AutomationPage::New | AutomationPage::Editor => self.render_automation_picker(cx),
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
            .children(self.render_automation_discard_confirm(cx))
            .into_any_element()
    }

    /// "Discard changes?" before Reset, Cancel or Discard drop edits, or a
    /// new automation replaces an unsaved one.
    fn render_automation_discard_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let discard = self.automations.pending_discard.as_ref()?;
        let draft_name = |draft: Option<&crate::db::AutomationRow>| {
            draft
                .map(|d| d.name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Untitled".to_string())
        };
        // As typed while the editor shows it, else as stashed.
        let editing_new = self.editor_draft(cx).filter(|d| d.id == NEW_DRAFT);
        let new = editing_new
            .as_ref()
            .or(self.automations.stashed.get(NEW_DRAFT));
        let (title, message, confirm) = match discard {
            Discard::Editor if self.edited_automation().is_some() => (
                "Discard changes?",
                format!(
                    "Your edits to “{}” will be lost.",
                    self.automations.name_input.read(cx).text().trim()
                ),
                "Discard",
            ),
            Discard::Editor | Discard::StashedNew => (
                "Discard this automation?",
                format!(
                    "“{}” has not been created; its settings will be lost.",
                    draft_name(new)
                ),
                "Discard",
            ),
            Discard::ReplaceNew(_) => (
                "Start another automation?",
                format!(
                    "“{}” has not been created; its settings will be lost.",
                    draft_name(new)
                ),
                "Discard and start",
            ),
        };
        let close = app_callback(cx, |this, cx| {
            this.automations.pending_discard = None;
            cx.notify();
        });
        let confirm_discard = app_callback(cx, |this, cx| this.confirm_discard(cx));
        Some(
            ConfirmDialog::new("automation-discard-confirm", title, message, close)
                .confirm(confirm)
                .destructive()
                .on_confirm(confirm_discard),
        )
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
