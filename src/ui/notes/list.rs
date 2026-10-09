//! MonoCode `NotesView`'s list and `NoteCard`: the filter, New note, and a
//! card per note with its project, age, title, first prose and tags.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::notes::{note_matches, note_preview, note_project};
use crate::app::{BenCodeApp, now_ms};
use crate::db::Note;
use crate::ui::mascot::pixel_sprite;
use crate::ui::page_parts::tint;
use crate::ui::relative_time;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;

/// MonoCode shows this many tags on a card, then "+N".
const CARD_TAGS: usize = 3;

impl BenCodeApp {
    pub(super) fn render_notes_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let query = self
            .notes
            .filter_input
            .read(cx)
            .text()
            .trim()
            .to_lowercase();
        let now = now_ms();
        let cards: Vec<AnyElement> = self
            .notes
            .items
            .iter()
            .enumerate()
            .filter(|(_, note)| note_matches(note, &query))
            .map(|(ix, note)| self.render_note_card(ix, note, now, cx))
            .collect();
        let quiet = |text: String| {
            div()
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .text_color(fg.opacity(tint::SOFT))
                .child(text)
                .into_any_element()
        };
        let list = match (&self.notes.load_error, cards.is_empty()) {
            (Some(error), true) if self.notes.items.is_empty() => quiet(error.clone()),
            (_, true) if !query.is_empty() => quiet("No matching notes".to_string()),
            (_, true) => quiet(
                "No notes yet. Save a turn from the transcript, or create one here.".to_string(),
            ),
            (_, false) => {
                let cards = div()
                    .id("note-cards")
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .size_full()
                    .p(px(6.0))
                    .overflow_y_scroll()
                    .children(cards);
                Scrolled::new("note-cards-scrollbar", cards).into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(36.0))
                    .pl(px(10.0))
                    .pr_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Xs)
                            .color(colors.fg_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.0))
                            .child(self.notes.filter_input.clone()),
                    )
                    .child(
                        IconButton::new("notes-new", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("New note")
                            .disabled(self.notes.creating)
                            .on_click(cx.listener(|this, _, _, cx| this.create_new_note(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list))
    }

    /// MonoCode `NoteCard`.
    fn render_note_card(&self, ix: usize, note: &Note, now: i64, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let active = self.notes.selected_id.as_deref() == Some(note.id.as_str());
        let project = note_project(note);
        let preview = note_preview(&note.body, &note.title);
        let hint = match project {
            Some(project) => format!("{} · {project}", note.title),
            None => note.title.clone(),
        };
        let id = note.id.clone();
        let mark = project
            .zip(note.source_cwd.as_deref())
            .map(|(project, cwd)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w_0()
                    .child(pixel_sprite(
                        &self.project_mascot(cwd).rest,
                        px(12.0),
                        self.project_color(cwd),
                        false,
                    ))
                    .child(div().min_w_0().truncate().child(project.to_string()))
            });
        let tags = (!note.tags.is_empty()).then(|| {
            let extra = note.tags.len().saturating_sub(CARD_TAGS);
            div()
                .flex()
                .items_center()
                .gap_1()
                .mt(px(6.0))
                .min_w_0()
                .overflow_hidden()
                .children(note.tags.iter().take(CARD_TAGS).map(|tag| {
                    div()
                        .flex_none()
                        .max_w(px(96.0))
                        .truncate()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(fg.opacity(tint::FILL))
                        .text_size(px(10.0))
                        .text_color(fg.opacity(tint::SOFT))
                        .child(format!("#{tag}"))
                }))
                .when(extra > 0, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(px(10.0))
                            .text_color(fg.opacity(tint::HINT))
                            .child(format!("+{extra}")),
                    )
                })
        });
        div()
            .id(("note-card", ix))
            .flex()
            .flex_col()
            .flex_none()
            .px(px(10.0))
            .py_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if active {
                    el.bg(colors.active)
                } else {
                    el.hover(|style| style.bg(fg.opacity(tint::HOVER)))
                }
            })
            .tooltip(Tooltip::text(hint))
            .on_click(cx.listener(move |this, _, _, cx| this.select_note(id.clone(), cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(fg.opacity(tint::SOFT))
                            .children(mark),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(fg.opacity(tint::QUIET))
                            .child(relative_time::since(note.updated_at, now)),
                    ),
            )
            .child(
                div()
                    .mt_1()
                    .truncate()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(note.title.clone()),
            )
            .when(!preview.is_empty(), |el| {
                el.child(
                    div()
                        .mt_1()
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(tint::QUIET))
                        .child(preview),
                )
            })
            .children(tags)
            .into_any_element()
    }
}
