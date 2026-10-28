//! Selecting transcript text. MonoCode's transcript is `user-select: text`
//! (`.agent-transcript` in `src/styles/index.css`), so WebKit lets a drag
//! select across messages, a double click take a word, a triple click a
//! block, and ⌘C copy the text as shown. GPUI text has none of that; the
//! window's selection (`bencode_ui::selection`) gives it: every run of
//! transcript text joins its thread's document as a segment in reading
//! order, and one selection spans them.
//!
//! A selection inside one settled answer or one prompt also offers
//! MonoCode's `TranscriptSelectionMenu` (`useTranscriptSelection.ts`):
//! Add to chat, Add to notes (`selection_ui.rs`).

use std::ops::Range;

use bencode_ui::selection::{Document, Joint, SegKey, SelectableText, TextSelection};
use gpui::prelude::*;
use gpui::{
    App, CursorStyle, Div, Entity, Hsla, MouseButton, SharedString, Stateful, TextLayout, TextRun,
    div,
};

/// Splits `runs` at the ends of `range` and restyles the runs inside it.
pub fn restyle(runs: &mut Vec<TextRun>, range: &Range<usize>, style: impl Fn(&mut TextRun)) {
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs.drain(..) {
        let end = at + run.len;
        let cuts = [
            at,
            range.start.clamp(at, end),
            range.end.clamp(at, end),
            end,
        ];
        for pair in cuts.windows(2) {
            if pair[0] >= pair[1] {
                continue;
            }
            let mut piece = TextRun {
                len: pair[1] - pair[0],
                ..run.clone()
            };
            if pair[0] >= range.start && pair[1] <= range.end {
                style(&mut piece);
            }
            out.push(piece);
        }
        at = end;
    }
    *runs = out;
}

/// Where transcript text is drawn: its document (a thread, a review's
/// lane, an Inbox item), its block, and whether that block is a settled
/// answer or a prompt, whose selections can be quoted.
#[derive(Clone)]
pub struct SegCtx {
    pub doc: Document,
    pub block: usize,
    pub quotable: bool,
}

/// A run of transcript text the pointer can select; `layout` locates
/// characters for the caller's own clicks (links).
pub struct Selectable {
    pub element: Stateful<Div>,
    pub layout: TextLayout,
}

impl SegCtx {
    pub fn selection(&self) -> &Entity<TextSelection> {
        self.doc.selection()
    }

    /// `text` with `runs`, selectable as segment `seq` of the block.
    pub fn text(
        &self,
        seq: usize,
        joint: Joint,
        text: impl Into<SharedString>,
        runs: Vec<TextRun>,
    ) -> Selectable {
        let key = SegKey {
            block: self.block,
            seq,
        };
        let text = SelectableText::new("text", &self.doc, key, text, runs)
            .joint(joint)
            .quotable(self.quotable);
        let layout = text.layout();
        let id = SharedString::from(format!("sel-{}-{}-{seq}", self.doc.id(), self.block));
        let element = div().id(id).cursor(CursorStyle::IBeam).child(text);
        Selectable { element, layout }
    }
}

/// An element that answers its own press (a card header, a copy button,
/// a chip): a drag that starts on it selects no text. bencode-ui's buttons
/// already claim their presses so.
pub trait OwnsPress: InteractiveElement + Sized {
    fn owns_press(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            bencode_ui::selection::claim_press(cx);
        })
    }
}

impl<T: InteractiveElement> OwnsPress for T {}

/// Plain transcript text (a prompt, a notice) in the inherited font and
/// ink; `marks` paints find matches behind it.
#[derive(IntoElement)]
pub struct PlainText {
    ctx: SegCtx,
    text: String,
    marks: Vec<(Range<usize>, Hsla)>,
}

impl PlainText {
    pub fn new(ctx: SegCtx, text: impl Into<String>) -> Self {
        Self {
            ctx,
            text: text.into(),
            marks: Vec::new(),
        }
    }

    pub fn marks(mut self, marks: Vec<(Range<usize>, Hsla)>) -> Self {
        self.marks = marks;
        self
    }
}

impl RenderOnce for PlainText {
    fn render(self, window: &mut gpui::Window, _: &mut App) -> impl IntoElement {
        let style = window.text_style();
        let mut runs = if self.text.is_empty() {
            Vec::new()
        } else {
            vec![TextRun {
                len: self.text.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }]
        };
        for (range, color) in &self.marks {
            let range = range.start.min(self.text.len())..range.end.min(self.text.len());
            restyle(&mut runs, &range, |run| run.background_color = Some(*color));
        }
        self.ctx.text(0, Joint::Block, self.text, runs).element
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restyling_splits_the_runs_it_crosses() {
        let run = |len| TextRun {
            len,
            font: gpui::font("Body"),
            color: gpui::white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut painted = vec![run(4), run(6)];
        restyle(&mut painted, &(2..7), |run| {
            run.background_color = Some(gpui::blue())
        });
        let lens: Vec<_> = painted
            .iter()
            .map(|r| (r.len, r.background_color.is_some()))
            .collect();
        assert_eq!(lens, [(2, false), (2, true), (3, true), (3, false)]);
    }
}
