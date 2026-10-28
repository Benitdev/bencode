//! What is selected, by place in reading order rather than by where it was
//! painted: a selection outlives its text scrolling out of a virtual list,
//! ⌘A takes a whole document, and a copy reaches text never drawn.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use gpui::SharedString;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Joint {
    /// A new paragraph, heading, quote or code block.
    #[default]
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

/// One run of text as it was last painted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Segment {
    pub text: SharedString,
    pub joint: Joint,
    /// A selection that starts and ends in quotable text of one block can
    /// be quoted (`Model::quote`).
    pub quotable: bool,
}

/// Padding painted inside code chips; never part of what is copied.
const CHIP_PAD: char = '\u{202F}';

/// The window's one selection, in the document it was made in.
#[derive(Default)]
pub(crate) struct Model {
    doc: Option<SharedString>,
    /// The unit the press landed on; a drag extends from it.
    anchor: Option<(Caret, Caret)>,
    head: Option<Caret>,
    unit: Unit,
    dragging: bool,
    /// The press that made the selection was let go.
    released: bool,
    /// The document's blocks are lines (a diff's rows): a copy joins
    /// blocks with a newline, not a blank line.
    lines: bool,
    /// Every segment drawn, per document, for copying and word bounds.
    segments: HashMap<SharedString, BTreeMap<SegKey, Segment>>,
}

impl Model {
    pub fn register(&mut self, doc: &SharedString, key: SegKey, segment: Segment) {
        let segments = self.segments.entry(doc.clone()).or_default();
        if segments.get(&key) != Some(&segment) {
            segments.insert(key, segment);
        }
    }

    /// Drops the text of documents neither drawn nor selected.
    pub fn retain_docs(&mut self, drawn: impl Fn(&SharedString) -> bool) {
        let selected = self.doc.clone();
        self.segments
            .retain(|doc, _| drawn(doc) || selected.as_ref() == Some(doc));
    }

    /// Makes `doc` the selection's document.
    fn enter(&mut self, doc: &SharedString, lines: bool) {
        if self.doc.as_ref() != Some(doc) {
            self.doc = Some(doc.clone());
        }
        self.lines = lines;
    }

    pub fn segment(&self, doc: &str, key: SegKey) -> Option<&Segment> {
        self.segments.get(doc)?.get(&key)
    }

    fn own_segment(&self, key: SegKey) -> Option<&Segment> {
        self.segment(self.doc.as_ref()?, key)
    }

    /// The unit around `caret`, as two carets.
    fn unit_at(&self, caret: Caret) -> (Caret, Caret) {
        let Some(text) = self.own_segment(caret.key).map(|s| s.text.as_ref()) else {
            return (caret, caret);
        };
        let range = match self.unit {
            Unit::Char => caret.ix..caret.ix,
            Unit::Word => word_at(text, caret.ix),
            Unit::Line => line_at(text, caret.ix),
        };
        let at = |ix| Caret { key: caret.key, ix };
        (at(range.start), at(range.end))
    }

    /// The selection's ends in reading order; `None` when nothing is
    /// selected.
    pub fn span(&self) -> Option<(Caret, Caret)> {
        let (from, to) = self.anchor?;
        let (head_from, head_to) = self.unit_at(self.head?);
        let (start, end) = (from.min(head_from), to.max(head_to));
        (start < end).then_some((start, end))
    }

    pub fn doc(&self) -> Option<&SharedString> {
        self.doc.as_ref()
    }

    pub fn has_selection(&self) -> bool {
        self.span().is_some()
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn released(&self) -> bool {
        self.released && !self.dragging
    }

    /// The part of segment `key` (`len` bytes) of `doc` that is selected.
    pub fn range_in(&self, doc: &str, key: SegKey, len: usize) -> Option<Range<usize>> {
        if self.doc.as_deref() != Some(doc) {
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

    /// Whether the selection goes on past segment `key` of `doc`.
    pub fn continues_past(&self, doc: &str, key: SegKey) -> bool {
        self.doc.as_deref() == Some(doc) && self.span().is_some_and(|(_, end)| end.key > key)
    }

    /// A press at `caret` of `doc`: `clicks` picks the unit; `extend`
    /// (shift) moves the head of the selection already there.
    pub fn press(
        &mut self,
        doc: &SharedString,
        lines: bool,
        caret: Caret,
        clicks: usize,
        extend: bool,
    ) {
        self.released = false;
        self.dragging = true;
        if extend && self.doc.as_ref() == Some(doc) && self.anchor.is_some() {
            self.head = Some(caret);
            return;
        }
        self.enter(doc, lines);
        self.unit = match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        self.anchor = Some(self.unit_at(caret));
        self.head = Some(caret);
    }

    /// The drag moved to `caret`; `true` when that changed the selection.
    pub fn drag_to(&mut self, caret: Caret) -> bool {
        if !self.dragging || self.head == Some(caret) {
            return false;
        }
        let before = self.span();
        self.head = Some(caret);
        self.span() != before
    }

    /// The press ended; `true` when one was going on.
    pub fn release(&mut self) -> bool {
        if !self.dragging {
            return false;
        }
        self.dragging = false;
        self.released = true;
        true
    }

    /// The selection stays but is no longer offered for quoting (the text
    /// moved out from under its menu); `true` when it was.
    pub fn dismiss(&mut self) -> bool {
        std::mem::take(&mut self.released) && self.has_selection()
    }

    /// `true` when there was a selection or a press to drop.
    pub fn clear(&mut self) -> bool {
        let had = self.anchor.is_some();
        self.anchor = None;
        self.head = None;
        self.dragging = false;
        self.released = false;
        had
    }

    /// Drops the text of every document `gone` names, and the selection if
    /// it was made there: their blocks changed under it. `true` when that
    /// cleared a selection.
    pub fn forget(&mut self, gone: impl Fn(&str) -> bool) -> bool {
        self.segments.retain(|doc, _| !gone(doc));
        if !self.doc.as_deref().is_some_and(&gone) {
            return false;
        }
        self.doc = None;
        self.clear()
    }

    /// ⌘A: the whole of `doc`.
    pub fn select_all(&mut self, doc: &SharedString, lines: bool) {
        self.enter(doc, lines);
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
        self.released = false;
    }

    /// The selected text as shown. `fallback` gives the text of blocks the
    /// document has not drawn (a virtual list's), used only for blocks
    /// wholly inside the selection.
    pub fn selected_text(&self, fallback: &[(usize, String)]) -> Option<String> {
        let (start, end) = self.span()?;
        let doc = self.doc.as_ref()?;
        let empty = BTreeMap::new();
        let segments = self.segments.get(doc).unwrap_or(&empty);
        let mut pieces: BTreeMap<SegKey, (Joint, String)> = BTreeMap::new();
        for (key, segment) in segments.range(start.key..=end.key) {
            if let Some(range) = self.range_in(doc, *key, segment.text.len()) {
                let text: String = segment.text[range]
                    .chars()
                    .filter(|c| *c != CHIP_PAD)
                    .collect();
                pieces.insert(*key, (segment.joint, text));
            } else if self.lines && *key > start.key && *key < end.key {
                // A blank line inside the selection is still a line.
                pieces.insert(*key, (segment.joint, String::new()));
            }
        }
        for (block, text) in fallback {
            let keys = SegKey::block(*block);
            let whole = *keys.start() >= start.key && *keys.end() <= end.key;
            let drawn = segments.range(keys.clone()).next().is_some();
            if whole && !drawn && (self.lines || !text.is_empty()) {
                pieces.insert(*keys.start(), (Joint::Block, text.clone()));
            }
        }
        let between = if self.lines {
            Joint::Line
        } else {
            Joint::Block
        };
        let mut out = String::new();
        let mut previous: Option<usize> = None;
        for (key, (joint, text)) in pieces {
            if text.is_empty() && !self.lines {
                continue;
            }
            if let Some(block) = previous {
                out.push_str(if block == key.block {
                    joint.separator()
                } else {
                    between.separator()
                });
            }
            out.push_str(&text);
            previous = Some(key.block);
        }
        (!out.is_empty()).then_some(out)
    }

    /// A selection inside the quotable text of one block, trimmed (MonoCode
    /// `validateTranscriptSelection`).
    pub fn quote(&self) -> Option<String> {
        let (start, end) = self.span()?;
        if start.key.block != end.key.block {
            return None;
        }
        if !self.own_segment(start.key)?.quotable || !self.own_segment(end.key)?.quotable {
            return None;
        }
        let text = self.selected_text(&[])?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
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

    fn with(segments: &[(usize, usize, &str, Joint, bool)]) -> (Model, SharedString) {
        let doc = SharedString::from("s");
        let mut model = Model::default();
        for (block, seq, text, joint, quotable) in segments {
            model.register(
                &doc,
                key(*block, *seq),
                Segment {
                    text: SharedString::from(text.to_string()),
                    joint: *joint,
                    quotable: *quotable,
                },
            );
        }
        (model, doc)
    }

    #[test]
    fn a_drag_spans_blocks_and_copies_as_shown() {
        let (mut sel, s) = with(&[
            (0, 0, "Fix the bug", Joint::Block, true),
            (1, 0, "First paragraph", Joint::Block, true),
            (1, 1, "one", Joint::Block, true),
            (1, 2, "two", Joint::Line, true),
        ]);
        sel.press(&s, false, caret(0, 0, 4), 1, false);
        assert!(sel.drag_to(caret(1, 2, 2)));
        assert!(sel.release());
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("the bug\n\nFirst paragraph\n\none\ntw")
        );
        assert_eq!(sel.range_in("s", key(1, 0), 15), Some(0..15));
        assert_eq!(sel.range_in("s", key(0, 0), 11), Some(4..11));
        assert_eq!(sel.range_in("other", key(0, 0), 11), None);
        assert!(sel.continues_past("s", key(1, 1)));
        assert!(!sel.continues_past("s", key(1, 2)));
        // Across two blocks: nothing to quote.
        assert_eq!(sel.quote(), None);
    }

    #[test]
    fn a_backwards_drag_selects_the_same_text() {
        let (mut sel, s) = with(&[(0, 0, "hello world", Joint::Block, true)]);
        sel.press(&s, false, caret(0, 0, 8), 1, false);
        sel.drag_to(caret(0, 0, 2));
        sel.release();
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("llo wo"));
        assert_eq!(sel.quote().as_deref(), Some("llo wo"));
        assert!(sel.released());
    }

    #[test]
    fn a_click_selects_nothing() {
        let (mut sel, s) = with(&[(0, 0, "hello", Joint::Block, true)]);
        sel.press(&s, false, caret(0, 0, 2), 1, false);
        assert!(!sel.drag_to(caret(0, 0, 2)));
        sel.release();
        assert!(!sel.has_selection());
        assert_eq!(sel.quote(), None);
    }

    #[test]
    fn double_and_triple_clicks_take_a_word_and_a_line() {
        let (mut sel, s) = with(&[(0, 0, "let x = foo_bar(1);\nnext line", Joint::Block, true)]);
        sel.press(&s, false, caret(0, 0, 10), 2, false);
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("foo_bar"));
        // Dragging on by words keeps whole words.
        sel.drag_to(caret(0, 0, 21));
        assert_eq!(sel.selected_text(&[]).as_deref(), Some("foo_bar(1);\nnext"));
        sel.press(&s, false, caret(0, 0, 3), 3, false);
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
        sel.press(&s, false, caret(0, 0, 0), 1, false);
        sel.drag_to(caret(0, 0, 5));
        sel.release();
        sel.press(&s, false, caret(0, 0, 11), 1, true);
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
        sel.select_all(&s, false);
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("Run cargo now\n\na\tb\nc")
        );
        // Not quotable: nothing to quote.
        assert_eq!(sel.quote(), None);
    }

    #[test]
    fn select_all_fills_blocks_never_drawn() {
        let (mut sel, s) = with(&[
            (0, 0, "first", Joint::Block, true),
            (2, 0, "third", Joint::Block, true),
        ]);
        sel.select_all(&s, false);
        let fallback = vec![(0, "raw first".to_string()), (1, "second".to_string())];
        assert_eq!(
            sel.selected_text(&fallback).as_deref(),
            Some("first\n\nsecond\n\nthird")
        );
    }

    #[test]
    fn line_blocks_copy_as_lines_blank_ones_included() {
        let doc = SharedString::from("d");
        let mut sel = Model::default();
        for (block, text) in [(0, "fn a() {"), (1, ""), (3, "}")] {
            sel.register(
                &doc,
                key(block, 0),
                Segment {
                    text: SharedString::from(text),
                    joint: Joint::Line,
                    quotable: false,
                },
            );
        }
        sel.select_all(&doc, true);
        let fallback = vec![(2, "    body".to_string())];
        assert_eq!(
            sel.selected_text(&fallback).as_deref(),
            Some("fn a() {\n\n    body\n}")
        );
        sel.press(&doc, true, caret(0, 0, 3), 1, false);
        sel.drag_to(caret(3, 0, 1));
        assert_eq!(
            sel.selected_text(&fallback).as_deref(),
            Some("a() {\n\n    body\n}")
        );
    }

    #[test]
    fn another_document_starts_afresh() {
        let (mut sel, s) = with(&[(0, 0, "hello", Joint::Block, true)]);
        sel.select_all(&s, false);
        let other = SharedString::from("t");
        sel.press(&other, false, caret(0, 0, 0), 1, false);
        assert_eq!(sel.doc().map(|d| d.as_ref()), Some("t"));
        assert!(!sel.has_selection());
        // The old document's text goes once it is neither drawn nor selected.
        sel.retain_docs(|doc| doc == "t");
        assert!(sel.segment("s", key(0, 0)).is_none());
    }

    #[test]
    fn forgetting_a_document_drops_its_selection() {
        let (mut sel, s) = with(&[(0, 0, "hello", Joint::Block, true)]);
        sel.select_all(&s, false);
        assert!(!sel.forget(|doc| doc == "other"));
        assert!(sel.has_selection());
        assert!(sel.forget(|doc| doc == "s"));
        assert!(!sel.has_selection());
        assert_eq!(sel.doc(), None);
    }
}
