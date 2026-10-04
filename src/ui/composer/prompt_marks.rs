//! What the prompt paints as you type (MonoCode's highlight layer over the
//! textarea): a leading `/plan` in the plan colour, `/draft` dimmed, known
//! `/skill` words in the skill colour, and known `@file` labels in the
//! mention colour with their `@` swapped for the file's icon.

use std::ops::Range;

use ely_gpui_component::forms::Highlight;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Hsla, IntoElement, ParentElement, Pixels, Point, Styled, Window, anchored,
    deferred, div, point, px,
};

use super::mentions::{MentionIndex, MentionTarget};
use super::mode_commands::{self, ModeCommand};
use crate::app::BenCodeApp;
use crate::ui::file_tree::resolve_entry_icon;

/// MonoCode's mention icon: 13px over the `@`.
const MARK_SIZE: f32 = 13.0;

/// The spans the prompt paints, sorted and without overlaps (the earlier
/// span wins).
pub fn prompt_highlights(
    text: &str,
    mentions: &MentionIndex,
    skills: &[String],
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
    pub icon: IconName,
    pub tint: Hsla,
}

fn mark_icon(target: &MentionTarget, cx: &App) -> (IconName, Hsla) {
    if target.relative.starts_with("note/") {
        return (IconName::StickyNote, cx.theme().colors.info);
    }
    let name = target
        .relative
        .rsplit('/')
        .next()
        .unwrap_or(&target.relative);
    resolve_entry_icon(name, target.dir, false)
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
                let (icon, tint) = mark_icon(target, cx);
                Some(MentionMark {
                    center: glyph.center(),
                    icon,
                    tint,
                })
            })
            .collect();
        if marks != self.mention_marks {
            self.mention_marks = marks;
            window.request_animation_frame();
        }
    }

    /// The icons over the prompt's `@`s.
    pub fn render_mention_marks(&self) -> Option<AnyElement> {
        if self.mention_marks.is_empty() {
            return None;
        }
        let half = px(MARK_SIZE / 2.0);
        Some(
            deferred(div().children(self.mention_marks.iter().map(|mark| {
                anchored()
                    .position(point(mark.center.x - half, mark.center.y - half))
                    .child(
                        div()
                            .size(px(MARK_SIZE))
                            .child(Icon::new(mark.icon).size(IconSize::Xs).color(mark.tint)),
                    )
            })))
            .with_priority(1)
            .into_any_element(),
        )
    }
}
