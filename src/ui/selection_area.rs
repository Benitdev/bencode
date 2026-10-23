//! What every view that selects text through `TranscriptSelection` shares
//! (the transcript, a review, an Inbox item): the keys of its selection
//! while it holds one, a press beside the text starting at the nearest
//! character (so a click on empty space clears, as in WebKit), a drag past
//! the list's edges scrolling it, and what ⌘C, ⌘A and Esc do.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Entity, FocusHandle, InteractiveElement, MouseButton, MouseMoveEvent,
    ParentElement, SharedString, canvas,
};

use crate::ui::transcript::selection::TranscriptSelection;

pub(crate) struct SelectionArea {
    pub selection: Entity<TranscriptSelection>,
    pub focus: FocusHandle,
    /// The key context its ⌘C, ⌘A and Esc are bound in.
    pub key_context: &'static str,
    /// The list it scrolls while a drag is past its edges.
    pub list: Option<gpui::ListState>,
    /// The places whose text it draws.
    pub owns: Rc<dyn Fn(&str) -> bool>,
    /// Where a press beside the text starts; None leaves such a press be.
    pub beside: Option<SharedString>,
    /// Holds the keys while nothing is selected anywhere.
    pub idle_owner: bool,
    /// Redraws what shows it, as a drag scrolls.
    pub redraw: Rc<dyn Fn(&mut App)>,
}

impl SelectionArea {
    pub fn wrap<E: InteractiveElement + ParentElement + IntoElement>(self, body: E, cx: &App) -> E {
        let Self {
            selection,
            focus,
            key_context,
            list,
            owns,
            beside,
            idle_owner,
            redraw,
        } = self;
        let holds = (selection.read(cx).session()).map_or(idle_owner, |place| owns(place));
        let scroll = list.map(|list| {
            let (selection, owns) = (selection.clone(), owns.clone());
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let (selection, list, owns, redraw) = (
                        selection.clone(),
                        list.clone(),
                        owns.clone(),
                        redraw.clone(),
                    );
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Bubble {
                            return;
                        }
                        let dragging_here = {
                            let selection = selection.read(cx);
                            selection
                                .session()
                                .is_some_and(|place| owns(place) && selection.is_dragging_in(place))
                        };
                        if !dragging_here {
                            return;
                        }
                        let view = list.viewport_bounds();
                        let y = event.position.y;
                        let past = if y < view.top() {
                            y - view.top()
                        } else if y > view.bottom() {
                            y - view.bottom()
                        } else {
                            return;
                        };
                        list.scroll_by(past / 2.0);
                        redraw(cx);
                    });
                },
            )
            .absolute()
            .size_full()
        });
        // Text, buttons (`OwnsPress`) and the scroll bar take their presses
        // first; what is left starts at the nearest character.
        let body = match beside {
            Some(place) => {
                let (selection, focus) = (selection.clone(), focus.clone());
                body.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    if window.default_prevented() {
                        return;
                    }
                    let started = selection.update(cx, |selection, cx| {
                        let started = selection.press_beside(
                            &place,
                            event.click_count,
                            event.modifiers.shift,
                        );
                        // A plain click selects nothing yet: no redraw until a drag.
                        if started && selection.has_selection() {
                            cx.notify();
                        }
                        started
                    });
                    if started {
                        window.focus(&focus, cx);
                    }
                })
            }
            None => body,
        };
        body.when(holds, |el| el.track_focus(&focus).key_context(key_context))
            .children(scroll)
    }
}

/// ⌘C: the selected text as shown; `fallback` gives the text of blocks the
/// list has not drawn.
pub(crate) fn copy(
    selection: &Entity<TranscriptSelection>,
    fallback: &[(usize, String)],
    cx: &mut App,
) {
    match selection.read(cx).selected_text(fallback) {
        Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        None => log::debug!("copy with nothing selected"),
    }
}

/// ⌘A: every block of `place`.
pub(crate) fn select_all(
    selection: &Entity<TranscriptSelection>,
    place: &SharedString,
    cx: &mut App,
) {
    selection.update(cx, |selection, cx| {
        selection.select_all(place);
        cx.notify();
    });
}

/// Esc: drops the selection; `false` when there was none, so the key can
/// do what it does elsewhere.
pub(crate) fn clear(selection: &Entity<TranscriptSelection>, cx: &mut App) -> bool {
    selection.update(cx, |selection, cx| {
        let cleared = selection.clear();
        if cleared {
            cx.notify();
        }
        cleared
    })
}

/// The places `gone` names changed: what was selected by block no longer
/// names the same text.
pub(crate) fn forget(
    selection: &Entity<TranscriptSelection>,
    gone: impl Fn(&str) -> bool,
    cx: &mut App,
) {
    selection.update(cx, |selection, cx| {
        if selection.forget(gone) {
            cx.notify();
        }
    });
}
