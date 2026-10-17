//! What a Mono's chat draws in either transcript, BenCode's window and the
//! floating chat (MonoCode `AgentTranscript` with `agentMascot` and
//! `daySeparators`, `monoReaction.ts`, `monoWorkStatus.ts`): when a turn
//! opens a new stretch of the conversation and the day pill over it
//! (`DaySeparator`), the reply that is only a reaction moved onto the
//! message it answers (`monoReactionBlocks`) and its badge
//! (`MonoReactionBadge`), the Mono's signature pill (`MonoSignaturePill`),
//! and the one-line status of the work while a turn runs. The logic is
//! pure, so it is tested without GPUI.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, App, BoxShadow, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Stateful, Styled, div, point,
};

use crate::db::Block;
use crate::monos::{mono_reaction, partial_reaction};
use crate::ui::scale::px;
use crate::ui::transcript::turns::{
    WorkKind, is_notice, is_prose, is_thinking, is_tool, text, tool_kind, tool_kind_name,
};
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `STRETCH_GAP`: a message this long after the one before gets
/// its own day and time.
pub const STRETCH_GAP_MS: i64 = 60 * 60 * 1000;

/// MonoCode `THINKING_LABELS`, one per thought so the line moves on.
const THINKING_LABELS: [&str; 4] = [
    "Thinking…",
    "Pondering…",
    "Considering…",
    "Working it through…",
];

/// What the running turn waits on, which the app knows and the blocks do
/// not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub approval: bool,
    pub question: bool,
    /// Background tasks the turn waits on (MonoCode `backgroundTasks`).
    pub background: usize,
}

fn flag(block: &Block, key: &str) -> bool {
    block.extra.get(key).and_then(|v| v.as_bool()) == Some(true)
}

/// A message the user sent: MonoCode shows neither the app's own prompts
/// nor drafts.
pub fn is_message(block: &Block) -> bool {
    block.role == "user" && !flag(block, "internal") && !flag(block, "draft")
}

/// MonoCode `Block.monoHabit`: a message one of the Mono's habits posted
/// on its own, outside any turn: (habit id, its name, when).
pub fn habit_post(block: &Block) -> Option<(&str, &str, i64)> {
    let habit = block.extra.get("monoHabit")?;
    Some((
        habit.get("id")?.as_str()?,
        habit.get("name")?.as_str()?,
        habit.get("at")?.as_i64()?,
    ))
}

/// MonoCode `isMonoCompletionPrompt`: the hidden turn the app writes when
/// sessions a Mono started report back.
pub fn is_completion_prompt(block: &Block) -> bool {
    block.role == "user"
        && flag(block, "internal")
        && (block.extra.contains_key("monoSessionCompletion")
            || block
                .extra
                .get("appRequestId")
                .and_then(|id| id.as_str())
                .is_some_and(|id| id.starts_with("mono-completion-")))
}

/// MonoCode `groupMonoTurns`: where a turn of a Mono's chat starts, a
/// message or a habit's post (which stands alone).
pub fn starts_turn(block: &Block) -> bool {
    is_message(block) || habit_post(block).is_some()
}

/// MonoCode `opensNewStretch`: the first message, a new day or after a break.
pub fn opens_new_stretch(at: i64, previous: Option<i64>) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    let day = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date())
    };
    at - previous > STRETCH_GAP_MS || day(at) != day(previous)
}

/// MonoCode `monoReactionBlocks`: what the chat shows once each reaction
/// has moved onto the message before it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reactions {
    /// The blocks shown, by index, in order.
    pub kept: Vec<usize>,
    /// The emoji on a message, by the message's index.
    pub on_message: Vec<(usize, String)>,
    /// A reaction with no message to sit on (a reply the Mono sent on its
    /// own), by its block: shown as the emoji alone.
    pub alone: Vec<(usize, String)>,
}

impl Reactions {
    pub fn on(&self, message: usize) -> Option<&str> {
        find(&self.on_message, message)
    }

    pub fn alone(&self, block: usize) -> Option<&str> {
        find(&self.alone, block)
    }
}

fn find(list: &[(usize, String)], ix: usize) -> Option<&str> {
    list.iter()
        .find(|(at, _)| *at == ix)
        .map(|(_, emoji)| emoji.as_str())
}

/// Moves each reaction reply onto the user's message before it, dropping
/// the steps between them but their notices: the reaction is the whole
/// answer, and the work behind it would only draw an empty reply.
/// `streaming`: the last block is still coming in, and is held back while
/// it may yet turn out to be a reaction.
pub fn mono_reactions(blocks: &[Block], streaming: bool) -> Reactions {
    let mut out = Reactions::default();
    // Where in `kept` the message a reaction lands on is.
    let mut target = None;
    for (ix, block) in blocks.iter().enumerate() {
        if block.role == "user" {
            if !flag(block, "internal") {
                target = Some(out.kept.len());
            }
            out.kept.push(ix);
            continue;
        }
        if block.role != "assistant" {
            out.kept.push(ix);
            continue;
        }
        if streaming && ix + 1 == blocks.len() && partial_reaction(text(block)) {
            continue;
        }
        let Some(emoji) = mono_reaction(text(block)) else {
            out.kept.push(ix);
            continue;
        };
        match target {
            Some(at) => {
                let after = out.kept.split_off(at + 1);
                out.kept
                    .extend(after.into_iter().filter(|&step| is_notice(&blocks[step])));
                let message = out.kept[at];
                out.on_message.retain(|(on, _)| *on != message);
                out.on_message.push((message, emoji.to_string()));
            }
            None => {
                out.kept.push(ix);
                out.alone.push((ix, emoji.to_string()));
            }
        }
    }
    out
}

/// MonoCode `monoWorkStatus`: a short line for the latest activity, stable
/// across its streamed chunks.
pub fn work_status(blocks: &[&Block], live: Live) -> String {
    if live.approval {
        return "Waiting for approval…".to_string();
    }
    if live.question {
        return "Waiting for answers…".to_string();
    }
    let mut thoughts = 0usize;
    let mut latest: Option<&Block> = None;
    for block in blocks {
        if is_prose(block) || is_thinking(block) {
            thoughts += 1;
            latest = Some(block);
        } else if is_tool(block) {
            latest = Some(block);
        }
    }
    match latest.filter(|block| is_tool(block)) {
        Some(tool) => match tool_kind(tool) {
            WorkKind::Run => "Running command…",
            WorkKind::Edit => "Editing file…",
            WorkKind::Research if tool_kind_name(tool) == "read" => "Reading file…",
            WorkKind::Research => "Exploring project…",
            WorkKind::Agent => "Running agent…",
            _ => "Using tool…",
        }
        .to_string(),
        None => match live.background {
            0 => THINKING_LABELS[thoughts.saturating_sub(1) % THINKING_LABELS.len()].to_string(),
            1 => "Waiting for background task…".to_string(),
            n => format!("Waiting for {n} background tasks…"),
        },
    }
}

/// "11:33": a time of day, in the user's time zone.
pub fn clock_time(ms: i64) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(
        at.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%H:%M")
            .to_string(),
    )
}

/// MonoCode `dayStamp`: "Today", "Yesterday" or the date ("Tue, Oct 14",
/// with the year when it is not this one), and the time beside it.
pub fn day_stamp(ms: i64) -> (String, String) {
    let tz = jiff::tz::TimeZone::system();
    let Ok(at) = jiff::Timestamp::from_millisecond(ms) else {
        return (String::new(), String::new());
    };
    let at = at.to_zoned(tz.clone());
    let today = jiff::Timestamp::now().to_zoned(tz).date();
    let date = at.date();
    let day = if date == today {
        "Today".to_string()
    } else if today.yesterday().ok() == Some(date) {
        "Yesterday".to_string()
    } else if date.year() == today.year() {
        at.strftime("%a, %b %-d").to_string()
    } else {
        at.strftime("%a, %b %-d, %Y").to_string()
    };
    (day, clock_time(ms).unwrap_or_default())
}

/// MonoCode `DaySeparator`: `flex justify-center px-4 pb-3 pt-6
/// text-[12px]`, a `rounded-full bg-content/[0.07] px-3 py-1` pill with the
/// day in semibold at 75% and the time at 45%.
pub fn day_separator(id: impl Into<ElementId>, at: i64, cx: &App) -> Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    let (day, time) = day_stamp(at);
    div()
        .id(id)
        .flex()
        .w_full()
        .justify_center()
        .px_4()
        .pt_6()
        .pb_3()
        .text_size(Text::Base.px())
        .line_height(px(16.0))
        .child(
            div()
                .flex()
                .gap_1()
                .rounded_full()
                .bg(fg.fill(Fill::Line))
                .px_3()
                .py_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg.opacity(0.75))
                        .child(day),
                )
                .child(div().text_color(fg.ink(Ink::Muted)).child(time)),
        )
}

/// MonoCode `MonoSignaturePill`: `flex min-w-0 items-center gap-1.5
/// rounded-full bg-content/[0.07] py-0.5 pr-3 pl-2`, around the Mono's
/// mark and its name or a status.
pub fn signature_pill(mark: AnyElement, label: AnyElement, cx: &App) -> gpui::Div {
    let fg = cx.theme().colors.fg;
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .rounded_full()
        .bg(fg.fill(Fill::Line))
        .py_0p5()
        .pl_2()
        .pr_3()
        .child(mark)
        .child(label)
}

/// The Mono's name in its pill: `font-medium text-content/80`, in the
/// work line's `text-sm`.
pub fn signature_name(name: String, cx: &App) -> AnyElement {
    div()
        .min_w_0()
        .truncate()
        .text_size(Text::Lg.px())
        .line_height(px(20.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().colors.fg.ink(Ink::Strong))
        .child(name)
        .into_any_element()
}

/// MonoCode `MonoReactionBadge` (`.mono-reaction-badge`): on the message's
/// top left corner, `h-[22px] rounded-[8px] px-[7px] text-[11px]`, a shade
/// lighter than the bubble in dark and the page's own colour lifted off it
/// in light. GPUI has no backdrop blur, so its frosted glass is solid.
pub fn reaction_badge(emoji: &str, cx: &App) -> gpui::Div {
    let colors = &cx.theme().colors;
    let (fg, bg) = (colors.fg, colors.bg);
    let shadow = |x: f32, y: f32, blur: f32, spread: f32, color: Hsla, inset: bool| BoxShadow {
        color,
        offset: point(px(x), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset,
    };
    let (fill, shadows) = if cx.theme().is_dark() {
        (
            bg.blend(fg.opacity(0.17)),
            vec![
                shadow(0.0, 0.5, 0.0, 0.0, fg.opacity(0.22), true),
                shadow(0.0, 0.0, 0.0, 0.5, fg.fill(Fill::Strong), false),
                shadow(0.0, 2.0, 6.0, 0.0, gpui::black().opacity(0.28), false),
            ],
        )
    } else {
        (
            bg,
            vec![
                shadow(0.0, 0.0, 0.0, 1.0, bg, false),
                shadow(0.0, 1.0, 4.0, 0.0, gpui::black().opacity(0.12), false),
            ],
        )
    };
    div()
        .absolute()
        .top(px(-10.0))
        .left(px(-10.0))
        .flex()
        .items_center()
        .justify_center()
        .h(px(22.0))
        .px(px(7.0))
        .rounded(px(8.0))
        .bg(fill)
        .shadow(shadows)
        .text_size(Text::Sm.px())
        .line_height(px(11.0))
        .child(emoji.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(kind: &str) -> Block {
        let mut block = Block::new(format!("t-{kind}"), "tool", "");
        block.tool = Some(serde_json::json!({"kind": kind, "title": "/repo/a.rs"}));
        block
    }

    fn notice(text: &str) -> Block {
        let mut block = Block::new("n", "system", text);
        block
            .extra
            .insert("notice".into(), serde_json::json!("error"));
        block
    }

    #[test]
    fn the_ticker_names_the_latest_step() {
        let read = tool("read");
        let search = tool("search");
        let run = tool("execute");
        let think = Block::new("r", "reasoning", "hmm");
        let say = Block::new("a", "assistant", "ok");
        let live = Live::default();
        assert_eq!(work_status(&[&read], live), "Reading file…");
        assert_eq!(work_status(&[&search], live), "Exploring project…");
        assert_eq!(work_status(&[&read, &run], live), "Running command…");
        assert_eq!(work_status(&[&run, &think], live), "Thinking…");
        assert_eq!(work_status(&[&think, &say], live), "Pondering…");
        assert_eq!(work_status(&[], live), "Thinking…");
        let waiting = Live {
            approval: true,
            ..live
        };
        assert_eq!(work_status(&[&run], waiting), "Waiting for approval…");
        let asking = Live {
            question: true,
            ..live
        };
        assert_eq!(work_status(&[&run], asking), "Waiting for answers…");
        let background = Live {
            background: 2,
            ..live
        };
        assert_eq!(
            work_status(&[&think], background),
            "Waiting for 2 background tasks…"
        );
        // A step still running is named over the background wait.
        assert_eq!(work_status(&[&run], background), "Running command…");
    }

    #[test]
    fn a_reaction_drops_the_work_before_it_but_its_notices() {
        let mut internal = Block::new("i", "user", "a report came in");
        internal
            .extra
            .insert("internal".into(), serde_json::json!(true));
        let blocks = [
            Block::new("u", "user", "thanks"),
            Block::new("a0", "assistant", "Let me look."),
            tool("read"),
            notice("Retrying."),
            internal,
            Block::new("a1", "assistant", "<reaction>🙌</reaction>"),
        ];
        let reactions = mono_reactions(&blocks, false);
        // The internal prompt is no message: the reaction lands on "thanks".
        assert_eq!(reactions.kept, [0, 3]);
        assert_eq!(reactions.on_message, [(0, "🙌".to_string())]);
        assert!(reactions.alone.is_empty());
        // A later reaction to the same message takes its place.
        let mut again = blocks.to_vec();
        again.push(Block::new("a2", "assistant", "<reaction>👍</reaction>"));
        assert_eq!(
            mono_reactions(&again, false).on_message,
            [(0, "👍".to_string())]
        );
    }

    #[test]
    fn a_reply_that_may_be_a_reaction_waits_while_it_streams() {
        let blocks = [
            Block::new("u", "user", "thanks"),
            Block::new("a", "assistant", "<reac"),
        ];
        assert_eq!(mono_reactions(&blocks, true).kept, [0]);
        assert_eq!(mono_reactions(&blocks, false).kept, [0, 1]);
    }

    #[test]
    fn a_reaction_alone_stays_where_it_was() {
        let blocks = [Block::new("a", "assistant", "<reaction>🎉</reaction>")];
        let reactions = mono_reactions(&blocks, false);
        assert_eq!(reactions.kept, [0]);
        assert_eq!(reactions.alone(0), Some("🎉"));
        assert_eq!(reactions.on(0), None);
    }

    #[test]
    fn a_stretch_opens_on_a_break_or_a_new_day() {
        let noon = 1_760_443_200_000;
        assert!(opens_new_stretch(noon, None));
        assert!(!opens_new_stretch(noon + 60_000, Some(noon)));
        assert!(opens_new_stretch(noon + STRETCH_GAP_MS + 1, Some(noon)));
        assert!(opens_new_stretch(noon + 24 * STRETCH_GAP_MS, Some(noon)));
    }
}
