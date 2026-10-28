//! Selecting a review's lines (`diff_viewer`) with the pointer, as
//! MonoCode's WebKit review allows: a drag, a double or triple click, ⌘C,
//! ⌘A, Esc. It is the window's selection with a line per block: each lane
//! is a document, each of its rows one segment, so a side by side review
//! selects within one lane and a copy is that side's code.

use std::ops::Range;
use std::rc::Rc;

use bencode_ui::selection::{Document, Joint, TextSelection};
use gpui::prelude::*;
use gpui::{Context, Div, HighlightStyle, SharedString, TextRun};

use crate::app::BenCodeApp;
use crate::ui::selection_area::{self, SelectionArea};
use crate::ui::transcript::selection::SegCtx;

/// Which lane of a review a row's text is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Unified,
    Old,
    New,
}

impl Lane {
    fn name(self) -> &'static str {
        match self {
            Lane::Unified => "unified",
            Lane::Old => "old",
            Lane::New => "new",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [Lane::Unified, Lane::Old, Lane::New]
            .into_iter()
            .find(|lane| lane.name() == name)
    }
}

const PREFIX: &str = "diff:";

/// Where lane `lane` of review `key` registers its text.
fn lane_place(key: &str, lane: Lane) -> SharedString {
    SharedString::from(format!("{PREFIX}{key}:{}", lane.name()))
}

/// The review and lane a place names.
fn parse_place(place: &str) -> Option<(&str, Lane)> {
    let (key, lane) = place.strip_prefix(PREFIX)?.rsplit_once(':')?;
    Some((key, Lane::from_name(lane)?))
}

/// A line's text, selectable as row `ctx.block` of its lane, in the font
/// and ink its box sets; `highlights` are its syntax colours.
#[derive(IntoElement)]
pub(crate) struct LineText {
    ctx: SegCtx,
    text: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

impl LineText {
    pub(crate) fn new(
        ctx: SegCtx,
        text: SharedString,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
    ) -> Self {
        Self {
            ctx,
            text,
            highlights,
        }
    }
}

impl RenderOnce for LineText {
    fn render(self, window: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        let style = window.text_style();
        let base = TextRun {
            len: 0,
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // One pass over the sorted spans: a long line has hundreds of them,
        // and restyling run by run would copy the runs once per span.
        let len = self.text.len();
        let mut runs = Vec::with_capacity(self.highlights.len() * 2 + 1);
        let mut at = 0;
        for (range, highlight) in &self.highlights {
            let Some(color) = highlight.color else {
                continue;
            };
            let (start, end) = (range.start.clamp(at, len), range.end.min(len));
            if start >= end {
                continue;
            }
            if start > at {
                runs.push(TextRun {
                    len: start - at,
                    ..base.clone()
                });
            }
            runs.push(TextRun {
                len: end - start,
                color,
                ..base.clone()
            });
            at = end;
        }
        if at < len {
            runs.push(TextRun {
                len: len - at,
                ..base
            });
        }
        self.ctx.text(0, Joint::Line, self.text, runs).element
    }
}

impl BenCodeApp {
    /// Where row `row` of review `key` draws its text in `lane`.
    pub(crate) fn diff_seg_ctx(
        &self,
        key: &str,
        lane: Lane,
        row: usize,
        list: &gpui::ListState,
        cx: &Context<Self>,
    ) -> SegCtx {
        let (app, redrawn) = (cx.weak_entity(), key.to_string());
        let doc = Document::new(lane_place(key, lane), &self.text_selection)
            .lines()
            .list(list.clone())
            .focus(self.diff_focus.clone())
            .on_change(move |cx| {
                if let Err(err) = app.update(cx, |this, cx| this.redraw_diff_doc(&redrawn, cx)) {
                    log::debug!("diff selection after app drop: {err:#}");
                }
            });
        SegCtx {
            doc,
            block: row,
            quotable: false,
        }
    }

    /// Gives review `key` the keys of its selection while it holds one.
    pub(crate) fn with_diff_selection(&self, body: Div, key: &str, cx: &Context<Self>) -> Div {
        let owned = key.to_string();
        SelectionArea {
            selection: self.text_selection.clone(),
            focus: self.diff_focus.clone(),
            key_context: "DiffText",
            owns: Rc::new(move |doc| parse_place(doc).is_some_and(|(k, _)| k == owned)),
            // Its lanes are documents of their own; a press beside the code
            // would not know which it meant.
            beside: None,
            idle_owner: false,
        }
        .wrap(body, cx)
    }

    /// ⌘C: the selected lines as written, rows the list never drew
    /// included.
    pub fn copy_diff_selection(&mut self, cx: &mut Context<Self>) {
        let doc = self.text_selection.read(cx).doc();
        let Some((key, lane)) = doc.as_deref().and_then(parse_place) else {
            return;
        };
        let rows = (self.diff_docs.get(key))
            .map(|doc| doc.lane_texts(lane))
            .unwrap_or_default();
        selection_area::copy(&self.text_selection, &rows, cx);
    }

    /// ⌘A: every line of the lane the selection is in.
    pub fn select_all_diff(&mut self, cx: &mut Context<Self>) {
        let doc = self.text_selection.read(cx).doc();
        if let Some(doc) = doc.filter(|doc| parse_place(doc).is_some()) {
            TextSelection::select_all(&self.text_selection, &doc, cx);
        }
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_diff_selection(&mut self, cx: &mut Context<Self>) -> bool {
        TextSelection::clear(&self.text_selection, cx)
    }

    /// Review `key`'s rows changed (a load, a fold, a layout): what was
    /// selected by row no longer names the same lines. `None`: every review.
    pub(crate) fn forget_diff_selection(&mut self, key: Option<&str>, cx: &mut Context<Self>) {
        let gone =
            |doc: &str| parse_place(doc).is_some_and(|(k, _)| key.is_none_or(|key| k == key));
        TextSelection::forget(&self.text_selection, gone, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_name_their_review_and_lane() {
        let place = lane_place("commit:/repo:abc", Lane::Old);
        assert_eq!(parse_place(&place), Some(("commit:/repo:abc", Lane::Old)));
        assert_eq!(parse_place("diff:x:sideways"), None);
        assert_eq!(parse_place("bencode-1-2"), None);
    }
}
