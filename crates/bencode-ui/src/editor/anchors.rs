use std::ops::Range;

use super::state::{CodeEditor, Marks};

/// Where `offset` lands after `edits`, sorted and apart, each a replaced range and the length written; none when an edit replaced it.
fn moved(offset: usize, edits: &[(Range<usize>, usize)]) -> Option<usize> {
    let mut at = offset as isize;
    for (range, written) in edits {
        if offset <= range.start {
            break;
        }
        if offset < range.end {
            return None;
        }
        at += *written as isize - range.len() as isize;
    }
    Some(at as usize)
}

/// Where the character at `offset` lands; text written at it goes before.
fn carried(offset: usize, edits: &[(Range<usize>, usize)]) -> Option<usize> {
    let mut at = offset as isize;
    for (range, written) in edits {
        if offset < range.start {
            break;
        }
        if offset < range.end {
            return None;
        }
        at += *written as isize - range.len() as isize;
    }
    Some(at as usize)
}

/// A range through `edits`; none when an edit cut into it.
fn moved_range(range: &Range<usize>, edits: &[(Range<usize>, usize)]) -> Option<Range<usize>> {
    Some(moved(range.start, edits)?..moved(range.end, edits)?)
}

/// Carries the owner's offsets through an edit of the text, dropping what it replaced.
pub(crate) fn shift(marks: &mut Marks, edits: &[(Range<usize>, usize)]) {
    marks.diagnostics.retain_mut(|diagnostic| {
        moved_range(&diagnostic.range, edits)
            .map(|range| diagnostic.range = range)
            .is_some()
    });
    marks.backgrounds.retain_mut(|(range, _)| {
        moved_range(range, edits)
            .map(|moved| *range = moved)
            .is_some()
    });
    marks.hints.retain_mut(|hint| {
        moved(hint.offset, edits)
            .map(|at| hint.offset = at)
            .is_some()
    });
    marks.ghost = marks.ghost.take().and_then(|mut ghost| {
        ghost.offset = moved(ghost.offset, edits)?;
        Some(ghost)
    });
    for anchor in &mut marks.anchors {
        *anchor = anchor.and_then(|at| carried(at, edits));
    }
}

/// Drops the owner's offsets when the whole text changes; they named the old one.
pub(crate) fn clear(marks: &mut Marks) {
    marks.diagnostics.clear();
    marks.backgrounds.clear();
    marks.hints.clear();
    marks.ghost = None;
    marks.anchors.fill(None);
}

impl CodeEditor {}

#[cfg(test)]
mod tests {
    use super::carried;

    #[test]
    fn anchors_go_after_text_written_at_them() {
        let edits = [(2..4, 1), (6..6, 3)];
        assert_eq!(carried(2, &edits), None, "its character replaced");
        assert_eq!(carried(4, &edits), Some(3), "just past a replace");
        assert_eq!(carried(6, &edits), Some(8), "after an insertion at it");
        assert_eq!(carried(1, &edits), Some(1));
    }
}
