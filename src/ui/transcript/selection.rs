//! Selecting transcript text. MonoCode's transcript is `user-select: text`
//! (`.agent-transcript` in `src/styles/index.css`), so WebKit lets a drag
//! select across messages, a double click take a word, a triple click a
//! block, and ⌘C copy the text as shown. GPUI text has none of that, so
//! every run of transcript text registers here as a segment in reading
//! order, and one selection spans them.
//!
//! A selection inside one settled answer or one prompt also offers
//! MonoCode's `TranscriptSelectionMenu` (`useTranscriptSelection.ts`):
//! Add to chat, Add to notes.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::rc::Rc;

use ely_gpui_component::theme::ActiveTheme;
use gpui::prelude::*;
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Div, Entity, FocusHandle, Hsla, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, Stateful, StyledText,
    TextLayout, TextRun, canvas, div, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

/// Where a run of text sits in reading order: the block it draws, then its
/// place among that block's runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegKey {
    pub block: usize,
    pub seq: usize,
}

impl SegKey {
    const FIRST: Self = Self { block: 0, seq: 0 };
    const LAST: Self = Self {
        block: usize::MAX,
        seq: usize::MAX,
    };

    /// Every key of `block`.
    fn block(block: usize) -> std::ops::RangeInclusive<Self> {
        Self { block, seq: 0 }..=Self {
            block,
            seq: usize::MAX,
        }
    }
}

/// A position between two characters of a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub key: SegKey,
    pub ix: usize,
}

/// What a press selects: a caret, its word (double click) or its line of
/// the block (triple click, as WebKit does inside one block).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Unit {
    #[default]
    Char,
    Word,
    Line,
}

/// What a copy puts between a segment and the one before it in its block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    /// A new paragraph, heading, quote or code block.
    Block,
    /// The next list item or table row.
    Line,
    /// The next table cell.
    Cell,
}

impl Joint {
    fn separator(self) -> &'static str {
        match self {
            Joint::Block => "\n\n",
            Joint::Line => "\n",
            Joint::Cell => "\t",
        }
    }
}

/// One run of text as it was painted.
#[derive(Clone, Debug, PartialEq)]
struct Segment {
    text: SharedString,
    joint: Joint,
    /// Part of a settled answer or a prompt (MonoCode
    /// `data-selectable-agent-response`): selections inside one get the menu.
    response: bool,
}

/// Where something was last painted, read on the next frame.
type PaintedAt = Rc<Cell<Option<Bounds<Pixels>>>>;

/// Padding painted inside code chips; never part of what is copied.
const CHIP_PAD: char = '\u{202F}';

/// The transcript's one selection, in the thread it was made in.
#[derive(Default)]
pub struct TranscriptSelection {
    session: Option<SharedString>,
    /// The unit the press landed on; a drag extends from it.
    anchor: Option<(Caret, Caret)>,
    head: Option<Caret>,
    unit: Unit,
    dragging: bool,
    menu_open: bool,
    /// Every segment drawn, per thread, for copying and word bounds.
    segments: HashMap<SharedString, BTreeMap<SegKey, Segment>>,
    /// The selection's first line as last painted: the menu sits over it.
    first_line: PaintedAt,
    /// The menu as last painted: a press there keeps the selection.
    menu_bounds: PaintedAt,
    /// Why Add to notes failed, shown in the menu (MonoCode).
    note_error: Option<String>,
}

impl TranscriptSelection {
    fn register(&mut self, session: &SharedString, key: SegKey, segment: Segment) {
        self.segments
            .entry(session.clone())
            .or_default()
            .insert(key, segment);
    }

    /// Makes `session` the selection's thread; only its text is kept.
    fn enter(&mut self, session: &SharedString) {
        if self.session.as_ref() != Some(session) {
            self.segments.retain(|id, _| id == session);
            self.session = Some(session.clone());
        }
    }

    fn segment(&self, key: SegKey) -> Option<&Segment> {
        self.segments.get(self.session.as_ref()?)?.get(&key)
    }

    /// The unit around `caret`, as two carets.
    fn unit_at(&self, caret: Caret) -> (Caret, Caret) {
        let Some(text) = self.segment(caret.key).map(|s| s.text.as_ref()) else {
            return (caret, caret);
        };
        let range = match self.unit {
            Unit::Char => caret.ix..caret.ix,
            Unit::Word => word_at(text, caret.ix),
            Unit::Line => line_at(text, caret.ix),
        };
        (
            Caret {
                key: caret.key,
                ix: range.start,
            },
            Caret {
                key: caret.key,
                ix: range.end,
            },
        )
    }

    /// The selection's ends in reading order; `None` when nothing is
    /// selected.
    fn span(&self) -> Option<(Caret, Caret)> {
        let (from, to) = self.anchor?;
        let (head_from, head_to) = self.unit_at(self.head?);
        let (start, end) = (from.min(head_from), to.max(head_to));
        (start < end).then_some((start, end))
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn has_selection(&self) -> bool {
        self.span().is_some()
    }

    /// The part of segment `key` (`len` bytes) that is selected.
    fn range_in(&self, session: &str, key: SegKey, len: usize) -> Option<Range<usize>> {
        if self.session.as_deref() != Some(session) {
            return None;
        }
        let (start, end) = self.span()?;
        if key < start.key || key > end.key {
            return None;
        }
        let from = if key == start.key {
            start.ix.min(len)
        } else {
            0
        };
        let to = if key == end.key { end.ix.min(len) } else { len };
        (from < to).then_some(from..to)
    }

    /// A press at `caret`: `clicks` picks the unit; `extend` (shift) moves
    /// the head of the selection already there.
    fn press(&mut self, session: &SharedString, caret: Caret, clicks: usize, extend: bool) {
        self.close_menu();
        self.dragging = true;
        if extend && self.session.as_ref() == Some(session) && self.anchor.is_some() {
            self.head = Some(caret);
            return;
        }
        self.enter(session);
        self.unit = match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        self.anchor = Some(self.unit_at(caret));
        self.head = Some(caret);
    }

    fn drag_to(&mut self, caret: Caret) -> bool {
        if !self.dragging || self.head == Some(caret) {
            return false;
        }
        self.head = Some(caret);
        true
    }

    /// The press ended: MonoCode reports the selection once the pointer is
    /// up, so the menu opens now.
    fn release(&mut self) -> bool {
        if !self.dragging {
            return false;
        }
        self.dragging = false;
        self.menu_open = self.menu_text().is_some();
        true
    }

    pub fn clear(&mut self) -> bool {
        let had = self.anchor.is_some();
        self.anchor = None;
        self.head = None;
        self.dragging = false;
        self.close_menu();
        had
    }

    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_bounds.set(None);
        self.note_error = None;
    }

    /// ⌘A: the whole thread.
    pub fn select_all(&mut self, session: &SharedString) {
        self.enter(session);
        self.unit = Unit::Char;
        let first = Caret {
            key: SegKey::FIRST,
            ix: 0,
        };
        self.anchor = Some((first, first));
        self.head = Some(Caret {
            key: SegKey::LAST,
            ix: usize::MAX,
        });
        self.dragging = false;
        self.close_menu();
    }

    /// Scrolling moves the text out from under the menu (MonoCode closes
    /// it and keeps the selection).
    pub fn dismiss_menu(&mut self) -> bool {
        let open = self.menu_open;
        self.close_menu();
        open
    }

    pub fn set_note_error(&mut self, error: Option<String>) {
        self.note_error = error;
    }

    pub fn note_error(&self) -> Option<&str> {
        self.note_error.as_deref()
    }

    pub fn is_dragging_in(&self, session: &str) -> bool {
        self.dragging && self.session.as_deref() == Some(session)
    }

    /// The selected text as shown. `fallback` gives the text of blocks the
    /// list has not drawn (it is virtualized; MonoCode's DOM holds every
    /// turn), used only for blocks wholly inside the selection.
    pub fn selected_text(&self, fallback: &[(usize, String)]) -> Option<String> {
        let (start, end) = self.span()?;
        let session = self.session.as_ref()?;
        let segments = self.segments.get(session)?;
        let mut pieces: BTreeMap<SegKey, (Joint, String)> = BTreeMap::new();
        for (key, segment) in segments.range(start.key..=end.key) {
            if let Some(range) = self.range_in(session, *key, segment.text.len()) {
                let text: String = segment.text[range]
                    .chars()
                    .filter(|c| *c != CHIP_PAD)
                    .collect();
                pieces.insert(*key, (segment.joint, text));
            }
        }
        for (block, text) in fallback {
            let keys = SegKey::block(*block);
            let whole = *keys.start() >= start.key && *keys.end() <= end.key;
            let drawn = segments.range(keys.clone()).next().is_some();
            if whole && !drawn && !text.is_empty() {
                pieces.insert(*keys.start(), (Joint::Block, text.clone()));
            }
        }
        let mut out = String::new();
        let mut previous: Option<usize> = None;
        for (key, (joint, text)) in pieces {
            if text.is_empty() {
                continue;
            }
            if let Some(block) = previous {
                out.push_str(if block == key.block {
                    joint.separator()
                } else {
                    Joint::Block.separator()
                });
            }
            out.push_str(&text);
            previous = Some(key.block);
        }
        (!out.is_empty()).then_some(out)
    }

    /// What the menu acts on: a selection inside one settled answer or one
    /// prompt, trimmed (MonoCode `validateTranscriptSelection`).
    pub fn menu_text(&self) -> Option<String> {
        let (start, end) = self.span()?;
        if start.key.block != end.key.block {
            return None;
        }
        if !self.segment(start.key)?.response || !self.segment(end.key)?.response {
            return None;
        }
        let text = self.selected_text(&[])?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// The menu's text and where it goes, when it is open over `session`.
    pub fn menu(&self, session: &str) -> Option<(String, Bounds<Pixels>)> {
        if !self.menu_open || self.dragging || self.session.as_deref() != Some(session) {
            return None;
        }
        Some((self.menu_text()?, self.first_line.get()?))
    }

    pub fn menu_bounds(&self) -> PaintedAt {
        self.menu_bounds.clone()
    }
}

/// Floors `ix` to a character boundary of `text`.
fn floor_boundary(text: &str, ix: usize) -> usize {
    let mut ix = ix.min(text.len());
    while !text.is_char_boundary(ix) {
        ix -= 1;
    }
    ix
}

/// The word (or run of spaces or punctuation) at `ix`. A caret just past a
/// word takes that word, as the pointer was over its last letter.
fn word_at(text: &str, ix: usize) -> Range<usize> {
    let ix = floor_boundary(text, ix);
    let mut previous: Option<Range<usize>> = None;
    for (start, piece) in text.split_word_bound_indices() {
        let range = start..start + piece.len();
        if ix < range.end {
            let is_word = |r: &Range<usize>| text[r.clone()].chars().any(char::is_alphanumeric);
            return match previous {
                Some(prev) if ix == range.start && !is_word(&range) && is_word(&prev) => prev,
                _ => range,
            };
        }
        previous = Some(range);
    }
    previous.unwrap_or(ix..ix)
}

/// The line of a segment at `ix`, without its newline.
fn line_at(text: &str, ix: usize) -> Range<usize> {
    let ix = floor_boundary(text, ix);
    let start = text[..ix].rfind('\n').map_or(0, |at| at + 1);
    let end = text[ix..].find('\n').map_or(text.len(), |at| ix + at);
    start..end
}

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

/// Where transcript text is drawn: the selection, its thread, its block,
/// and whether that block is a settled answer or a prompt.
#[derive(Clone)]
pub struct SegCtx {
    pub selection: Entity<TranscriptSelection>,
    pub focus: FocusHandle,
    /// The thread's list: a drag past its edges selects to the edge.
    pub list: Option<gpui::ListState>,
    pub session: SharedString,
    pub block: usize,
    pub response: bool,
}

/// A run of transcript text the pointer can select; `layout` locates
/// characters for the caller's own clicks (links).
pub struct Selectable {
    pub element: Stateful<Div>,
    pub layout: TextLayout,
}

fn caret_at(layout: &TextLayout, key: SegKey, position: gpui::Point<Pixels>) -> Caret {
    let ix = match layout.index_for_position(position) {
        Ok(ix) | Err(ix) => ix,
    };
    Caret { key, ix }
}

impl SegCtx {
    /// `text` with `runs`, selectable as segment `seq` of the block.
    /// `cell` marks text side by side with other segments (table cells): a
    /// drag reaches it only over its own column.
    pub fn text(
        &self,
        seq: usize,
        joint: Joint,
        cell: bool,
        text: String,
        runs: Vec<TextRun>,
        cx: &mut App,
    ) -> Selectable {
        let key = SegKey {
            block: self.block,
            seq,
        };
        let text = SharedString::from(text);
        let (range, starts_here) = {
            let selection = self.selection.read(cx);
            let range = selection.range_in(&self.session, key, text.len());
            let starts_here = selection
                .span()
                .filter(|(start, _)| range.is_some() && start.key == key)
                .map(|span| (span, selection.first_line.clone()));
            (range, starts_here)
        };
        self.selection.update(cx, |selection, _| {
            selection.register(
                &self.session,
                key,
                Segment {
                    text: text.clone(),
                    joint,
                    response: self.response,
                },
            )
        });
        let mut runs = runs;
        if let Some(range) = &range {
            let color = cx.theme().colors.selection;
            restyle(&mut runs, range, |run| run.background_color = Some(color));
        }
        let styled = StyledText::new(text).with_runs(runs);
        let layout = styled.layout().clone();

        let id = SharedString::from(format!("sel-{}-{}-{seq}", self.session, self.block));
        let press = {
            let (ctx, layout) = (self.clone(), layout.clone());
            move |event: &MouseDownEvent, window: &mut gpui::Window, cx: &mut App| {
                let caret = caret_at(&layout, key, event.position);
                window.focus(&ctx.focus, cx);
                ctx.selection.update(cx, |selection, cx| {
                    selection.press(
                        &ctx.session,
                        caret,
                        event.click_count,
                        event.modifiers.shift,
                    );
                    cx.notify();
                });
            }
        };
        let painted = Painted {
            ctx: self.clone(),
            key,
            cell,
            layout: layout.clone(),
        };
        let listen = move |bounds, (), window: &mut gpui::Window, _: &mut App| {
            if let Some((span, first_line)) = &starts_here {
                painted.note_first_line(*span, bounds, first_line);
            }
            painted.listen(bounds, window);
        };
        let element = div()
            .id(id)
            .relative()
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, press)
            .child(styled)
            .child(
                canvas(|_, _, _| (), listen)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            );
        Selectable { element, layout }
    }
}

/// A segment as painted this frame, for the listeners it adds.
struct Painted {
    ctx: SegCtx,
    key: SegKey,
    /// Side by side with other segments (a table cell): a drag reaches it
    /// only over its own column.
    cell: bool,
    layout: TextLayout,
}

impl Painted {
    /// Records the selection's first line, which starts in this segment,
    /// for the menu above it.
    fn note_first_line(
        &self,
        (start, end): (Caret, Caret),
        bounds: Bounds<Pixels>,
        at: &PaintedAt,
    ) {
        let Some(from) = self.layout.position_for_index(start.ix) else {
            return;
        };
        let to = (end.key == self.key)
            .then(|| self.layout.position_for_index(end.ix))
            .flatten()
            .filter(|to| to.y == from.y)
            .map_or(bounds.right(), |to| to.x);
        let line = size((to - from.x).max(px(1.0)), self.layout.line_height());
        at.set(Some(Bounds::new(from, line)));
    }

    /// Adds this frame's listeners: drags across the segment, the release,
    /// and presses elsewhere that drop the selection.
    fn listen(&self, bounds: Bounds<Pixels>, window: &mut gpui::Window) {
        let (ctx, key, cell, layout) = (self.ctx.clone(), self.key, self.cell, self.layout.clone());
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble
                || !ctx.selection.read(cx).is_dragging_in(&ctx.session)
            {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left) {
                // The release happened outside the window.
                ctx.selection.update(cx, |selection, cx| {
                    if selection.release() {
                        cx.notify();
                    }
                });
                return;
            }
            // Past the list's edges, the drag selects to the edge row.
            let mut at = event.position;
            if let Some(view) = ctx.list.as_ref().map(|list| list.viewport_bounds())
                && view.size.height > px(1.0)
            {
                at.y = at.y.clamp(view.top(), view.bottom() - px(1.0));
            }
            let rows = at.y >= bounds.top() && at.y < bounds.bottom();
            let column = at.x >= bounds.left() && at.x < bounds.right();
            if !rows || (cell && !column) {
                return;
            }
            let caret = caret_at(&layout, key, at);
            ctx.selection.update(cx, |selection, cx| {
                if selection.drag_to(caret) {
                    cx.notify();
                }
            });
        });

        let selection = self.ctx.selection.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                selection.update(cx, |selection, cx| {
                    if selection.release() {
                        cx.notify();
                    }
                });
            }
        });

        // A press anywhere drops the selection before the press lands, so
        // one on text starts a new one; the menu's own buttons keep it.
        let selection = self.ctx.selection.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture
                || event.button != MouseButton::Left
                || event.modifiers.shift
            {
                return;
            }
            let on_menu = selection
                .read(cx)
                .menu_bounds
                .get()
                .is_some_and(|menu| menu.contains(&event.position));
            if !on_menu {
                selection.update(cx, |selection, cx| {
                    if selection.clear() {
                        cx.notify();
                    }
                });
            }
        });
    }
}

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
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
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
        self.ctx
            .text(0, Joint::Block, false, self.text, runs, cx)
            .element
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(block: usize, seq: usize) -> SegKey {
        SegKey { block, seq }
    }

    fn caret(block: usize, seq: usize, ix: usize) -> Caret {
        Caret {
            key: key(block, seq),
            ix,
        }
    }

    fn with(segments: &[(usize, usize, &str, Joint, bool)]) -> (TranscriptSelection, SharedString) {
        let session = SharedString::from("s");
        let mut selection = TranscriptSelection::default();
        for (block, seq, text, joint, response) in segments {
            selection.register(
                &session,
                key(*block, *seq),
                Segment {
                    text: SharedString::from(text.to_string()),
                    joint: *joint,
                    response: *response,
                },
            );
        }
        (selection, session)
    }

    #[test]
    fn a_drag_spans_blocks_and_copies_as_shown() {
        let (mut sel, s) = with(&[
            (0, 0, "Fix the bug", Joint::Block, true),
            (1, 0, "First paragraph", Joint::Block, true),
            (1, 1, "one", Joint::Block, true),
            (1, 2, "two", Joint::Line, true),
        ]);
        sel.press(&s, caret(0, 0, 4), 1, false);
        assert!(sel.drag_to(caret(1, 2, 2)));
        assert!(sel.release());
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("the bug\n\nFirst paragraph\n\none\ntw")
        );
        assert_eq!(sel.range_in("s", key(1, 0), 15), Some(0..15));
        assert_eq!(sel.range_in("s", key(0, 0), 11), Some(4..11));
        assert_eq!(sel.range_in("other", key(0, 0), 11), None);
        // Across two blocks: no menu.
        assert_eq!(sel.menu_text(), None);
    }

    #[test]
    fn a_backwards_drag_selects_the_same_text() {
        let (mut sel, s) = with(&[(0, 0, "hello world", Joint::Block, true)]);
        sel.press(&s, caret(0, 0, 8), 1, false);
        sel.drag_to(caret(0, 0, 2));
        sel.release();
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("llo wo"));
        assert_eq!(sel.menu_text().as_deref(), Some("llo wo"));
    }

    #[test]
    fn a_click_selects_nothing_and_opens_no_menu() {
        let (mut sel, s) = with(&[(0, 0, "hello", Joint::Block, true)]);
        sel.press(&s, caret(0, 0, 2), 1, false);
        sel.release();
        assert!(!sel.has_selection());
        assert_eq!(sel.menu("s"), None);
    }

    #[test]
    fn double_and_triple_clicks_take_a_word_and_a_line() {
        let (mut sel, s) = with(&[(0, 0, "let x = foo_bar(1);\nnext line", Joint::Block, true)]);
        sel.press(&s, caret(0, 0, 10), 2, false);
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("foo_bar"));
        // Dragging on by words keeps whole words.
        sel.drag_to(caret(0, 0, 21));
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("foo_bar(1);\nnext"));
        sel.press(&s, caret(0, 0, 3), 3, false);
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("let x = foo_bar(1);")
        );
    }

    #[test]
    fn a_word_is_the_one_the_caret_closes() {
        assert_eq!(word_at("hello world", 5), 0..5);
        assert_eq!(word_at("hello world", 6), 6..11);
        assert_eq!(word_at("hello world", 11), 6..11);
        assert_eq!(word_at("a  b", 2), 1..3);
    }

    #[test]
    fn shift_extends_the_selection() {
        let (mut sel, s) = with(&[(0, 0, "hello world", Joint::Block, true)]);
        sel.press(&s, caret(0, 0, 0), 1, false);
        sel.drag_to(caret(0, 0, 5));
        sel.release();
        sel.press(&s, caret(0, 0, 11), 1, true);
        sel.release();
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("hello world"));
    }

    #[test]
    fn copies_drop_chip_padding_and_join_cells() {
        let (mut sel, s) = with(&[
            (0, 0, "Run \u{202F}cargo\u{202F} now", Joint::Block, false),
            (0, 1, "a", Joint::Block, false),
            (0, 2, "b", Joint::Cell, false),
            (0, 3, "c", Joint::Line, false),
        ]);
        sel.select_all(&s);
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("Run cargo now\n\na\tb\nc")
        );
        // Not a response: no menu.
        assert_eq!(sel.menu_text(), None);
    }

    #[test]
    fn select_all_fills_blocks_the_list_never_drew() {
        let (mut sel, s) = with(&[
            (0, 0, "first", Joint::Block, true),
            (2, 0, "third", Joint::Block, true),
        ]);
        sel.select_all(&s);
        let fallback = vec![(0, "raw first".to_string()), (1, "second".to_string())];
        assert_eq!(
            sel.selected_text(&fallback).as_deref(),
            Some("first\n\nsecond\n\nthird")
        );
    }

    #[test]
    fn a_new_thread_starts_afresh() {
        let (mut sel, s) = with(&[(0, 0, "hello", Joint::Block, true)]);
        sel.select_all(&s);
        let other = SharedString::from("t");
        sel.press(&other, caret(0, 0, 0), 1, false);
        assert_eq!(sel.session(), Some("t"));
        assert!(!sel.has_selection());
        assert!(!sel.segments.contains_key(&s));
    }

    #[test]
    fn selection_paints_over_split_runs() {
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
