//! A multi-line field that scrolls in a box of its own. Ely's `TextInput`
//! scrolls only to keep its caret in view and ignores the wheel, so such a
//! field grows to fit its text, sits in a scrolling `div` with this handle,
//! and the box follows the caret when it moves (not while the wheel scrolls).

use std::cell::Cell;
use std::rc::Rc;

use bencode_ui::forms::TextInput;
use gpui::{Canvas, Entity, Pixels, ScrollHandle, canvas, point};

#[derive(Default)]
pub struct CaretFollow {
    /// The box's scroll: `track_scroll` it.
    pub scroll: ScrollHandle,
    /// The caret and the text's length as last seen.
    seen: Rc<Cell<(usize, usize)>>,
}

impl CaretFollow {
    /// Drawn inside the box after `input` (`.absolute().size_0()`): once the
    /// field is laid out, scrolls the box to its caret, `pad` inside.
    pub fn follow(&self, input: &Entity<TextInput>, pad: Pixels) -> Canvas<()> {
        let input = input.clone();
        let scroll = self.scroll.clone();
        let seen = self.seen.clone();
        canvas(
            move |_, window, cx| {
                let input = input.read(cx);
                let now = (input.cursor(), input.text().len());
                if seen.replace(now) == now {
                    return;
                }
                let Some(caret) = input.caret_bounds() else {
                    return;
                };
                let view = scroll.bounds();
                let offset = scroll.offset();
                let shift = if caret.bottom() > view.bottom() - pad {
                    view.bottom() - pad - caret.bottom()
                } else if caret.top() < view.top() + pad {
                    view.top() + pad - caret.top()
                } else {
                    return;
                };
                let y = (offset.y + shift)
                    .min(Pixels::ZERO)
                    .max(-scroll.max_offset().y);
                if y != offset.y {
                    scroll.set_offset(point(offset.x, y));
                    // The box was placed before this ran: draw it again,
                    // in a region too (`refresh` is a no-op mid-draw).
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
    }
}
