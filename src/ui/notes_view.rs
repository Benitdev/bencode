use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled,
    div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::{Note, NoteUpsert};
use crate::ui::theme::MonoTheme;

pub fn format_relative_time(millis: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let diff_secs = (now - millis).max(0) / 1000;
    if diff_secs < 60 {
        "just now".to_string()
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        format!("{}d ago", diff_secs / 86400)
    }
}

impl BenCodeApp {
    pub fn open_notes(&mut self, cx: &mut Context<Self>) {
        self.is_notes_open = true;
        self.refresh_notes(cx);
        cx.notify();
    }

    pub fn close_notes(&mut self, cx: &mut Context<Self>) {
        self.is_notes_open = false;
        cx.notify();
    }

    pub fn refresh_notes(&mut self, cx: &mut Context<Self>) {
        if let Ok(notes) = self.db.list_notes() {
            self.notes = notes;
            if self.selected_note_id.is_none() && !self.notes.is_empty() {
                let first_id = self.notes[0].id.clone();
                self.select_note(first_id, cx);
            }
        }
    }

    pub fn create_new_note(&mut self, cx: &mut Context<Self>) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let new_id = format!("note-{}", stamp);
        let upsert = NoteUpsert {
            id: new_id.clone(),
            title: "Untitled Note".to_string(),
            body: "".to_string(),
            tags: vec![],
            source_session_id: self.selected_session_id.clone(),
            source_cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string()),
        };

        if let Ok(note) = self.db.upsert_note(&upsert) {
            self.notes.insert(0, note);
            self.select_note(new_id, cx);
        }
        cx.notify();
    }

    pub fn select_note(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_note_id = Some(id.clone());
        if let Some(note) = self.notes.iter().find(|n| n.id == id) {
            let title = note.title.clone();
            let body = note.body.clone();
            self.note_title_input.update(cx, |this, cx| {
                this.set_text(title, cx);
            });
            self.note_body_input.update(cx, |this, cx| {
                this.set_text(body, cx);
            });
        }
        cx.notify();
    }

    pub fn save_selected_note(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = &self.selected_note_id {
            let title = self.note_title_input.read(cx).text().to_string();
            let body = self.note_body_input.read(cx).text().to_string();
            let existing = self.notes.iter().find(|n| &n.id == id);
            let tags = existing.map(|n| n.tags.clone()).unwrap_or_default();
            let source_cwd = existing.and_then(|n| n.source_cwd.clone());

            let upsert = NoteUpsert {
                id: id.clone(),
                title: if title.trim().is_empty() {
                    "Untitled Note".to_string()
                } else {
                    title
                },
                body,
                tags,
                source_session_id: self.selected_session_id.clone(),
                source_cwd,
            };

            if let Ok(saved) = self.db.upsert_note(&upsert)
                && let Some(pos) = self.notes.iter().position(|n| &n.id == id) {
                    self.notes[pos] = saved;
                }
        }
        cx.notify();
    }

    pub fn delete_selected_note(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_note_id.take() {
            if let Err(err) = self.db.delete_note(&id) {
            log::error!("delete_note failed: {err:#}");
        }
            self.notes.retain(|n| n.id != id);
            if let Some(first) = self.notes.first() {
                let next_id = first.id.clone();
                self.select_note(next_id, cx);
            } else {
                self.note_title_input.update(cx, |this, cx| {
                    this.set_text("", cx);
                });
                self.note_body_input.update(cx, |this, cx| {
                    this.set_text("", cx);
                });
            }
        }
        cx.notify();
    }

    pub fn add_selected_note_to_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = &self.selected_note_id
            && let Some(note) = self.notes.iter().find(|n| &n.id == id) {
                let note_content = format!(
                    "--- Note: {} (@note/{}) ---\n{}\n--- End Note ---",
                    note.title, note.slug, note.body
                );
                self.prompt_input.update(cx, |this, cx| {
                    let current = this.text().to_string();
                    let new_text = if current.is_empty() {
                        note_content
                    } else {
                        format!("{}\n\n{}", current, note_content)
                    };
                    this.set_text(new_text, cx);
                });
                self.is_notes_open = false;
            }
        cx.notify();
    }

    pub fn render_notes_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let query = self.note_filter_query.to_lowercase();
        let filtered_notes: Vec<Note> = self
            .notes
            .iter()
            .filter(|n| {
                if query.is_empty() {
                    true
                } else {
                    n.title.to_lowercase().contains(&query)
                        || n.body.to_lowercase().contains(&query)
                        || n.tags.iter().any(|t| t.to_lowercase().contains(&query))
                }
            })
            .cloned()
            .collect();

        let selected_id = self.selected_note_id.clone();

        div()
            .id("notes-overlay-backdrop")
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("notes-modal-card")
                    .w(px(900.0))
                    .h(px(600.0))
                    .max_w_full()
                    .max_h_full()
                    .bg(MonoTheme::bg_surface())
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .rounded(theme.radius(Radius::Lg))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // Modal Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(44.0))
                            .px_4()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::FileText)
                                            .size(IconSize::Sm)
                                            .color(MonoTheme::accent()),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(MonoTheme::fg_base())
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .child("Notes & Scratchpad"),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child(format!("({} notes)", self.notes.len())),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("new-note-btn")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_hover())
                                            .text_color(MonoTheme::fg_base())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::MEDIUM)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::accent()).text_color(MonoTheme::on_accent()))
                                            .child(Icon::new(IconName::Plus).size(IconSize::Xs))
                                            .child("New Note")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.create_new_note(cx);
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("close-notes-btn")
                                            .size(px(24.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(theme.radius(Radius::Sm))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .child(
                                                Icon::new(IconName::X)
                                                    .size(IconSize::Xs)
                                                    .color(MonoTheme::fg_subtle()),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_notes(cx);
                                            })),
                                    ),
                            ),
                    )
                    // Two-Pane Content
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            // Left Pane: Notes List (w=280px)
                            .child(
                                div()
                                    .w(px(280.0))
                                    .border_r_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_base())
                                    .flex()
                                    .flex_col()
                                    .child(
                                        // Search Bar
                                        div()
                                            .p_2()
                                            .border_b_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .child(self.note_filter_input.clone()),
                                    )
                                    // List
                                    .child(
                                        div()
                                            .id("notes-list-scroll")
                                            .flex_1()
                                            .overflow_y_scroll()
                                            .p_1()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .children(filtered_notes.into_iter().map(|note| {
                                                let is_selected = selected_id.as_deref() == Some(&note.id);
                                                let note_id = note.id.clone();
                                                let time_label = format_relative_time(note.updated_at);
                                                let preview = if note.body.trim().is_empty() {
                                                    "Empty note".to_string()
                                                } else {
                                                    note.body.lines().next().unwrap_or("").to_string()
                                                };

                                                div()
                                                    .id(format!("note-item-{}", note.id))
                                                    .p_2()
                                                    .rounded(theme.radius(Radius::Sm))
                                                    .cursor_pointer()
                                                    .bg(if is_selected {
                                                        MonoTheme::bg_active()
                                                    } else {
                                                        gpui::rgba(0x00000000)
                                                    })
                                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .justify_between()
                                                            .child(
                                                                div()
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_size(theme.text_size(TextSize::Sm))
                                                                    .text_color(if is_selected {
                                                                        MonoTheme::fg_base()
                                                                    } else {
                                                                        MonoTheme::fg_muted()
                                                                    })
                                                                    .truncate()
                                                                    .child(note.title.clone()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(theme.text_size(TextSize::Xs))
                                                                    .text_color(MonoTheme::fg_subtle())
                                                                    .child(time_label),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .mt_1()
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_subtle())
                                                            .truncate()
                                                            .child(preview),
                                                    )
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.select_note(note_id.clone(), cx);
                                                    }))
                                            })),
                                    ),
                            )
                            // Right Pane: Note Editor
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .bg(MonoTheme::bg_surface())
                                    .p_4()
                                    // Top Action Bar
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .pb_3()
                                            .border_b_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .mr_4()
                                                    .child(self.note_title_input.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .id("add-note-to-chat-btn")
                                                            .px_2()
                                                            .py_1()
                                                            .rounded(theme.radius(Radius::Sm))
                                                            .bg(MonoTheme::bg_active())
                                                            .text_color(MonoTheme::accent())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::accent()).text_color(MonoTheme::on_accent()))
                                                            .child("↵ Add to Chat")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.add_selected_note_to_chat(cx);
                                                            })),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("save-note-btn")
                                                            .flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded(theme.radius(Radius::Sm))
                                                            .bg(MonoTheme::bg_hover())
                                                            .text_color(MonoTheme::fg_base())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_active()))
                                                            .child(Icon::new(IconName::Save).size(IconSize::Xs))
                                                            .child("Save")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.save_selected_note(cx);
                                                            })),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("delete-note-btn")
                                                            .flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded(theme.radius(Radius::Sm))
                                                            .bg(MonoTheme::bg_hover())
                                                            .text_color(MonoTheme::status_error())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::status_error()).text_color(MonoTheme::on_accent()))
                                                            .child(
                                                                Icon::new(IconName::Trash2)
                                                                    .size(IconSize::Xs)
                                                                    .color(MonoTheme::status_error()),
                                                            )
                                                            .child("Delete")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.delete_selected_note(cx);
                                                            })),
                                                    ),
                                            ),
                                    )
                                    // Body Editor Area
                                    .child(
                                        div()
                                            .flex_1()
                                            .pt_3()
                                            .flex()
                                            .flex_col()
                                            .child(self.note_body_input.clone()),
                                    ),
                            ),
                    ),
            )
    }
}
