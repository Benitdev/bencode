//! Notes: a markdown scratchpad stored in MonoCode's notes table.

use std::rc::Rc;
use std::time::Duration;

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::{Input, SearchInput};
use ely_gpui_component::forms::{InputEvent, TextInput};
use ely_gpui_component::layout::MasterDetail;
use ely_gpui_component::lists::ListItem;
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use ely_gpui_component::typography::Caption;
use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, Styled, Window, div, prelude::*,
    uniform_list,
};

use crate::app::{BenCodeApp, Surface, multiline_input, now_ms, text_input};
use crate::db::{Note, NoteUpsert};
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// The Notes surface's data, selection, fields and pending dialogs.
pub struct NotesState {
    pub items: Vec<Note>,
    pub selected_id: Option<String>,
    pub filter_query: String,
    /// The notes list's scroll, for its scroll bar.
    pub scroll: gpui::UniformListScrollHandle,
    pub filter_input: Entity<TextInput>,
    pub title_input: Entity<TextInput>,
    pub body_input: Entity<TextInput>,
    /// Note id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// Bumped on every note edit; a pending autosave only runs if it still matches.
    pub autosave_generation: u64,
    /// Last failed note save, shown with a Retry action.
    pub save_error: Option<String>,
}

impl NotesState {
    /// The fields, with no notes loaded yet.
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        Self {
            items: Vec::new(),
            selected_id: None,
            filter_query: String::new(),
            scroll: Default::default(),
            filter_input: text_input(window, cx, "Filter notes..."),
            title_input: text_input(window, cx, "Note title..."),
            body_input: multiline_input(
                window,
                cx,
                "Write note or scratchpad in markdown...",
                (5, 25),
            ),
            pending_delete: None,
            autosave_generation: 0,
            save_error: None,
        }
    }
}

const UNTITLED_NOTE: &str = "Untitled";
/// MonoCode saves this long after the last keystroke (`NotesView.tsx`).
const AUTOSAVE_DELAY: Duration = Duration::from_millis(400);

/// "just now", "5m ago", "3h ago" or "2d ago" for a millisecond timestamp.
fn relative_time(now: i64, millis: i64) -> String {
    let secs = (now - millis).max(0) / 1000;
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// `query` must already be lower-case.
fn note_matches(note: &Note, query: &str) -> bool {
    query.is_empty()
        || note.title.to_lowercase().contains(query)
        || note.body.to_lowercase().contains(query)
        || note.tags.iter().any(|t| t.to_lowercase().contains(query))
}

/// Whether the editor's fields differ from the stored note.
fn note_is_dirty(note: &Note, title: &str, body: &str) -> bool {
    let title = if title.is_empty() {
        UNTITLED_NOTE
    } else {
        title
    };
    note.title != title || note.body != body
}

fn note_preview(body: &str) -> &str {
    body.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Empty note")
}

impl BenCodeApp {
    pub fn open_notes(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Notes, cx);
        self.refresh_notes(cx);
        cx.notify();
    }

    pub fn close_notes(&mut self, cx: &mut Context<Self>) {
        if self.surface_open(Surface::Notes) {
            self.close_surface(cx);
        }
    }

    fn refresh_notes(&mut self, cx: &mut Context<Self>) {
        match self.db.list_notes() {
            Ok(notes) => self.notes.items = notes,
            Err(err) => log::error!("list_notes failed: {err:#}"),
        }
        let selection_exists = self
            .notes.selected_id
            .as_ref()
            .is_some_and(|id| self.notes.items.iter().any(|n| &n.id == id));
        if !selection_exists {
            self.notes.selected_id = None;
            match self.notes.items.first().map(|n| n.id.clone()) {
                Some(id) => self.select_note(id, cx),
                None => self.clear_note_inputs(cx),
            }
        }
    }

    fn create_new_note(&mut self, cx: &mut Context<Self>) {
        self.save_note_if_dirty(cx);
        let upsert = NoteUpsert {
            id: crate::app::unique_id("note"),
            title: UNTITLED_NOTE.to_string(),
            body: String::new(),
            tags: Vec::new(),
            source_session_id: self.selected_session_id.clone(),
            source_cwd: Some(self.current_cwd.clone()),
        };
        match self.db.upsert_note(&upsert) {
            Ok(note) => {
                let id = note.id.clone();
                self.notes.items.insert(0, note);
                self.select_note(id, cx);
            }
            Err(err) => log::error!("upsert_note failed: {err:#}"),
        }
        cx.notify();
    }

    pub(crate) fn select_note(&mut self, id: String, cx: &mut Context<Self>) {
        if self.notes.selected_id.as_ref() != Some(&id) {
            self.save_note_if_dirty(cx);
        }
        if let Some(note) = self.notes.items.iter().find(|n| n.id == id) {
            let (title, body) = (note.title.clone(), note.body.clone());
            self.notes.title_input
                .update(cx, |input, cx| input.set_text(title, cx));
            self.notes.body_input
                .update(cx, |input, cx| input.set_text(body, cx));
        }
        self.notes.selected_id = Some(id);
        cx.notify();
    }

    fn clear_note_inputs(&mut self, cx: &mut Context<Self>) {
        self.notes.title_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.notes.body_input
            .update(cx, |input, cx| input.set_text("", cx));
    }

    /// Title/body edits save themselves `AUTOSAVE_DELAY` after typing stops,
    /// and right away when the title loses focus.
    pub(crate) fn on_note_input_event(
        &mut self,
        _: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => self.schedule_note_autosave(cx),
            InputEvent::Blur | InputEvent::Submit => self.save_note_if_dirty(cx),
            _ => {}
        }
    }

    fn schedule_note_autosave(&mut self, cx: &mut Context<Self>) {
        self.notes.autosave_generation += 1;
        let generation = self.notes.autosave_generation;
        let timer = cx.background_executor().timer(AUTOSAVE_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let saved = this.update(cx, |this, cx| {
                if this.notes.autosave_generation == generation {
                    this.save_note_if_dirty(cx);
                }
            });
            if let Err(err) = saved {
                log::debug!("note autosave after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Saves the open note when its fields differ from what is stored, so
    /// switching, creating or closing never drops edits.
    pub(crate) fn save_note_if_dirty(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self
            .notes.selected_id
            .as_ref()
            .and_then(|id| self.notes.items.iter().find(|n| &n.id == id))
        else {
            return;
        };
        let title = self.notes.title_input.read(cx).text().trim().to_string();
        let body = self.notes.body_input.read(cx).text().to_string();
        if note_is_dirty(note, &title, &body) {
            self.save_selected_note(cx);
        }
    }

    fn save_selected_note(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.notes.selected_id.clone() else {
            return;
        };
        let Some(pos) = self.notes.items.iter().position(|n| n.id == id) else {
            return;
        };
        let title = self.notes.title_input.read(cx).text().trim().to_string();
        let existing = &self.notes.items[pos];
        let upsert = NoteUpsert {
            id,
            title: if title.is_empty() {
                UNTITLED_NOTE.to_string()
            } else {
                title
            },
            body: self.notes.body_input.read(cx).text().to_string(),
            tags: existing.tags.clone(),
            source_session_id: existing.source_session_id.clone(),
            source_cwd: existing.source_cwd.clone(),
        };
        match self.db.upsert_note(&upsert) {
            Ok(saved) => {
                self.notes.items[pos] = saved;
                self.notes.save_error = None;
            }
            Err(err) => {
                log::error!("upsert_note failed: {err:#}");
                self.notes.save_error = Some(format!("Could not save note: {err}"));
            }
        }
        cx.notify();
    }

    fn delete_note(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(err) = self.db.delete_note(id) {
            log::error!("delete_note failed: {err:#}");
            return;
        }
        self.notes.items.retain(|n| n.id != id);
        if self.notes.selected_id.as_deref() == Some(id) {
            self.notes.selected_id = None;
            match self.notes.items.first().map(|n| n.id.clone()) {
                Some(next) => self.select_note(next, cx),
                None => self.clear_note_inputs(cx),
            }
        }
        cx.notify();
    }

    /// MonoCode "Add to chat": a new thread carrying the note as a card.
    fn add_selected_note_to_chat(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self
            .notes
            .items
            .iter()
            .find(|n| Some(&n.id) == self.notes.selected_id.as_ref())
            .cloned()
        else {
            return;
        };
        self.add_note_to_chat(&note, cx);
    }

    pub(crate) fn render_notes_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let min = theme.pane_min().to_pixels(theme.base_rem() * crate::ui::scale::ui_scale());
        div()
            .size_full()
            .child(MasterDetail::new(
                "notes-split",
                self.render_notes_master(cx),
                self.render_note_detail(cx),
                min,
            ))
            .children(self.render_note_delete_confirm(cx))
            .into_any_element()
    }

    fn render_notes_master(&self, cx: &Context<Self>) -> impl IntoElement {
        let query = self.notes.filter_query.trim().to_lowercase();
        let shown: Rc<[usize]> = (0..self.notes.items.len())
            .filter(|&ix| note_matches(&self.notes.items[ix], &query))
            .collect();
        let list = if shown.is_empty() {
            EmptyState::new("notes-empty", IconName::FileText, "No notes")
                .body(if query.is_empty() {
                    "Create one to start a scratchpad."
                } else {
                    "Nothing matches the filter."
                })
                .into_any_element()
        } else {
            let now = now_ms();
            let rows = cx.processor({
                let shown = shown.clone();
                move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|row| this.render_note_row(shown[row], now, cx))
                        .collect::<Vec<_>>()
                }
            });
            crate::ui::scrollbar::framed(
                "notes-list-scrollbar",
                &self.notes.scroll,
                uniform_list("notes-list", shown.len(), rows)
                    .track_scroll(&self.notes.scroll)
                    .size_full()
                    .pr(crate::ui::scrollbar::gutter(&self.notes.scroll)),
            )
            .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .size_full()
            .pr_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            SearchInput::new("notes-filter", &self.notes.filter_input)
                                .size(ControlSize::Sm),
                        ),
                    )
                    .child(
                        IconButton::new("notes-new", IconName::Plus)
                            .size(ControlSize::Sm)
                            .tooltip("New note")
                            .on_click(cx.listener(|this, _, _, cx| this.create_new_note(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list))
    }

    fn render_note_row(&self, ix: usize, now: i64, cx: &Context<Self>) -> ListItem {
        let note = &self.notes.items[ix];
        let id = note.id.clone();
        ListItem::new(("note-row", ix), note.title.clone())
            .description(note_preview(&note.body).to_string())
            .trailing(Caption::new(relative_time(now, note.updated_at)))
            .selected(self.notes.selected_id.as_deref() == Some(note.id.as_str()))
            .on_click(cx.listener(move |this, _, _, cx| this.select_note(id.clone(), cx)))
    }

    fn render_note_detail(&self, cx: &Context<Self>) -> AnyElement {
        let Some(id) = self.notes.selected_id.clone() else {
            return EmptyState::new("note-none", IconName::FileText, "No note selected")
                .action(
                    Button::new("note-none-new", "New note")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.create_new_note(cx))),
                )
                .into_any_element();
        };
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().child(Input::new(&self.notes.title_input)))
            .child(
                Button::new("note-to-chat", "Add to chat")
                    .variant(ButtonVariant::Secondary)
                    .icon(IconName::MessageSquare)
                    .on_click(cx.listener(|this, _, _, cx| this.add_selected_note_to_chat(cx))),
            )
            .child(
                IconButton::new("note-delete", IconName::Trash2)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Delete note")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.notes.pending_delete = Some(id.clone());
                        cx.notify();
                    })),
            );
        div()
            .flex()
            .flex_col()
            .gap_3()
            .pl_4()
            .child(toolbar)
            .when_some(self.notes.save_error.clone(), |el, error| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.0))
                        .text_color(cx.theme().colors.danger)
                        .child(error)
                        .child(
                            Button::new("note-save-retry", "Retry")
                                .variant(ButtonVariant::Ghost)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.save_selected_note(cx)),
                                ),
                        ),
                )
            })
            .child(Input::new(&self.notes.body_input))
            .into_any_element()
    }

    fn render_note_delete_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let id = self.notes.pending_delete.clone()?;
        let title = self
            .notes
            .items
            .iter()
            .find(|n| n.id == id)
            .map_or(UNTITLED_NOTE, |n| n.title.as_str());
        let close = app_callback(cx, |this, cx| {
            this.notes.pending_delete = None;
            cx.notify();
        });
        let delete = app_callback(cx, move |this, cx| this.delete_note(&id, cx));
        Some(
            ConfirmDialog::new(
                "note-delete-confirm",
                "Delete note?",
                format!("“{title}” will be removed."),
                close,
            )
            .confirm("Delete")
            .destructive()
            .on_confirm(delete),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, body: &str, tags: &[&str]) -> Note {
        Note {
            id: "n".into(),
            slug: "n".into(),
            title: title.into(),
            body: body.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            source_session_id: None,
            source_cwd: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn relative_time_buckets() {
        let now = 10 * 86_400_000;
        assert_eq!(relative_time(now, now + 5_000), "just now");
        assert_eq!(relative_time(now, now - 30_000), "just now");
        assert_eq!(relative_time(now, now - 5 * 60_000), "5m ago");
        assert_eq!(relative_time(now, now - 3 * 3_600_000), "3h ago");
        assert_eq!(relative_time(now, now - 2 * 86_400_000), "2d ago");
    }

    #[test]
    fn filter_checks_title_body_and_tags() {
        let n = note("Plan", "Ship the Parser", &["Rust"]);
        assert!(note_matches(&n, ""));
        assert!(note_matches(&n, "parser"));
        assert!(note_matches(&n, "rust"));
        assert!(!note_matches(&n, "go"));
    }

    #[test]
    fn preview_skips_blank_lines() {
        assert_eq!(note_preview("\n  \nfirst\nsecond"), "first");
        assert_eq!(note_preview("   "), "Empty note");
    }

    #[test]
    fn dirty_check_treats_blank_title_as_untitled() {
        let note = Note {
            title: UNTITLED_NOTE.into(),
            body: "x".into(),
            ..Default::default()
        };
        assert!(!note_is_dirty(&note, "", "x"));
        assert!(note_is_dirty(&note, "", "y"));
        assert!(note_is_dirty(&note, "Plan", "x"));
    }
}
