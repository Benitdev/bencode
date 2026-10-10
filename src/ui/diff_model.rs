//! MonoCode's unified review model (`unifiedDiff.ts`, `unifiedDiffWindow.ts`):
//! a file's lines with full context, unchanged runs folded down to three
//! lines around each change, and folds that open twenty lines at a time.

use std::sync::Arc;

use crate::git::{DiffLineKind, number_rows};

/// Unchanged lines kept around each change (`UNIFIED_CONTEXT_DEFAULT`).
pub const CONTEXT: usize = 3;
/// Lines one click on a fold arrow reveals (`UNIFIED_FOLD_STEP`).
pub const FOLD_STEP: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Add,
    Del,
    Context,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub text: Arc<str>,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

impl Line {
    /// The number the gutter shows: the old side for deletions.
    pub fn number(&self) -> Option<u32> {
        match self.kind {
            LineKind::Del => self.old,
            _ => self.new,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Hunk(Vec<Line>),
    Fold { id: usize, lines: Vec<Line> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnifiedFile {
    pub additions: usize,
    pub deletions: usize,
    pub blocks: Vec<Block>,
}

/// How much of a fold is open: `start` lines from its top, `end` from its bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reveal {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expand {
    Up,
    Down,
    All,
}

/// A full-context patch as numbered lines and folds; hunk headers are dropped.
pub fn build(patch: Vec<DiffLineKind>) -> UnifiedFile {
    let mut file = UnifiedFile::default();
    let lines: Vec<Line> = number_rows(patch)
        .into_iter()
        .filter_map(|row| {
            let (kind, text) = match row.kind {
                DiffLineKind::Header(_) => return None,
                DiffLineKind::Addition(text) => (LineKind::Add, text),
                DiffLineKind::Deletion(text) => (LineKind::Del, text),
                DiffLineKind::Context(text) => (LineKind::Context, text),
            };
            Some(Line {
                kind,
                text: text.into(),
                old: row.old,
                new: row.new,
            })
        })
        .collect();
    for line in &lines {
        match line.kind {
            LineKind::Add => file.additions += 1,
            LineKind::Del => file.deletions += 1,
            LineKind::Context => {}
        }
    }
    file.blocks = fold_lines(lines, CONTEXT);
    file
}

/// MonoCode `foldUnifiedLines`: context further than `context` lines from
/// any change becomes a fold.
fn fold_lines(lines: Vec<Line>, context: usize) -> Vec<Block> {
    let mut visible: Vec<bool> = lines.iter().map(|l| l.kind != LineKind::Context).collect();
    for (ix, line) in lines.iter().enumerate() {
        if line.kind == LineKind::Context {
            continue;
        }
        let to = (ix + context).min(lines.len().saturating_sub(1));
        for shown in &mut visible[ix.saturating_sub(context)..=to] {
            *shown = true;
        }
    }
    let mut blocks = Vec::new();
    let mut run: Vec<Line> = Vec::new();
    let mut run_folded = false;
    let mut next_fold = 0;
    let mut flush = |run: &mut Vec<Line>, folded: bool, blocks: &mut Vec<Block>| {
        if run.is_empty() {
            return;
        }
        let lines = std::mem::take(run);
        blocks.push(if folded {
            next_fold += 1;
            Block::Fold {
                id: next_fold - 1,
                lines,
            }
        } else {
            Block::Hunk(lines)
        });
    };
    for (line, shown) in lines.into_iter().zip(visible) {
        let folded = !shown;
        if folded != run_folded {
            flush(&mut run, run_folded, &mut blocks);
            run_folded = folded;
        }
        run.push(line);
    }
    flush(&mut run, run_folded, &mut blocks);
    blocks
}

/// MonoCode `revealedFold`: lines shown above the bar, below it, and hidden.
pub fn revealed(total: usize, reveal: Reveal) -> (usize, usize, usize) {
    let head = reveal.start.min(total);
    let tail = reveal.end.min(total - head);
    (head, tail, total - head - tail)
}

/// MonoCode `expandFold`. "Down" grows the lines shown under the hunk above
/// the fold; "Up" grows those over the hunk below it.
pub fn expand(reveal: Reveal, total: usize, how: Expand) -> Reveal {
    match how {
        Expand::All => Reveal {
            start: total,
            end: 0,
        },
        Expand::Down => Reveal {
            start: reveal.start + FOLD_STEP,
            ..reveal
        },
        Expand::Up => Reveal {
            end: reveal.end + FOLD_STEP,
            ..reveal
        },
    }
}

/// One drawn row of a file's body (MonoCode `DiffViewRow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyRow {
    Line(Line),
    Fold { id: usize, hidden: usize },
}

/// MonoCode `flattenVisibleRows`: the rows a file shows with its folds as
/// `reveal_for` has opened them.
pub fn body_rows(blocks: &[Block], reveal_for: impl Fn(usize) -> Reveal) -> Vec<BodyRow> {
    let mut rows = Vec::new();
    for block in blocks {
        match block {
            Block::Hunk(lines) => rows.extend(lines.iter().cloned().map(BodyRow::Line)),
            Block::Fold { id, lines } => {
                let (head, tail, hidden) = revealed(lines.len(), reveal_for(*id));
                rows.extend(lines[..head].iter().cloned().map(BodyRow::Line));
                if hidden > 0 {
                    rows.push(BodyRow::Fold { id: *id, hidden });
                }
                rows.extend(
                    lines[lines.len() - tail..]
                        .iter()
                        .cloned()
                        .map(BodyRow::Line),
                );
            }
        }
    }
    rows
}

/// One drawn row of a file's body side by side: the old line beside the
/// new one, either of them missing where only one side has a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitRow {
    Pair {
        old: Option<Line>,
        new: Option<Line>,
    },
    Fold {
        id: usize,
        hidden: usize,
    },
}

/// `rows` side by side. An unchanged line is on both sides; a run of
/// deletions is paired, line by line, with the additions that follow it.
pub fn split_rows(rows: Vec<BodyRow>) -> Vec<SplitRow> {
    fn flush(dels: &mut Vec<Line>, adds: &mut Vec<Line>, out: &mut Vec<SplitRow>) {
        let (mut dels, mut adds) = (dels.drain(..), adds.drain(..));
        loop {
            match (dels.next(), adds.next()) {
                (None, None) => break,
                (old, new) => out.push(SplitRow::Pair { old, new }),
            }
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    let (mut dels, mut adds) = (Vec::new(), Vec::new());
    for row in rows {
        match row {
            BodyRow::Line(line) => match line.kind {
                LineKind::Del => {
                    // Deletions after additions start another change.
                    if !adds.is_empty() {
                        flush(&mut dels, &mut adds, &mut out);
                    }
                    dels.push(line);
                }
                LineKind::Add => adds.push(line),
                LineKind::Context => {
                    flush(&mut dels, &mut adds, &mut out);
                    out.push(SplitRow::Pair {
                        old: Some(line.clone()),
                        new: Some(line),
                    });
                }
            },
            BodyRow::Fold { id, hidden } => {
                flush(&mut dels, &mut adds, &mut out);
                out.push(SplitRow::Fold { id, hidden });
            }
        }
    }
    flush(&mut dels, &mut adds, &mut out);
    out
}

/// How a file's lines are coloured, from its name (MonoCode
/// `languageForPath`, with one lexer for every language).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// Prose and data: no colours.
    Plain,
    Code,
    /// Code whose comments start at a `#`.
    HashComments,
    Json,
}

const PLAIN_EXTENSIONS: &[&str] = &[
    "md", "markdown", "mdx", "txt", "text", "rst", "adoc", "org", "log", "csv", "tsv", "lock",
    "svg", "patch", "diff",
];
const HASH_EXTENSIONS: &[&str] = &[
    "py",
    "pyi",
    "rb",
    "sh",
    "bash",
    "zsh",
    "fish",
    "yml",
    "yaml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "pl",
    "r",
    "jl",
    "ex",
    "exs",
    "tf",
    "nix",
    "ps1",
    "mk",
    "cmake",
    "env",
    "properties",
];
const HASH_NAMES: &[&str] = &[
    "dockerfile",
    "makefile",
    "gemfile",
    "rakefile",
    "justfile",
    "cmakelists.txt",
];
const PLAIN_NAMES: &[&str] = &["license", "copying", "authors", "notice", "changelog"];

pub fn syntax_for(path: &str) -> Syntax {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if HASH_NAMES.contains(&name.as_str()) {
        return Syntax::HashComments;
    }
    if PLAIN_NAMES.contains(&name.as_str()) {
        return Syntax::Plain;
    }
    // A dotfile is named by what follows its dot (`.gitignore`, `.env`).
    let extension = name.rsplit('.').next().unwrap_or("");
    if name.starts_with('.') && !name[1..].contains('.') {
        return Syntax::HashComments;
    }
    if PLAIN_EXTENSIONS.contains(&extension) {
        Syntax::Plain
    } else if HASH_EXTENSIONS.contains(&extension) {
        Syntax::HashComments
    } else if matches!(extension, "json" | "jsonc" | "json5") {
        Syntax::Json
    } else {
        Syntax::Code
    }
}

/// Where a `#` comment starts in `text`, given the spans its lexer found
/// (a string is one span, so a `#` inside one is not a span of its own).
/// The `#` opens the line or follows a space, which leaves `$#` alone.
pub fn hash_comment_start(
    text: &str,
    spans: impl IntoIterator<Item = std::ops::Range<usize>>,
) -> Option<usize> {
    spans
        .into_iter()
        .find(|span| {
            &text[span.clone()] == "#"
                && text[..span.start]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace)
        })
        .map(|span| span.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(changed_at: usize, total: usize) -> Vec<DiffLineKind> {
        let mut out = vec![DiffLineKind::Header(format!("@@ -1,{total} +1,{total} @@"))];
        for n in 1..=total {
            if n == changed_at {
                out.push(DiffLineKind::Deletion(format!("old {n}")));
                out.push(DiffLineKind::Addition(format!("new {n}")));
            } else {
                out.push(DiffLineKind::Context(format!("line {n}")));
            }
        }
        out
    }

    fn shape(blocks: &[Block]) -> Vec<(bool, usize)> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Hunk(l) => (false, l.len()),
                Block::Fold { lines, .. } => (true, lines.len()),
            })
            .collect()
    }

    #[test]
    fn far_context_folds_and_counts_follow_lines() {
        let file = build(patch(30, 60));
        assert_eq!((file.additions, file.deletions), (1, 1));
        // 26 folded, 3 + del + add + 3 shown, 27 folded.
        assert_eq!(shape(&file.blocks), [(true, 26), (false, 8), (true, 27)]);
        let Block::Hunk(lines) = &file.blocks[1] else {
            panic!("expected the change")
        };
        assert_eq!(lines[3].number(), Some(30));
        assert_eq!(lines[3].kind, LineKind::Del);
        assert_eq!((lines[4].old, lines[4].new), (None, Some(30)));
    }

    #[test]
    fn near_edges_nothing_folds() {
        let file = build(patch(2, 5));
        assert_eq!(shape(&file.blocks), [(false, 6)]);
        assert!(build(Vec::new()).blocks.is_empty());
    }

    #[test]
    fn folds_open_from_both_ends() {
        let mut reveal = Reveal::default();
        assert_eq!(revealed(26, reveal), (0, 0, 26));
        reveal = expand(reveal, 26, Expand::Down);
        assert_eq!(revealed(26, reveal), (20, 0, 6));
        reveal = expand(reveal, 26, Expand::Up);
        assert_eq!(revealed(26, reveal), (20, 6, 0));
        assert_eq!(revealed(26, expand(reveal, 26, Expand::All)), (26, 0, 0));
    }

    #[test]
    fn rows_put_a_bar_where_lines_stay_hidden() {
        let file = build(patch(30, 60));
        let rows = body_rows(&file.blocks, |_| Reveal::default());
        assert_eq!(rows.len(), 1 + 8 + 1);
        assert_eq!(rows[0], BodyRow::Fold { id: 0, hidden: 26 });
        let opened = body_rows(&file.blocks, |id| {
            if id == 0 {
                Reveal { start: 0, end: 20 }
            } else {
                Reveal::default()
            }
        });
        assert_eq!(opened[0], BodyRow::Fold { id: 0, hidden: 6 });
        assert_eq!(opened.len(), 1 + 20 + 8 + 1);
    }

    fn line(kind: LineKind, text: &str) -> BodyRow {
        BodyRow::Line(Line {
            kind,
            text: text.into(),
            old: None,
            new: None,
        })
    }

    /// Each row as `(old text, new text)`; a fold is `("…", "…")`.
    fn sides(rows: &[SplitRow]) -> Vec<(&str, &str)> {
        fn text(line: &Option<Line>) -> &str {
            line.as_ref().map_or("", |l| &*l.text)
        }
        rows.iter()
            .map(|row| match row {
                SplitRow::Pair { old, new } => (text(old), text(new)),
                SplitRow::Fold { .. } => ("…", "…"),
            })
            .collect()
    }

    #[test]
    fn deletions_sit_beside_the_additions_after_them() {
        let rows = split_rows(vec![
            line(LineKind::Context, "a"),
            line(LineKind::Del, "b"),
            line(LineKind::Del, "c"),
            line(LineKind::Add, "B"),
            line(LineKind::Context, "d"),
            line(LineKind::Add, "e"),
            BodyRow::Fold { id: 0, hidden: 4 },
            line(LineKind::Del, "f"),
        ]);
        assert_eq!(
            sides(&rows),
            [
                ("a", "a"),
                ("b", "B"),
                ("c", ""),
                ("d", "d"),
                ("", "e"),
                ("…", "…"),
                ("f", ""),
            ]
        );
    }

    #[test]
    fn deletions_after_additions_are_another_change() {
        let rows = split_rows(vec![
            line(LineKind::Add, "A"),
            line(LineKind::Del, "b"),
            line(LineKind::Add, "B"),
        ]);
        assert_eq!(sides(&rows), [("", "A"), ("b", "B")]);
    }

    #[test]
    fn a_file_name_picks_its_colours() {
        assert_eq!(syntax_for("src/ui/diff_model.rs"), Syntax::Code);
        assert_eq!(syntax_for("README.md"), Syntax::Plain);
        assert_eq!(syntax_for("LICENSE"), Syntax::Plain);
        assert_eq!(syntax_for("ci/build.sh"), Syntax::HashComments);
        assert_eq!(syntax_for("Dockerfile"), Syntax::HashComments);
        assert_eq!(syntax_for(".gitignore"), Syntax::HashComments);
        assert_eq!(syntax_for("package.json"), Syntax::Json);
        assert_eq!(syntax_for(".eslintrc.json"), Syntax::Json);
    }

    #[test]
    fn a_hash_comment_starts_outside_strings_and_words() {
        // The spans a lexer would give: the string is one of them.
        let text = r#"x = "a # b" # why"#;
        let spans = [0..1, 2..3, 4..11, 12..13, 14..17];
        assert_eq!(hash_comment_start(text, spans), Some(12));
        assert_eq!(hash_comment_start("echo $#", [0..4, 5..6, 6..7]), None);
        assert_eq!(hash_comment_start("# top", [0..1, 2..5]), Some(0));
    }
}
