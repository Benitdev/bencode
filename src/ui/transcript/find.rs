//! Find in conversation (MonoCode `TranscriptFind` + `transcriptFind.ts`):
//! ⌘F opens a bar over the focused thread, Enter / ⌘G step through the
//! messages and tool calls that contain the query, Esc closes it. The
//! current match is revealed (its fold and phase open) and scrolled to just
//! under the bar.

use std::ops::Range;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ListOffset, ParentElement,
    Styled, div, prelude::*, px,
};
use serde_json::Value;

use super::turns::{self, Item, Row};
use crate::app::{BenCodeApp, ViewMode};
use crate::db::Block;
use crate::ui::composer::focus_later;

/// MonoCode scrolls the match to 42px under the top, clear of the bar.
const MATCH_TOP_GAP: f32 = 42.0;
/// About two frames: long enough for the match's neighbours to be measured.
const GAP_SETTLE: std::time::Duration = std::time::Duration::from_millis(32);

/// The open find bar: which thread it searches and the selected match.
pub struct FindState {
    pub session_id: String,
    pub active: usize,
    /// The list row last scrolled to; it is re-measured when the match
    /// moves on, since a long message shows in full only while current.
    shown_row: Option<usize>,
}

/// MonoCode `transcriptBlockText`: the words a block shows, tool output
/// included.
pub fn block_text(block: &Block) -> String {
    let tool = |key: &str| block.tool.as_ref().and_then(|t| t.get(key));
    let preview = |key: &str| tool("preview").and_then(|p| p.get(key));
    let image = |key: &str| block.extra.get("image").and_then(|i| i.get(key));
    [
        Some(turns::text(block)).filter(|t| !t.trim().is_empty()),
        tool("title").and_then(Value::as_str),
        image("name").and_then(Value::as_str),
        image("alt").and_then(Value::as_str),
        tool("detail").and_then(Value::as_str),
        preview("query").and_then(Value::as_str),
        preview("path").and_then(Value::as_str),
        preview("output").and_then(Value::as_str),
        preview("title").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
}

/// MonoCode `findTranscriptBlocks`: the blocks a case-blind `query` hits.
pub fn find_blocks(blocks: &[Block], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            matches!(
                b.role.as_str(),
                "user" | "assistant" | "tool" | "tasks" | "plan" | "image"
            ) && block_text(b).to_lowercase().contains(&needle)
        })
        .map(|(ix, _)| ix)
        .collect()
}

/// Byte ranges of `query` in `text`, case-blind, for painting words.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    // Lowercasing can change byte lengths; match on a char-aligned copy.
    let lower: Vec<(usize, char)> = text
        .char_indices()
        .flat_map(|(at, c)| c.to_lowercase().map(move |l| (at, l)))
        .collect();
    let needle: Vec<char> = needle.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i + needle.len() <= lower.len() {
        if lower[i..i + needle.len()]
            .iter()
            .map(|(_, c)| *c)
            .eq(needle.iter().copied())
        {
            let start = lower[i].0;
            let end = lower
                .get(i + needle.len())
                .map_or(text.len(), |(at, _)| *at);
            if ranges.last().is_none_or(|r: &Range<usize>| r.end <= start) {
                ranges.push(start..end);
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    ranges
}

/// Whether list row `row` draws block `block`.
pub fn row_shows(row: &Row, turns: &[turns::TurnLayout], block: usize) -> bool {
    let (Row::Item { turn, item } | Row::FoldItem { turn, item, .. }) = row else {
        return false;
    };
    match &turns[*turn].items[*item] {
        Item::Block(ix) => *ix == block,
        Item::Activity(group) => group.contains(&block),
    }
}

impl BenCodeApp {
    pub fn find_query(&self, cx: &gpui::App) -> String {
        self.find_input.read(cx).text().to_string()
    }

    /// The query while the bar is open on `session_id`.
    pub fn find_query_for(&self, session_id: &str, cx: &gpui::App) -> Option<String> {
        let state = self.transcript_find.as_ref()?;
        (state.session_id == session_id).then(|| self.find_query(cx))
    }

    /// The selected match's block index in `session_id`, if any.
    pub fn find_current_block(&self, session_id: &str, cx: &gpui::App) -> Option<usize> {
        self.find_hits(session_id, cx).0
    }

    /// The selected match's block and the match count in `session_id`, from
    /// one scan of the thread (a pane works these out once per frame).
    pub fn find_hits(&self, session_id: &str, cx: &gpui::App) -> (Option<usize>, usize) {
        let Some(state) = self
            .transcript_find
            .as_ref()
            .filter(|state| state.session_id == session_id)
        else {
            return (None, 0);
        };
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return (None, 0);
        };
        let matches = find_blocks(&session.blocks, &self.find_query(cx));
        let current = matches
            .len()
            .checked_sub(1)
            .and_then(|last| matches.get(state.active.min(last)).copied());
        (current, matches.len())
    }

    fn find_matches(&self, cx: &gpui::App) -> Vec<usize> {
        self.transcript_find
            .as_ref()
            .and_then(|state| self.sessions.iter().find(|s| s.id == state.session_id))
            .map_or_else(Vec::new, |s| find_blocks(&s.blocks, &self.find_query(cx)))
    }

    /// ⌘F: opens the bar on the focused thread with the last query selected.
    pub fn open_find(&mut self, cx: &mut Context<Self>) {
        if self.active_view_mode != ViewMode::Chat || self.surface.is_some() {
            return;
        }
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        let active = self
            .transcript_find
            .as_ref()
            .filter(|s| s.session_id == session_id)
            .map_or(0, |s| s.active);
        self.transcript_find = Some(FindState {
            session_id,
            active,
            shown_row: None,
        });
        let len = self.find_query(cx).len();
        self.find_input
            .update(cx, |input, cx| input.select(0..len, cx));
        focus_later(self.find_input.read(cx).focus_handle(cx), cx);
        self.reveal_find_match(cx);
        cx.notify();
    }

    pub fn close_find(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.transcript_find.take() else {
            return false;
        };
        // A message shown in full while it was the match folds back.
        if let (Some(view), Some(row)) = (self.transcripts.get(&state.session_id), state.shown_row)
        {
            view.list.remeasure_items(row..row + 1);
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    /// Enter / ⌘G forward, ⇧Enter / ⇧⌘G back, wrapping.
    pub fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.find_matches(cx).len();
        let Some(state) = self.transcript_find.as_mut() else {
            return;
        };
        if len == 0 {
            return;
        }
        state.active =
            (state.active.min(len - 1) as isize + delta).rem_euclid(len as isize) as usize;
        self.reveal_find_match(cx);
        cx.notify();
    }

    /// The query changed: back to the first match.
    pub fn on_find_query_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.transcript_find.as_mut() {
            state.active = 0;
            self.reveal_find_match(cx);
            cx.notify();
        }
    }

    /// MonoCode `navigateToBlock`: opens the match's fold and phase, then
    /// scrolls it under the bar.
    fn reveal_find_match(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.transcript_find.as_ref().map(|s| s.session_id.clone()) else {
            return;
        };
        let Some(block) = self.find_current_block(&session_id, cx) else {
            return;
        };
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let running = self.is_agent_running_in(&session_id);
        let layouts = turns::layout_turns(&session.blocks, running);
        if let Some(turn) = layouts.iter().find(|t| t.range.contains(&block)) {
            self.transcript_ui
                .open_folds
                .insert(turn.id(&session.blocks).to_string());
            let group = turn.items.iter().find_map(|item| match item {
                Item::Activity(group) if group.contains(&block) => Some(group),
                _ => None,
            });
            if let Some(group) = group {
                for phase in turns::build_phases(&session.blocks, group) {
                    if phase.steps.contains(&block) || phase.headline == Some(block) {
                        self.transcript_ui.phase_open.insert(phase.id, true);
                    }
                }
            }
        }
        self.sync_transcript_list_for(&session_id);
        let Some(view) = self.transcripts.get(&session_id) else {
            return;
        };
        let Some(row) = view
            .rows
            .iter()
            .position(|row| row_shows(row, &view.turns, block))
        else {
            return;
        };
        view.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.0),
        });
        // Rows above have no height until drawn, so the gap under the bar is
        // taken once the frame at `row` has measured its neighbours.
        let list = view.list.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(GAP_SETTLE).await;
            let settled = this.update(cx, |_, cx| {
                if list.logical_scroll_top().item_ix == row {
                    list.scroll_by(px(-MATCH_TOP_GAP));
                    cx.notify();
                }
            });
            if let Err(err) = settled {
                log::debug!("find scroll after app drop: {err:#}");
            }
        })
        .detach();
        // Painting can unclamp a long message: retake the old and new rows.
        let previous = self
            .transcript_find
            .as_mut()
            .and_then(|s| s.shown_row.replace(row));
        for row in previous.into_iter().chain([row]) {
            view.list.remeasure_items(row..row + 1);
        }
    }

    /// MonoCode's find bar: top-right of the pane, 360px.
    pub fn render_find_bar(&self, session_id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let query = self.find_query_for(session_id, cx)?;
        let colors = &cx.theme().colors;
        // Counted by the pane this frame, so the thread is scanned once.
        let matches = self.transcripts.get(session_id).map_or(0, |view| view.find_count);
        let active = self.transcript_find.as_ref().map_or(0, |s| s.active);
        let count = if query.trim().is_empty() {
            String::new()
        } else if matches == 0 {
            "No results".into()
        } else {
            format!("{} of {matches}", active.min(matches - 1) + 1)
        };
        let button = |id: &'static str, tip: &'static str, icon: IconName, enabled: bool| {
            let hover = colors.fg.opacity(0.1);
            div()
                .id(id)
                .size(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .tooltip(Tooltip::text(tip))
                .when(enabled, |el| {
                    el.cursor_pointer().hover(move |s| s.bg(hover))
                })
                .when(!enabled, |el| el.opacity(0.3))
                .child(
                    Icon::new(icon)
                        .size(IconSize::Xs)
                        .color(colors.fg.opacity(0.55)),
                )
        };
        Some(
            div()
                .absolute()
                .top_2()
                .right_3()
                .child(
                    div()
                        .id("transcript-find")
                        .w(px(360.0))
                        .flex()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(colors.fg.opacity(0.1))
                        .bg(colors.surface)
                        .shadow_lg()
                        .child(
                            div().ml_1().child(
                                Icon::new(IconName::Search)
                                    .size(IconSize::Xs)
                                    .color(colors.fg.opacity(0.5)),
                            ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.0))
                                .child(self.find_input.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .min_w(px(70.0))
                                .text_right()
                                .font_family(cx.theme().mono_family.clone())
                                .text_size(px(11.0))
                                .text_color(colors.fg.opacity(0.5))
                                .child(count),
                        )
                        .child(
                            button(
                                "find-prev",
                                "Previous match",
                                IconName::ChevronUp,
                                matches > 0,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                        )
                        .child(
                            button(
                                "find-next",
                                "Next match",
                                IconName::ChevronDown,
                                matches > 0,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                        )
                        .child(
                            button("find-close", "Close find", IconName::X, true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.close_find(cx);
                                }),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// Paints `query`'s hits in `text`; the current block's first hit stands out
/// (MonoCode's `::highlight` match and current colours).
pub fn highlighted_text(
    text: &str,
    query: &str,
    current: bool,
    cx: &gpui::App,
) -> gpui::StyledText {
    let colors = &cx.theme().colors;
    let highlights = match_ranges(text, query)
        .into_iter()
        .enumerate()
        .map(|(ix, range)| {
            let color = if current && ix == 0 {
                colors.accent.opacity(0.62)
            } else {
                colors.warning.opacity(0.46)
            };
            (
                range,
                gpui::HighlightStyle {
                    background_color: Some(color),
                    ..Default::default()
                },
            )
        });
    gpui::StyledText::new(text.to_string()).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn block(id: &str, role: &str, text: &str) -> Block {
        Block::new(id, role, text)
    }

    #[test]
    fn finds_messages_and_tool_output_but_not_thinking() {
        let mut tool = block("t", "tool", "");
        tool.tool = Some(json!({ "title": "rg search", "preview": { "output": "Search module" } }));
        let blocks = [
            block("u", "user", "Check the SEARCH code"),
            block("r", "reasoning", "search thoughts"),
            tool,
            block("a", "assistant", "No match here"),
        ];
        assert_eq!(find_blocks(&blocks, " search "), [0, 2]);
        assert!(find_blocks(&blocks, "  ").is_empty());
    }

    #[test]
    fn match_ranges_are_case_blind_byte_ranges() {
        let text = "Điều này, điều kia";
        let ranges = match_ranges(text, "điều");
        assert_eq!(ranges.len(), 2);
        assert_eq!(&text[ranges[1].clone()], "điều");
        assert_eq!(match_ranges("aaaa", "aa"), [0..2, 2..4]);
    }
}
