//! Editor pane rendering: Ely tabs, a toolbar, the active editor, notices and the discard dialog.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::Badge;
use ely_gpui_component::feedback::{Alert, EmptyState};
use ely_gpui_component::navigation::{EditorTab, EditorTabs};
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Severity};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, TextSize};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*,
};

use super::files::{detect_language, file_name};
use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

impl BenCodeApp {
    /// Renders the code editor pane.
    pub fn render_editor_pane(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cx: &Context<Self> = cx;
        let colors = &cx.theme().colors;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(colors.bg)
            .when(!self.editor.files.is_empty(), |el| {
                el.child(self.render_editor_tabs(cx))
            })
            .child(self.render_editor_toolbar(cx))
            .children(self.render_editor_notice(cx))
            .children(self.render_disk_conflict(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_editor_body()),
            )
            .children(self.render_editor_close_confirm(cx))
    }

    fn render_editor_body(&self) -> AnyElement {
        if let Some(file) = self.editor.files.active() {
            return file.handle.entity.clone().into_any_element();
        }
        let (title, body) = if self.editor.is_loading() {
            ("Opening file…", "Reading the file from disk.")
        } else {
            (
                "No file open",
                "Select a file from the explorer on the left to edit.",
            )
        };
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(EmptyState::new("no-editor-file", IconName::FileCode, title).body(body))
            .into_any_element()
    }

    fn render_editor_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let tabs = self.editor.files.iter().map(|file| {
            EditorTab::new(file.path.clone(), file_name(&file.path))
                .icon(IconName::FileCode)
                .dirty(file.is_dirty())
        });
        let strip = EditorTabs::new("editor-tabs", tabs)
            .on_select(cx.listener(|this, id: &SharedString, _, cx| {
                this.switch_editor_file(id, cx);
            }))
            .on_close(cx.listener(|this, id: &SharedString, _, cx| {
                this.request_close_editor_file(id, cx);
            }));
        match self.editor.files.active_path() {
            Some(active) => strip.selected(active.to_string()),
            None => strip,
        }
    }

    fn render_editor_notice(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let notice = self.editor.notice.clone()?;
        let dismiss = app_callback(cx, |this, cx| {
            this.editor.notice = None;
            cx.notify();
        });
        Some(
            div().p_2().child(
                Alert::new("editor-notice", Severity::Danger, notice.title)
                    .body(notice.body)
                    .on_dismiss(dismiss),
            ),
        )
    }

    fn render_editor_close_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let path = self.editor.pending_close.clone()?;
        let message = format!("Unsaved changes to {} will be lost.", file_name(&path));
        let cancel = app_callback(cx, |this, cx| {
            this.editor.pending_close = None;
            cx.notify();
        });
        let discard = app_callback(cx, move |this, cx| this.close_editor_file(&path, cx));
        Some(
            ConfirmDialog::new("editor-discard", "Discard changes?", message, cancel)
                .confirm("Discard")
                .destructive()
                .on_confirm(discard)
                .into_any_element(),
        )
    }

    /// Breadcrumb, language, cursor position and actions.
    fn render_editor_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let active = self.editor.files.active();
        let active_path = active.map(|f| f.path.clone());
        let is_dirty = active.is_some_and(|f| f.is_dirty());
        let cursor = active.map(|file| {
            let (line, col) = file.handle.entity.read(cx).position();
            (line, col, file.line_count())
        });

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(theme.control_height(ControlSize::Lg))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(colors.fg_muted)
                    .child(
                        div()
                            .flex_none()
                            .child(Icon::new(IconName::FileCode).size(IconSize::Xs)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(active_path.clone().unwrap_or_else(|| "No file".into())),
                    ),
            )
            .when_some(active_path.zip(cursor), |el, (path, (line, col, lines))| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .children(self.add_selection_button(&path, cx))
                        .child(Badge::new(detect_language(&path)))
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(colors.fg_subtle)
                                .child(format!("Ln {line}, Col {col} ({lines} lines)")),
                        )
                        .child(
                            IconButton::new("editor-open-external", IconName::ExternalLink)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Open file in external editor")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_active_file_externally(cx);
                                })),
                        )
                        .child(
                            IconButton::new("editor-save-btn", IconName::Save)
                                .size(ControlSize::Sm)
                                .variant(if is_dirty {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .tooltip(if is_dirty {
                                    "Save changes (⌘S)"
                                } else {
                                    "Saved"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_current_editor_file(cx);
                                })),
                        ),
                )
            })
    }

    /// MonoCode `EditorSelectionMenu`: "Add to chat" while lines of the
    /// active file are selected; the prompt gets `@path (lines a-b)`.
    fn add_selection_button(&self, path: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let selection = self.editor.selection.as_ref().filter(|s| s.path == path)?;
        let root = self.workspace_cwd();
        let relative = std::path::Path::new(path)
            .strip_prefix(&root)
            .map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
        let reference = crate::ui::composer::add_to_chat::selection_reference(
            &relative,
            selection.start_line,
            selection.end_line,
        );
        Some(
            Button::new("editor-add-to-chat", "Add to chat")
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .icon(IconName::MessageSquare)
                .on_click(cx.listener(move |this, _, _, cx| this.add_to_chat(&reference, cx)))
                .into_any_element(),
        )
    }
}
