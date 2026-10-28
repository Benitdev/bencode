//! What every view that selects text through the window's selection shares
//! (the transcript, a review, an Inbox item): the keys of its selection
//! while it holds one, a press beside the text starting at the nearest
//! character (so a click on empty space clears, as in WebKit), and what
//! ⌘C, ⌘A and Esc do. A drag past a list's edges scrolls it
//! (`bencode_ui::selection`, the document's `list`).

use std::rc::Rc;

use bencode_ui::selection::TextSelection;
use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Entity, FocusHandle, InteractiveElement, MouseButton, SharedString,
};

pub(crate) struct SelectionArea {
    pub selection: Entity<TextSelection>,
    pub focus: FocusHandle,
    /// The key context its ⌘C, ⌘A and Esc are bound in.
    pub key_context: &'static str,
    /// The documents whose text it draws.
    pub owns: Rc<dyn Fn(&str) -> bool>,
    /// Where a press beside the text starts; None leaves such a press be.
    pub beside: Option<SharedString>,
    /// Holds the keys while nothing is selected anywhere.
    pub idle_owner: bool,
}

impl SelectionArea {
    pub fn wrap<E: InteractiveElement + IntoElement>(self, body: E, cx: &App) -> E {
        let Self {
            selection,
            focus,
            key_context,
            owns,
            beside,
            idle_owner,
        } = self;
        let holds = (selection.read(cx).doc()).map_or(idle_owner, |doc| owns(&doc));
        // Text, buttons (`OwnsPress`) and the scroll bar take their presses
        // first; what is left starts at the nearest character.
        let body = match beside {
            Some(doc) => body.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !window.default_prevented() {
                    selection.read(cx).press_beside(&doc);
                }
            }),
            None => body,
        };
        body.when(holds, |el| el.track_focus(&focus).key_context(key_context))
    }
}

/// ⌘C: the selected text as shown; `fallback` gives the text of blocks the
/// list has not drawn.
pub(crate) fn copy(selection: &Entity<TextSelection>, fallback: &[(usize, String)], cx: &mut App) {
    match selection.read(cx).selected_text(fallback) {
        Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        None => log::debug!("copy with nothing selected"),
    }
}
