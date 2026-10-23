//! Selecting a review's lines (`diff_viewer`) with the pointer, as
//! MonoCode's WebKit review allows: a drag, a double or triple click, ⌘C,
//! ⌘A, Esc. It is the transcript's selection (`transcript::selection`)
//! with a line per block: each row of a lane is one segment, and a side by
//! side review selects within one lane, so a copy is that side's code.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Context, Div, HighlightStyle, MouseMoveEvent, SharedString, TextRun, canvas,
};

use crate::app::BenCodeApp;
use crate::ui::transcript::selection::{Joint, SegCtx};

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
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
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
        self.ctx
            .text(0, Joint::Line, false, self.text, runs, cx)
            .element
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
    ) -> SegCtx {
        SegCtx {
            selection: self.diff_selection.clone(),
            focus: self.diff_focus.clone(),
            list: Some(list.clone()),
            session: lane_place(key, lane),
            block: row,
            response: false,
        }
    }

    /// Gives review `key` the keys of its selection while it holds one, and
    /// keeps a drag selecting past the list's edges by scrolling it.
    pub(crate) fn with_diff_selection(
        &self,
        body: Div,
        key: &str,
        list: &gpui::ListState,
        cx: &Context<Self>,
    ) -> Div {
        let owns = self
            .diff_selection
            .read(cx)
            .session()
            .and_then(parse_place)
            .is_some_and(|(k, _)| k == key);
        let scroll = {
            let (selection, list, app, key) = (
                self.diff_selection.clone(),
                list.clone(),
                cx.entity().downgrade(),
                key.to_string(),
            );
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let (selection, list, app, key) =
                        (selection.clone(), list.clone(), app.clone(), key.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Bubble {
                            return;
                        }
                        let dragging_here = selection.read(cx).session().is_some_and(|place| {
                            parse_place(place).is_some_and(|(k, _)| k == key)
                                && selection.read(cx).is_dragging_in(place)
                        });
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
                        if let Err(err) = app.update(cx, |this, cx| this.redraw_diff_doc(&key, cx))
                        {
                            log::debug!("diff selection scroll after app drop: {err:#}");
                        }
                    });
                },
            )
            .absolute()
            .size_full()
        };
        body.when(owns, |el| {
            el.track_focus(&self.diff_focus).key_context("DiffText")
        })
        .child(scroll)
    }

    /// ⌘C: the selected lines as written, rows the list never drew
    /// included.
    pub fn copy_diff_selection(&mut self, cx: &mut Context<Self>) {
        let selection = self.diff_selection.read(cx);
        let Some((key, lane)) = selection.session().and_then(parse_place) else {
            return;
        };
        let rows = self
            .diff_docs
            .get(key)
            .map(|doc| doc.lane_texts(lane))
            .unwrap_or_default();
        match selection.selected_text(&rows) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => log::debug!("diff copy with nothing selected"),
        }
    }

    /// ⌘A: every line of the lane the selection is in.
    pub fn select_all_diff(&mut self, cx: &mut Context<Self>) {
        self.diff_selection.update(cx, |selection, cx| {
            let Some(place) = selection
                .session()
                .map(|s| SharedString::from(s.to_string()))
            else {
                return;
            };
            selection.select_all(&place);
            cx.notify();
        });
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_diff_selection(&mut self, cx: &mut Context<Self>) -> bool {
        self.diff_selection.update(cx, |selection, cx| {
            let cleared = selection.clear();
            if cleared {
                cx.notify();
            }
            cleared
        })
    }

    /// Review `key`'s rows changed (a load, a fold, a layout): what was
    /// selected by row no longer names the same lines. `None`: every review.
    pub(crate) fn forget_diff_selection(&mut self, key: Option<&str>, cx: &mut Context<Self>) {
        self.diff_selection.update(cx, |selection, cx| {
            let gone = |place: &str| {
                parse_place(place).is_some_and(|(k, _)| key.is_none_or(|key| k == key))
            };
            if selection.forget(gone) {
                cx.notify();
            }
        });
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
