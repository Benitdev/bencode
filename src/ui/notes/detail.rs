//! MonoCode `NoteDetail` / `NoteEditor`: the open note's slug and project,
//! title, tags, Add to chat and Delete, then its body as Preview or
//! Source, where images can be dropped. The Source field wraps as
//! MonoCode's does; it has no line numbers.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::app::notes::{MAX_NOTE_TAGS, NoteMode, note_project};
use crate::db::Note;
use crate::ui::app_callback::app_callback;
use crate::ui::page_parts::{page_tab, tint};
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;
use crate::ui::transcript::markdown::AgentMarkdown;

/// MonoCode `max-w-5xl`.
const PAGE_WIDTH: f32 = 1024.0;
/// MonoCode `min-h-[448px]`.
const BODY_MIN_HEIGHT: f32 = 448.0;

impl BenCodeApp {
    pub(super) fn render_note_detail(&self, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let Some(note) = self.note_draft(cx) else {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .size_full()
                .child(
                    Icon::new(IconName::File)
                        .size(IconSize::Lg)
                        .color(fg.opacity(0.3)),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(fg.opacity(tint::QUIET))
                        .child("Select a note"),
                )
                .into_any_element();
        };
        let mode = self.notes.mode();
        let tabs = div()
            .flex()
            .flex_none()
            .gap_4()
            .h(px(36.0))
            .border_b_1()
            .border_color(colors.border)
            .child(
                page_tab(
                    "note-tab-preview",
                    "Preview",
                    mode == NoteMode::Preview,
                    fg,
                    colors.fg_muted,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_note_mode(NoteMode::Preview, cx))),
            )
            .child(
                page_tab(
                    "note-tab-source",
                    "Source",
                    mode == NoteMode::Source,
                    fg,
                    colors.fg_muted,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_note_mode(NoteMode::Source, cx))),
            );
        let body: AnyElement = match mode {
            NoteMode::Source => div()
                .font_family(cx.theme().mono_family.clone())
                .text_size(px(13.0))
                .line_height(px(20.0))
                .child(self.notes.body_input.clone())
                .into_any_element(),
            NoteMode::Preview if note.body.trim().is_empty() => div()
                .text_size(px(13.0))
                .text_color(fg.opacity(tint::QUIET))
                .child("No description")
                .into_any_element(),
            NoteMode::Preview => {
                AgentMarkdown::new("note-preview", note.body.clone()).into_any_element()
            }
        };
        let page = div()
            .id("note-detail")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .mx_auto()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .p_8()
                    .child(self.render_note_header(&note, cx))
                    .child(tabs)
                    .child(self.render_note_body(body, cx)),
            );
        Scrolled::new("note-detail-scrollbar", page).into_any_element()
    }

    /// The body and MonoCode's drop zone around it: images dropped here are
    /// copied into the note.
    fn render_note_body(&self, body: AnyElement, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (accent, bg) = (colors.accent, colors.bg);
        div()
            .id("note-body")
            .relative()
            .min_h(px(BODY_MIN_HEIGHT))
            .rounded(px(8.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| {
                style
                    .border_color(accent.opacity(0.6))
                    .bg(accent.opacity(tint::HOVER))
            })
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_note_images(paths.paths().to_vec(), cx);
            }))
            .child(body)
            .when(self.notes.adding_images, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.0))
                        .bg(bg.opacity(0.8))
                        .text_size(px(12.0))
                        .text_color(colors.fg.opacity(tint::BODY))
                        .child("Adding images…"),
                )
            })
    }

    fn render_note_header(&self, note: &Note, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let delete_id = note.id.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .text_size(px(12.0))
                    .text_color(muted)
                    .child(Icon::new(IconName::File).size(IconSize::Sm).color(muted))
                    .child(div().flex_none().child("Note"))
                    .when(!note.slug.is_empty(), |el| {
                        el.child(div().min_w_0().truncate().child(note.slug.clone()))
                    })
                    .child(self.render_note_project(note, cx)),
            )
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(self.notes.title_input.clone()),
            )
            .children(self.note_updated().map(|time| {
                div()
                    .text_size(px(12.0))
                    .text_color(muted)
                    .child(format!("Updated {time}"))
            }))
            .child(self.render_note_tags(cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("note-to-chat", "Add to chat")
                            .primary()
                            .size(ControlSize::Sm)
                            .disabled(note.body.trim().is_empty())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.add_selected_note_to_chat(cx)),
                            ),
                    )
                    .child(
                        Button::new("note-delete", "Delete")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .icon(IconName::Trash2)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.notes.pending_delete = Some(delete_id.clone());
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.notes.save_error.clone(), |el, error| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.0))
                        .text_color(colors.danger)
                        .child(error)
                        .child(
                            Button::new("note-save-retry", "Retry")
                                .variant(ButtonVariant::Link)
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| this.retry_note_save(cx))),
                        ),
                )
            })
    }

    /// MonoCode `SearchableProjectPicker` in `move` mode: the project the
    /// note belongs to.
    fn render_note_project(&self, note: &Note, cx: &Context<Self>) -> impl IntoElement {
        let current = note.source_cwd.as_deref().unwrap_or_default();
        let menu = self
            .worktree_project_choices()
            .into_iter()
            .fold(Menu::new(), |menu, path| {
                let pick = app_callback(cx, {
                    let path = path.clone();
                    move |this, cx| this.move_note_to_project(&path, cx)
                });
                menu.item(
                    MenuItem::radio(self.rail_project_label(&path), path == current).on_click(pick),
                )
            });
        let label = note_project(note).map_or_else(|| "No project".to_string(), str::to_string);
        DropdownMenu::new("note-project", label, menu)
            .icon(IconName::Folder)
            .variant(ButtonVariant::Ghost)
    }

    /// MonoCode `NoteTagsEditor`.
    fn render_note_tags(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let tags = &self.notes.tags;
        let chips = tags.iter().enumerate().map(|(ix, tag)| {
            let remove = tag.clone();
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h(px(24.0))
                .max_w(px(192.0))
                .pl_2()
                .pr_1()
                .rounded(px(6.0))
                .bg(fg.opacity(tint::FILL))
                .text_size(px(11.0))
                .text_color(fg.opacity(tint::BODY))
                .child(div().min_w_0().truncate().child(format!("#{tag}")))
                .child(
                    IconButton::new(("note-tag-remove", ix), IconName::X)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip(format!("Remove #{tag}"))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_note_tag(&remove, cx)),
                        ),
                )
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .child(
                div()
                    .mr(px(2.0))
                    .text_size(px(11.0))
                    .text_color(fg.opacity(tint::QUIET))
                    .child("Tags"),
            )
            .children(chips)
            .when(tags.len() < MAX_NOTE_TAGS, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w(px(80.0))
                        .text_size(px(11.0))
                        .child(self.notes.tag_input.clone()),
                )
            })
    }
}
