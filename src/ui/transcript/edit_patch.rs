//! What an edit changed, under its tool row (Claude Code's `Update(…)`
//! rows; BenCode's own, MonoCode shows the call alone): "Added 2 lines,
//! removed 1 line", then the hunks in the review's line style. Open by
//! default; a long patch shows its head until "Show N more lines".
//! Drawn from `tool.patch` (`harness::patch::EditPatch`), which only
//! harnesses that report their changes leave.

use std::sync::Arc;

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};
use serde_json::Value;

use crate::app::BenCodeApp;
use crate::db::Block;
use crate::ui::diff_model::{self, Line, LineKind};
use crate::ui::diff_viewer::render_line;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// Lines shown before "Show N more lines".
const SHOWN_LINES: usize = 24;

#[derive(Debug, PartialEq)]
enum PatchRow {
    Line(Line),
    /// Unchanged lines between two hunks.
    Gap,
}

#[derive(Debug, PartialEq)]
struct PatchView {
    summary: String,
    rows: Vec<PatchRow>,
    /// Lines past what is shown, those the patch did not keep included.
    hidden: usize,
}

/// "Added 2 lines, removed 1 line"; "Wrote 3 lines" for a new file.
fn summary(added: usize, removed: usize, created: bool) -> String {
    let lines = |n: usize| {
        if n == 1 {
            "1 line".to_string()
        } else {
            format!("{n} lines")
        }
    };
    match (added, removed) {
        _ if created => format!("Wrote {}", lines(added)),
        (0, removed) => format!("Removed {}", lines(removed)),
        (added, 0) => format!("Added {}", lines(added)),
        (added, removed) => format!("Added {}, removed {}", lines(added), lines(removed)),
    }
}

/// The rows of a `tool.patch`, at most `limit` lines of them.
fn patch_view(patch: &Value, limit: usize) -> Option<PatchView> {
    let count = |key: &str| patch.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
    let created = patch
        .get("created")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hunks = patch.get("hunks")?.as_array()?;
    let mut rows = Vec::new();
    let mut shown = 0;
    let mut total = count("omitted");
    for hunk in hunks {
        let lines = hunk.get("lines").and_then(Value::as_array);
        let lines = lines.map(Vec::as_slice).unwrap_or_default();
        total += lines.len();
        if shown == limit {
            continue;
        }
        let start = |key: &str| hunk.get(key).and_then(Value::as_u64).unwrap_or(1) as u32;
        let (mut old, mut new) = (start("oldStart"), start("newStart"));
        if !rows.is_empty() {
            rows.push(PatchRow::Gap);
        }
        for line in lines.iter().filter_map(Value::as_str) {
            if shown == limit {
                break;
            }
            let (kind, text) = match line.split_at_checked(1) {
                Some(("+", text)) => (LineKind::Add, text),
                Some(("-", text)) => (LineKind::Del, text),
                Some((_, text)) => (LineKind::Context, text),
                None => (LineKind::Context, ""),
            };
            let numbers = match kind {
                LineKind::Add => (None, Some(new)),
                LineKind::Del => (Some(old), None),
                LineKind::Context => (Some(old), Some(new)),
            };
            old += u32::from(kind != LineKind::Add);
            new += u32::from(kind != LineKind::Del);
            rows.push(PatchRow::Line(Line {
                kind,
                text: Arc::from(text),
                old: numbers.0,
                new: numbers.1,
            }));
            shown += 1;
        }
    }
    (shown > 0).then(|| PatchView {
        summary: summary(count("added"), count("removed"), created),
        rows,
        hidden: total - shown,
    })
}

impl BenCodeApp {
    /// The patch under an edit's row, or `None` when it left none.
    pub(super) fn render_edit_patch(
        &self,
        block: &Block,
        path: Option<&str>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let patch = block.tool.as_ref()?.get("patch")?;
        let key = block.id.clone();
        let open = self.transcript_ui.open_patches.contains(&key);
        let view = patch_view(patch, if open { usize::MAX } else { SHOWN_LINES })?;
        let fg = cx.theme().colors.fg;
        let syntax = path.map_or(diff_model::Syntax::Plain, diff_model::syntax_for);
        let more = (view.hidden > 0 && !open).then(|| {
            let label = match view.hidden {
                1 => "Show 1 more line".to_string(),
                n => format!("Show {n} more lines"),
            };
            div()
                .id(SharedString::from(format!("patch-more-{key}")))
                .mt_1()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .cursor_pointer()
                .hover(move |s| s.text_color(fg.ink(Ink::Body)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.transcript_ui.open_patches.insert(key.clone());
                    this.transcript_ui.reflow += 1;
                    cx.notify();
                }))
                .child(label)
        });
        Some(
            div()
                .min_w_0()
                .pb_1()
                .child(
                    div()
                        .pb_1()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(view.summary),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(fg.fill(Fill::Line))
                        .children(view.rows.iter().map(|row| {
                            match row {
                                PatchRow::Line(line) => {
                                    render_line(line, line.number(), syntax, px(0.0), cx)
                                        .into_any_element()
                                }
                                PatchRow::Gap => {
                                    div().h(px(8.0)).bg(fg.fill(Fill::Faint)).into_any_element()
                                }
                            }
                        })),
                )
                .children(more)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(row: &PatchRow) -> (LineKind, &str, Option<u32>) {
        match row {
            PatchRow::Line(line) => (line.kind, &*line.text, line.number()),
            PatchRow::Gap => panic!("a gap"),
        }
    }

    #[test]
    fn lines_are_numbered_from_their_hunk() {
        let patch = json!({"added": 2, "removed": 1, "hunks": [
            {"oldStart": 229, "newStart": 229, "lines": [" a", "-b", "+c", "+d", " e"]}
        ]});
        let view = patch_view(&patch, 10).unwrap();
        assert_eq!(view.summary, "Added 2 lines, removed 1 line");
        let rows: Vec<_> = view.rows.iter().map(text).collect();
        assert_eq!(
            rows,
            [
                (LineKind::Context, "a", Some(229)),
                (LineKind::Del, "b", Some(230)),
                (LineKind::Add, "c", Some(230)),
                (LineKind::Add, "d", Some(231)),
                (LineKind::Context, "e", Some(232)),
            ]
        );
        assert_eq!(view.hidden, 0);
    }

    #[test]
    fn hunks_are_parted_and_a_long_patch_shows_its_head() {
        let patch = json!({"added": 0, "removed": 3, "omitted": 5, "hunks": [
            {"oldStart": 1, "newStart": 1, "lines": ["-a", "-b"]},
            {"oldStart": 40, "newStart": 38, "lines": ["-c", " d"]},
        ]});
        let view = patch_view(&patch, 3).unwrap();
        assert_eq!(view.summary, "Removed 3 lines");
        assert_eq!(view.rows.len(), 4);
        assert_eq!(view.rows[2], PatchRow::Gap);
        assert_eq!(text(&view.rows[3]), (LineKind::Del, "c", Some(40)));
        // One line of the second hunk, and what the patch did not keep.
        assert_eq!(view.hidden, 6);
    }

    #[test]
    fn a_new_file_says_it_was_written() {
        let patch = json!({"added": 1, "removed": 0, "created": true, "hunks": [
            {"oldStart": 0, "newStart": 1, "lines": ["+hi"]}
        ]});
        assert_eq!(patch_view(&patch, 10).unwrap().summary, "Wrote 1 line");
        assert_eq!(patch_view(&json!({"hunks": []}), 10), None);
    }
}
