//! Editor rendering inside the file pane: a toolbar, the active editor, notices and the discard dialog.

use bencode_ui::buttons::{Button, ButtonVariant, IconButton, SegmentedControl};
use bencode_ui::data_display::Badge;
use bencode_ui::feedback::{Alert, EmptyState};
use bencode_ui::overlays::ConfirmDialog;
use bencode_ui::primitives::{Icon, IconName, Severity};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize, TextSize};
use gpui::{
    AnyElement, Context, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement,
    SharedString, Styled, div, prelude::*,
};

use super::EditorHandle;
use super::files::{detect_language, file_name};
use super::open_files::OpenFile;
use super::preview::{FileViewMode, is_markdown_path};
use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::tokens::Text;

impl BenCodeApp {
    /// The pane's active file: toolbar, disk conflict and editor.
    pub(crate) fn render_editor_file(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_editor_toolbar(cx))
            .children(self.render_disk_conflict(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_editor_body(cx)),
            )
    }

    /// The file of the pane's active tab, once read.
    fn shown_file(&self) -> Option<&OpenFile<EditorHandle>> {
        match self.file_pane.active()? {
            PaneTab::File { path } => self.editor.files.get(path),
            _ => None,
        }
    }

    fn render_editor_body(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(file) = self.shown_file() {
            if is_markdown_path(&file.path)
                && self.file_view_mode(&file.path) == FileViewMode::Preview
            {
                let text = file.handle.entity.read(cx).text();
                return self.render_markdown_preview(&file.path, text, cx);
            }
            // ⌘-click: the editor places the caret, then the caret's
            // observer goes to the definition (`app/go_to_definition.rs`).
            let (down, up) = (file.path.clone(), file.path.clone());
            return div()
                .size_full()
                .capture_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    let jump = event.button == MouseButton::Left && event.modifiers.platform;
                    this.definition.jump_on_click = jump.then(|| down.clone());
                }))
                .capture_any_mouse_up(cx.listener(move |this, _: &MouseUpEvent, _, _| {
                    if this.definition.jump_on_click.as_deref() == Some(up.as_str()) {
                        this.definition.jump_on_click = None;
                    }
                }))
                .child(file.handle.entity.clone())
                .into_any_element();
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

    /// Why a file did not open, or an external editor did not start.
    pub(crate) fn render_editor_notice(&self, cx: &Context<Self>) -> Option<Alert> {
        let notice = self.editor.notice.clone()?;
        let dismiss = app_callback(cx, |this, cx| {
            this.editor.notice = None;
            cx.notify();
        });
        Some(
            Alert::new("editor-notice", Severity::Danger, notice.title)
                .body(notice.body)
                .on_dismiss(dismiss),
        )
    }

    pub(crate) fn render_editor_close_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let path = self.editor.pending_close.clone()?;
        let message = format!("Unsaved changes to {} will be lost.", file_name(&path));
        let cancel = app_callback(cx, |this, cx| {
            this.editor.pending_close = None;
            this.redraw_regions(&[crate::ui::region::Region::FilePane], cx);
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
        let active = self.shown_file();
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
                        .children(self.view_mode_toggle(&path, cx))
                        .children(self.add_selection_button(&path, cx))
                        .children(self.blame_chip(&path, cx))
                        .children(self.definition_message(&path).map(|message| {
                            div()
                                .max_w(px(260.0))
                                .truncate()
                                .text_size(Text::Sm.px())
                                .text_color(cx.theme().colors.warning)
                                .child(message.to_string())
                        }))
                        .child(Badge::new(detect_language(&path)))
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(colors.fg_subtle)
                                .child(format!("Ln {line}, Col {col} ({lines} lines)")),
                        )
                        .child({
                            let history = path.clone();
                            IconButton::new("editor-file-history", IconName::History)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("File History")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_file_history(&history, cx);
                                }))
                        })
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

    /// MonoCode `MarkdownModeToggle`, for a markdown file.
    fn view_mode_toggle(&self, path: &str, cx: &Context<Self>) -> Option<AnyElement> {
        if !is_markdown_path(path) {
            return None;
        }
        let key = path.to_string();
        Some(
            SegmentedControl::new("editor-view-mode", self.file_view_mode(path).key())
                .size(ControlSize::Sm)
                .segment(FileViewMode::Preview.key(), "Preview", None)
                .segment(FileViewMode::Source.key(), "Source", None)
                .on_change(cx.listener(move |this, mode: &SharedString, _, cx| {
                    this.set_file_view_mode(&key, FileViewMode::from_key(mode), cx);
                }))
                .into_any_element(),
        )
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
