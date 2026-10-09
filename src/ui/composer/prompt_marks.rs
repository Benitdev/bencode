//! What the prompt paints as you type (MonoCode's highlight layer over the
//! textarea): a leading `/plan` in the plan colour, `/draft` dimmed, known
//! `/skill` words in the skill colour, and known `@file` labels in the
//! mention colour with their `@` swapped for the file's icon; `/mcp` tags
//! (`@mcp/name`) are painted in the mention colour too.

use std::ops::Range;

use ely_gpui_component::forms::Highlight;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, IntoElement, ParentElement, Pixels, Point, Styled, Window, anchored, deferred,
    div, point,
};

use crate::ui::scale::px;

use super::mcp_tags::{McpTag, mcp_tag_ranges};
use super::mentions::{MentionIndex, MentionTarget};
use super::mode_commands::{self, ModeCommand};
use crate::app::BenCodeApp;
use crate::ui::file_tree::{EntryIcon, resolve_entry_icon};

/// MonoCode's mention icon: 13px over the `@`.
const MARK_SIZE: f32 = 13.0;

/// The spans the prompt paints, sorted and without overlaps (the earlier
/// span wins).
pub fn prompt_highlights(
    text: &str,
    mentions: &MentionIndex,
    skills: &[String],
    mcp_tags: &[McpTag],
    cx: &App,
) -> Vec<(Range<usize>, Highlight)> {
    let colors = &cx.theme().colors;
    let mut spans: Vec<(Range<usize>, Highlight)> = Vec::new();
    if let Some((mode, range)) = mode_commands::leading_mode(text) {
        let color = match mode {
            ModeCommand::Plan => colors.warning.opacity(0.9),
            ModeCommand::Draft => colors.fg.opacity(0.7),
        };
        spans.push((range, Highlight::new(color)));
    }
    let skill = Highlight::new(colors.warning);
    spans.extend(
        mode_commands::skill_tokens(text, skills)
            .into_iter()
            .map(|range| (range, skill)),
    );
    // The `@` keeps its width but not its ink; the icon sits on top.
    let hidden = Highlight::new(gpui::transparent_black());
    let mention = Highlight::new(colors.info);
    spans.extend(
        mcp_tag_ranges(text, mcp_tags)
            .into_iter()
            .map(|(range, _)| (range, mention)),
    );
    for (range, _, _) in mentions.scan(text) {
        spans.push((range.start..range.start + 1, hidden));
        spans.push((range.start + 1..range.end, mention));
    }
    spans.sort_by_key(|(range, _)| range.start);
    let mut end = 0;
    spans.retain(|(range, _)| {
        let keep = range.start >= end;
        if keep {
            end = range.end;
        }
        keep
    });
    spans
}

/// One `@` to cover: where it is in the window and what icon it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct MentionMark {
    pub center: Point<Pixels>,
    pub icon: MarkIcon,
}

/// A note's sticky note, or the mentioned file's own icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MarkIcon {
    Note,
    Entry(EntryIcon),
}

fn mark_icon(target: &MentionTarget) -> MarkIcon {
    if target.relative.starts_with("note/") {
        return MarkIcon::Note;
    }
    let name = target
        .relative
        .rsplit('/')
        .next()
        .unwrap_or(&target.relative);
    MarkIcon::Entry(resolve_entry_icon(name, target.dir, false))
}

impl BenCodeApp {
    /// Finds each mention's `@` in the prompt's last layout. The field lays
    /// itself out after the app, so when the marks moved (typing, scrolling)
    /// one more frame is asked for to catch up.
    pub fn sync_mention_marks(&mut self, window: &mut Window, cx: &App) {
        let input = self.prompt_input.read(cx);
        let mentions = self.project_files.mentions.borrow().clone();
        let marks: Vec<MentionMark> = mentions
            .scan(input.text())
            .into_iter()
            .filter_map(|(range, target, _)| {
                let glyph = input
                    .bounds_for_range(range.start..range.start + 1)
                    .into_iter()
                    .next()?;
                Some(MentionMark {
                    center: glyph.center(),
                    icon: mark_icon(target),
                })
            })
            .collect();
        if marks != self.mention_marks {
            self.mention_marks = marks;
            window.request_animation_frame();
        }
    }

    /// The icons over the prompt's `@`s.
    pub fn render_mention_marks(&self, cx: &App) -> Option<AnyElement> {
        if self.mention_marks.is_empty() {
            return None;
        }
        let note_tint = cx.theme().colors.info;
        let half = px(MARK_SIZE / 2.0);
        Some(
            deferred(div().children(self.mention_marks.iter().map(|mark| {
                anchored()
                    .position(point(mark.center.x - half, mark.center.y - half))
                    .child(
                        div().size(px(MARK_SIZE)).child(match mark.icon {
                            MarkIcon::Note => Icon::new(IconName::StickyNote)
                                .size(IconSize::Xs)
                                .color(note_tint)
                                .into_any_element(),
                            MarkIcon::Entry(icon) => icon.size(IconSize::Xs).into_any_element(),
                        }),
                    )
            })))
            .with_priority(1)
            .into_any_element(),
        )
    }
}
