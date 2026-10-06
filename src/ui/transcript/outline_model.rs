//! MonoCode `model/promptOutline.ts` and `PromptOutline`'s `barStack`:
//! which prompts get a bar, which one is current, how the bars fit, the
//! hover ripple, and the preview card's text. Pure, so it is tested here;
//! the view is `outline.rs`.

use crate::db::Block;

/// Fewer prompts than this show no outline.
pub const MIN_PROMPTS: usize = 2;
/// How far the hover ripple reaches, in bars on each side.
pub const RIPPLE_SPAN: usize = 2;
pub const BAR_HEIGHT: f32 = 2.0;
const BAR_GAP: f32 = 10.0;
const BAR_GAP_MIN: f32 = 1.0;
const BAR_STACK_MAX: f32 = 330.0;
const BAR_STACK_PANE_SHARE: f32 = 0.75;
const REPLY_SCAN_CHARS: usize = 2000;

fn flag(block: &Block, key: &str) -> bool {
    block.extra.get(key).and_then(|v| v.as_bool()) == Some(true)
}

/// MonoCode `promptBlocks`: the user's messages, as block indexes.
pub fn prompt_blocks(blocks: &[Block]) -> Vec<usize> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role == "user" && !flag(b, "internal"))
        .map(|(ix, _)| ix)
        .collect()
}

/// Where a prompt's row lies against the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Above,
    Inside,
    Below,
}

/// MonoCode `activePromptId`: the topmost prompt inside the viewport, else
/// the last one above it, else the first. Near the end the last prompt
/// wins: the prompts on the final screen cannot reach the top.
pub fn active_prompt(places: &[Place], near_end: bool) -> Option<usize> {
    if places.is_empty() {
        return None;
    }
    if near_end {
        return Some(places.len() - 1);
    }
    if let Some(inside) = places.iter().position(|p| *p == Place::Inside) {
        return Some(inside);
    }
    Some(places.iter().rposition(|p| *p == Place::Above).unwrap_or(0))
}

/// MonoCode `barWindow`: at most `max` bars; the window slides to keep the
/// active prompt inside and prefers the newest prompts.
pub fn bar_window(count: usize, active: Option<usize>, max: usize) -> std::ops::Range<usize> {
    if count <= max {
        return 0..count;
    }
    let newest = count - max;
    let start = active.map_or(newest, |a| a.min(newest));
    start..start + max
}

/// The bars shown and the gap between them.
#[derive(Clone, Debug, PartialEq)]
pub struct BarStack {
    pub range: std::ops::Range<usize>,
    pub gap: f32,
}

/// The stack's height budget for a transcript `viewport` tall.
pub fn stack_budget(viewport: f32) -> f32 {
    BAR_STACK_MAX.min((viewport * BAR_STACK_PANE_SHARE).floor())
}

/// MonoCode `barStack`: one bar per prompt while they fit the budget; the
/// gap shrinks first, then a window slides.
pub fn bar_stack(count: usize, active: Option<usize>, budget: f32) -> BarStack {
    let fit = (((budget + BAR_GAP_MIN) / (BAR_HEIGHT + BAR_GAP_MIN)).floor() as usize).max(1);
    let range = bar_window(count, active, fit);
    let shown = range.len();
    let gap = if shown > 1 {
        let room = ((budget - shown as f32 * BAR_HEIGHT) / (shown - 1) as f32).floor();
        BAR_GAP.min(BAR_GAP_MIN.max(room))
    } else {
        0.0
    };
    BarStack { range, gap }
}

/// MonoCode `barLift`: dock-style magnification, 1 on the hovered bar
/// tapering to 0 past the ripple span.
pub fn bar_lift(index: usize, hover: Option<usize>) -> f32 {
    let Some(hover) = hover else {
        return 0.0;
    };
    let distance = index.abs_diff(hover);
    if distance > RIPPLE_SPAN {
        return 0.0;
    }
    (RIPPLE_SPAN + 1 - distance) as f32 / (RIPPLE_SPAN + 1) as f32
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// MonoCode `promptLabel`: the bar's name and the card's title.
pub fn prompt_label(block: &Block) -> String {
    let card = block.second_opinion.as_ref();
    let handoff = card.is_some_and(|c| c.get("kind").and_then(|k| k.as_str()) == Some("handoff"));
    let text = if card.is_none() || handoff {
        first_line(block.text.as_deref().unwrap_or(""))
    } else {
        String::new()
    };
    if !text.is_empty() {
        return text;
    }
    if let Some(card) = card {
        if handoff {
            return "Handoff".into();
        }
        let request = first_line(card.get("request").and_then(|r| r.as_str()).unwrap_or(""));
        return if request.is_empty() {
            "Second opinion".into()
        } else {
            format!("Second opinion: {request}")
        };
    }
    if let Some(title) = block
        .extra
        .get("noteCard")
        .and_then(|n| n.get("title"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
    {
        return title.to_string();
    }
    let files: Vec<&str> = block
        .extra
        .get("attachments")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|f| f.get("name").and_then(|n| n.as_str()))
        .collect();
    match files.as_slice() {
        [] => "Empty message".into(),
        [one] => (*one).to_string(),
        [first, rest @ ..] => format!("{first} +{}", rest.len()),
    }
}

/// The prompt and the head of its reply, shown while a bar is hovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptPreview {
    pub title: String,
    pub reply: Option<String>,
    pub detail: Option<String>,
}

/// MonoCode `promptPreview`.
pub fn prompt_preview(blocks: &[Block], prompt: usize) -> Option<PromptPreview> {
    let block = blocks.get(prompt)?;
    let mut reply = Vec::new();
    for block in &blocks[prompt + 1..] {
        if block.role == "user" {
            break;
        }
        if block.role != "assistant" {
            continue;
        }
        reply = preview_lines(block.text.as_deref().unwrap_or(""), 2);
        if !reply.is_empty() {
            break;
        }
    }
    let mut reply = reply.into_iter();
    Some(PromptPreview {
        title: prompt_label(block),
        reply: reply.next(),
        detail: reply.next(),
    })
}

/// Drops a leading `#`/`>` run, a bullet or a list number, with the space
/// after it (MonoCode `/^(?:[#>]+|[-*+]|\d+[.)])\s+/`).
fn strip_marker(line: &str) -> &str {
    let marker_end = if line.starts_with(['#', '>']) {
        line.find(|c| c != '#' && c != '>').unwrap_or(line.len())
    } else if line.starts_with(['-', '*', '+']) {
        1
    } else {
        let digits = line
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(line.len());
        if digits > 0 && line[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            0
        }
    };
    if marker_end == 0 {
        return line;
    }
    let rest = &line[marker_end..];
    let trimmed = rest.trim_start();
    if trimmed.len() < rest.len() {
        trimmed
    } else {
        line
    }
}

/// MonoCode `previewLines`: the first `max` prose lines of a reply;
/// markers, fenced code and rules drop out.
pub fn preview_lines(text: &str, max: usize) -> Vec<String> {
    let scan: String = text.chars().take(REPLY_SCAN_CHARS).collect();
    let mut lines = Vec::new();
    let mut fenced = false;
    for raw in scan.lines() {
        let line = raw.trim();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let plain = strip_marker(line)
            .replace("**", "")
            .replace('`', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // Rules, table separators and lone punctuation read as noise.
        if !plain.chars().any(char::is_alphanumeric) {
            continue;
        }
        lines.push(plain);
        if lines.len() == max {
            break;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(role: &str, text: &str) -> Block {
        Block::new("b", role, text)
    }

    fn with(mut block: Block, key: &str, value: serde_json::Value) -> Block {
        block.extra.insert(key.into(), value);
        block
    }

    #[test]
    fn internal_messages_get_no_bar() {
        let blocks = [
            block("user", "a"),
            block("assistant", "x"),
            with(block("user", "hidden"), "internal", json!(true)),
            block("user", "b"),
        ];
        assert_eq!(prompt_blocks(&blocks), [0, 3]);
    }

    #[test]
    fn active_prompt_follows_monocode() {
        use Place::*;
        assert_eq!(active_prompt(&[], false), None);
        assert_eq!(active_prompt(&[Above, Inside, Inside], false), Some(1));
        assert_eq!(active_prompt(&[Above, Above, Below], false), Some(1));
        assert_eq!(active_prompt(&[Below, Below], false), Some(0));
        assert_eq!(active_prompt(&[Above, Inside, Below], true), Some(2));
    }

    #[test]
    fn window_prefers_newest_and_keeps_active() {
        assert_eq!(bar_window(5, None, 10), 0..5);
        assert_eq!(bar_window(10, None, 4), 6..10);
        assert_eq!(bar_window(10, Some(2), 4), 2..6);
        assert_eq!(bar_window(10, Some(9), 4), 6..10);
    }

    #[test]
    fn stack_shrinks_the_gap_then_slides() {
        assert_eq!(stack_budget(300.0), 225.0);
        assert_eq!(stack_budget(1000.0), 330.0);
        assert_eq!(
            bar_stack(6, None, 330.0),
            BarStack {
                range: 0..6,
                gap: 10.0
            }
        );
        // 40 bars: (330 - 80) / 39 = 6.4 → 6.
        assert_eq!(bar_stack(40, None, 330.0).gap, 6.0);
        // More than fit at a 1px gap: a window of 110.
        let many = bar_stack(200, None, 330.0);
        assert_eq!(many.range, 90..200);
        assert_eq!(many.gap, 1.0);
        assert_eq!(bar_stack(1, None, 330.0).gap, 0.0);
    }

    #[test]
    fn lift_tapers_over_the_ripple() {
        assert_eq!(bar_lift(3, None), 0.0);
        assert_eq!(bar_lift(3, Some(3)), 1.0);
        assert!((bar_lift(4, Some(3)) - 2.0 / 3.0).abs() < 1e-6);
        assert!((bar_lift(1, Some(3)) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(bar_lift(0, Some(3)), 0.0);
    }

    #[test]
    fn labels_fall_back_like_monocode() {
        assert_eq!(
            prompt_label(&block("user", "\n  fix   the bug \nmore")),
            "fix the bug"
        );
        let review = with(block("user", "ignored"), "x", json!(null));
        let review = Block {
            second_opinion: Some(json!({ "request": "check auth" })),
            ..review
        };
        assert_eq!(prompt_label(&review), "Second opinion: check auth");
        let handoff = Block {
            second_opinion: Some(json!({ "kind": "handoff" })),
            ..block("user", "")
        };
        assert_eq!(prompt_label(&handoff), "Handoff");
        let note = with(block("user", ""), "noteCard", json!({ "title": "Plan" }));
        assert_eq!(prompt_label(&note), "Plan");
        let files = with(
            block("user", ""),
            "attachments",
            json!([{ "name": "a.png" }, { "name": "b.png" }]),
        );
        assert_eq!(prompt_label(&files), "a.png +1");
        assert_eq!(prompt_label(&block("user", "")), "Empty message");
    }

    #[test]
    fn preview_skips_markers_code_and_rules() {
        let text = "## Summary\n\n---\n```rust\nlet x = 1;\n```\n- **Fixed** the `parser`\n1. Then tests\n";
        assert_eq!(preview_lines(text, 2), ["Summary", "Fixed the parser"]);
        assert_eq!(
            preview_lines("|---|---|\n> quoted line", 2),
            ["quoted line"]
        );
        assert_eq!(preview_lines("-dash kept", 1), ["-dash kept"]);
    }

    #[test]
    fn preview_takes_the_first_reply_with_prose() {
        let blocks = [
            block("user", "first"),
            block("tool", "read"),
            block("assistant", ""),
            block("assistant", "Done.\nAll green."),
            block("user", "second"),
        ];
        assert_eq!(
            prompt_preview(&blocks, 0),
            Some(PromptPreview {
                title: "first".into(),
                reply: Some("Done.".into()),
                detail: Some("All green.".into()),
            })
        );
        assert_eq!(prompt_preview(&blocks, 4).unwrap().reply, None);
    }
}
