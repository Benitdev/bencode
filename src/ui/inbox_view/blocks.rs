//! An item's description cut into its top-level blocks, so the detail
//! draws it as rows of a virtual list: a scroll frame lays out the blocks
//! in view, not every row of every table of a long Backlog issue (Ely's
//! `MarkdownRenderer` parses and lays out its whole source each frame).

use std::ops::Range;

use gpui::SharedString;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};

use crate::github::Details;

/// An item's details, its description already cut (off the UI thread).
pub struct Described {
    pub details: Details,
    pub blocks: Vec<SharedString>,
}

impl From<Details> for Described {
    fn from(details: Details) -> Self {
        let blocks = cut(&details.body);
        Self { details, blocks }
    }
}

/// `source` as the rows the detail draws.
pub(super) fn cut(source: &str) -> Vec<SharedString> {
    blocks(source)
        .into_iter()
        .map(|range| SharedString::from(source[range].to_string()))
        .collect()
}

/// The byte ranges of `source`'s top-level blocks. A source whose links
/// or footnotes refer to definitions elsewhere stays whole: a block drawn
/// alone would lose them.
fn blocks(source: &str) -> Vec<Range<usize>> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;
    let whole = std::iter::once(0..source.len()).collect();
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. })
                if matches!(
                    link_type,
                    LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
                ) =>
            {
                return whole;
            }
            Event::Start(Tag::FootnoteDefinition(_)) | Event::FootnoteReference(_) => {
                return whole;
            }
            Event::Start(_) => {
                if depth == 0 {
                    start = range.start;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    found.push(start..range.end);
                }
            }
            _ if depth == 0 => found.push(range),
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(source: &str) -> Vec<&str> {
        blocks(source)
            .into_iter()
            .map(|r| source[r].trim_end())
            .collect()
    }

    #[test]
    fn top_level_blocks_stand_alone() {
        let source = "# Title\n\nSome *text*.\n\n- one\n- two\n\n  nested\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\n```rust\nfn x() {}\n\n```\n";
        assert_eq!(
            cut(source),
            [
                "# Title",
                "Some *text*.",
                "- one\n- two\n\n  nested",
                "| a | b |\n|---|---|\n| 1 | 2 |",
                "---",
                "```rust\nfn x() {}\n\n```",
            ]
        );
    }

    #[test]
    fn references_keep_the_source_whole() {
        let source = "See [the docs][d].\n\n[d]: https://example.com\n";
        assert_eq!(cut(source), [source.trim_end()]);
        let source = "A note[^1].\n\n[^1]: The note.\n";
        assert_eq!(cut(source), [source.trim_end()]);
    }

    #[test]
    fn an_empty_body_has_no_blocks() {
        assert!(blocks(" \n").is_empty());
    }
}
