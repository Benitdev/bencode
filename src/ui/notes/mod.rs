//! MonoCode `NotesView.tsx`: the notes list beside the open note. State
//! and logic: `app/notes.rs`.

mod detail;
mod list;

use ely_gpui_component::layout::SplitPane;
use ely_gpui_component::overlays::ConfirmDialog;
use gpui::{AnyElement, Axis, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::app::notes::UNTITLED_NOTE;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// MonoCode `MIN_WIDTH`: the least the list (and the note) may be dragged to.
const MIN_PANE_WIDTH: f32 = 240.0;

impl BenCodeApp {
    pub(crate) fn render_notes_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let split = SplitPane::new("notes-split", Axis::Horizontal, px(MIN_PANE_WIDTH))
            .sizes(&self.notes.shares)
            .on_resize(move |shares, _window, cx| {
                let landed = weak.update(cx, |this, _| {
                    if let [list, note] = shares {
                        this.notes.shares = [*list, *note];
                    }
                });
                if let Err(err) = landed {
                    log::debug!("notes resize after app drop: {err:#}");
                }
            })
            .pane(div().size_full().child(self.render_notes_list(cx)))
            .pane(div().size_full().child(self.render_note_detail(cx)));
        div()
            .size_full()
            .child(split)
            .children(self.render_note_delete_confirm(cx))
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
