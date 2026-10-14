//! The floating chat's rows, from the Mono's blocks: MonoCode's messaging
//! transcript (`AgentTranscript` with `inlineWork`, `groupMonoTurnItems`,
//! `monoWorkStatus.ts`, `monoReaction.ts`, `DaySeparator`). Pure, so it is
//! tested without GPUI.
//!
//! A turn is the user's message and what the Mono did for it. Its work is
//! never drawn in the chat: the turn's identity line carries it, as a live
//! ticker ("Reading file…") while the turn runs, and the full trail is one
//! click away in BenCode. While it runs even its prose stays in the work;
//! once it settles the reply after the last step shows, with Copy and the
//! time it finished.

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::db::Block;
use crate::monos::mono_reaction;
use crate::ui::attachment_chip::ChipFile;
use crate::ui::transcript::turns::{
    WorkKind, is_notice, is_prose, is_thinking, is_tool, text, tool_kind, tool_kind_name,
};

/// MonoCode `MONO_PAGE_TURNS`: the turns shown at first, and how many
/// more each scroll to the top brings in.
pub const PAGE_TURNS: usize = 10;

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

/// Where a message the user sent stands (MonoCode `MessageDelivery`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// Waiting behind the running turn.
    Pending,
    /// Held since the agent was stopped: "Not sent", with Resume.
    Paused,
}

/// What the running turn waits on, which the app knows and the blocks do
/// not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub approval: bool,
    pub question: bool,
    /// Background tasks the turn waits on (MonoCode `backgroundTasks`).
    pub background: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloatItem {
    /// Earlier turns are left out; reaching this row brings them in.
    Earlier,
    /// MonoCode `DaySeparator`: "Today 11:33" over a turn that opens a new
    /// stretch of the conversation (ms).
    Day(i64),
    /// What the user sent: its text, its files, and the emoji the Mono
    /// answered it with when its whole reply was a reaction.
    User {
        text: String,
        files: Vec<ChipFile>,
        reaction: Option<String>,
        delivery: Delivery,
    },
    /// MonoCode `MonoTurnHeader`: the Mono's pill, and while the turn runs
    /// the ticker beside it.
    Header {
        ticker: Option<String>,
    },
    /// What the Mono said.
    Reply(String),
    Notice(String),
    /// MonoCode `TurnDuration`: Copy and when the turn finished (ms), shown
    /// while the turn is hovered.
    Footer {
        copy: String,
        at: Option<i64>,
    },
}

/// A row and the turn it belongs to (the index of its first block), which
/// hovering it reveals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatRow {
    pub turn: usize,
    pub item: FloatItem,
}

fn flag(block: &Block, key: &str) -> bool {
    block.extra.get(key).and_then(|v| v.as_bool()) == Some(true)
}

/// A message the user sent: MonoCode shows neither the app's own prompts
/// nor drafts.
pub fn is_message(block: &Block) -> bool {
    block.role == "user" && !flag(block, "internal") && !flag(block, "draft")
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

/// Where the newest `turns` turns start; 0 when there are no more.
pub fn window_start(blocks: &[Block], turns: usize) -> usize {
    let mut seen = 0;
    for (ix, block) in blocks.iter().enumerate().rev() {
        if is_message(block) {
            seen += 1;
            if seen == turns {
                // Nothing before it but what no message heads: all of it.
                return if blocks[..ix].iter().any(is_message) {
                    ix
                } else {
                    0
                };
            }
        }
    }
    0
}

/// What the rows read from `blocks[start..]`, folded into a number: while
/// it holds, so do the rows. A block changes in place only by its text
/// growing or a tool call's kind being filled in; anything else adds,
/// removes or replaces blocks, which moves the count or the ids at the
/// ends. No allocation: it runs on each of the app's redraws.
pub fn rows_key(blocks: &[Block], start: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    let shown = &blocks[start..];
    (start, shown.len()).hash(&mut hasher);
    shown.first().map(|block| &block.id).hash(&mut hasher);
    shown.last().map(|block| &block.id).hash(&mut hasher);
    for block in shown {
        hasher.write_usize(block.text.as_deref().map_or(0, str::len));
        block.duration_ms.hash(&mut hasher);
        if is_tool(block) {
            tool_kind_name(block).hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// The rows for `blocks[start..]`; `live` while its last turn runs.
pub fn float_rows(blocks: &[Block], start: usize, live: Option<Live>) -> Vec<FloatRow> {
    let mut rows = Vec::new();
    if start > 0 {
        rows.push(FloatRow {
            turn: usize::MAX,
            item: FloatItem::Earlier,
        });
    }
    let shown = &blocks[start..];
    // Each turn starts at a message; what comes before the first is a turn
    // with none (a reply the Mono sent on its own).
    let mut heads: Vec<usize> = shown
        .iter()
        .enumerate()
        .filter(|(_, block)| is_message(block))
        .map(|(ix, _)| ix)
        .collect();
    if heads.first() != Some(&0) {
        heads.insert(0, 0);
    }
    let mut previous_at = blocks[..start]
        .iter()
        .rev()
        .find(|block| is_message(block))
        .and_then(|block| block.started_at);
    let count = heads.len();
    for (turn, &head) in heads.iter().enumerate() {
        let end = heads.get(turn + 1).copied().unwrap_or(shown.len());
        let range = &shown[head..end];
        let (user, rest) = match range.split_first() {
            Some((first, rest)) if is_message(first) => (Some(first), rest),
            _ => (None, range),
        };
        if user.is_none() && rest.is_empty() {
            continue;
        }
        let live = live.filter(|_| turn + 1 == count);
        // Named by where it starts in the whole conversation, so bringing
        // in earlier turns leaves the rows after them as they were.
        push_turn(&mut rows, start + head, user, rest, live, &mut previous_at);
    }
    rows
}

fn push_turn(
    rows: &mut Vec<FloatRow>,
    turn: usize,
    user: Option<&Block>,
    rest: &[Block],
    live: Option<Live>,
    previous_at: &mut Option<i64>,
) {
    let push = |rows: &mut Vec<FloatRow>, item| rows.push(FloatRow { turn, item });
    let rest: Vec<&Block> = rest.iter().filter(|block| block.role != "user").collect();
    // MonoCode `groupMonoTurnItems`: the work runs to the last step (any
    // prose, while the turn is live); the prose after it is the reply.
    let work_end = rest.iter().rposition(|block| {
        is_tool(block) || is_thinking(block) || (live.is_some() && is_prose(block))
    });
    let shows = |ix: usize| live.is_none() && work_end.is_none_or(|end| ix > end);
    let replies: Vec<&str> = rest
        .iter()
        .enumerate()
        .filter(|(ix, block)| is_prose(block) && shows(*ix))
        .map(|(_, block)| text(block).trim())
        .collect();
    let reaction = (replies.len() == 1)
        .then(|| mono_reaction(replies[0]))
        .flatten()
        .map(str::to_string);
    if let Some(user) = user {
        if let Some(at) = user.started_at {
            if opens_new_stretch(at, *previous_at) {
                push(rows, FloatItem::Day(at));
            }
            *previous_at = Some(at);
        }
        let files = user
            .extra
            .get("attachments")
            .and_then(|files| files.as_array())
            .map(|files| files.iter().filter_map(ChipFile::from_block_json).collect())
            .unwrap_or_default();
        push(
            rows,
            FloatItem::User {
                text: text(user).trim().to_string(),
                files,
                // A Mono that only reacted answered on the message itself.
                reaction: reaction.clone(),
                delivery: Delivery::Sent,
            },
        );
    }
    let notices = rest.iter().filter(|block| is_notice(block));
    if reaction.is_some() && user.is_some() {
        for block in notices {
            push(rows, FloatItem::Notice(text(block).trim().to_string()));
        }
        return;
    }
    // MonoCode `showFoldLine`: the line is there from the first token on.
    let worked = rest.iter().any(|block| {
        !matches!(block.role.as_str(), "assistant" | "reasoning") || !text(block).trim().is_empty()
    });
    if live.is_none() && !worked {
        return;
    }
    push(
        rows,
        FloatItem::Header {
            ticker: live.map(|live| work_status(&rest, live)),
        },
    );
    for (ix, block) in rest.iter().enumerate() {
        if is_notice(block) {
            push(rows, FloatItem::Notice(text(block).trim().to_string()));
        } else if is_prose(block) && shows(ix) {
            let said = match &reaction {
                // No message to sit on: the emoji on its own.
                Some(emoji) => emoji.as_str(),
                None => text(block).trim(),
            };
            match rows.last_mut() {
                // A reply streams as one block, but a provider may split it.
                Some(FloatRow {
                    item: FloatItem::Reply(reply),
                    ..
                }) => {
                    reply.push_str("\n\n");
                    reply.push_str(said);
                }
                _ => push(rows, FloatItem::Reply(said.to_string())),
            }
        }
    }
    if live.is_none() {
        // MonoCode `turnCopyText` over what the turn shows.
        let copy = match &reaction {
            Some(emoji) => emoji.clone(),
            None => replies.join("\n\n"),
        };
        let at = user.and_then(|user| Some(user.started_at? + user.duration_ms.unwrap_or(0)));
        push(rows, FloatItem::Footer { copy, at });
    }
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

    fn items(blocks: &[Block], live: Option<Live>) -> Vec<FloatItem> {
        float_rows(blocks, 0, live)
            .into_iter()
            .map(|row| row.item)
            .collect()
    }

    fn user(text: &str) -> FloatItem {
        FloatItem::User {
            text: text.into(),
            files: Vec::new(),
            reaction: None,
            delivery: Delivery::Sent,
        }
    }

    #[test]
    fn a_settled_turn_shows_its_reply_after_the_work() {
        let blocks = [
            Block::new("u", "user", " fix it "),
            Block::new("a0", "assistant", "Let me look."),
            Block::new("r", "reasoning", "hmm"),
            tool("read"),
            Block::new("a1", "assistant", "Fixed."),
            Block::new("a2", "assistant", "Anything else?"),
        ];
        assert_eq!(
            items(&blocks, None),
            [
                user("fix it"),
                FloatItem::Header { ticker: None },
                FloatItem::Reply("Fixed.\n\nAnything else?".into()),
                FloatItem::Footer {
                    copy: "Fixed.\n\nAnything else?".into(),
                    at: None
                },
            ]
        );
    }

    #[test]
    fn a_live_turn_keeps_its_prose_in_the_ticker() {
        let blocks = [
            Block::new("u", "user", "fix it"),
            tool("read"),
            Block::new("a", "assistant", "Found it, now"),
        ];
        let live = Live::default();
        assert_eq!(
            items(&blocks, Some(live)),
            [
                user("fix it"),
                FloatItem::Header {
                    ticker: Some("Thinking…".into())
                },
            ]
        );
        // Only the last turn is live.
        let two = [
            Block::new("u1", "user", "hi"),
            Block::new("a1", "assistant", "Hello"),
            Block::new("u2", "user", "fix it"),
        ];
        let rows = items(&two, Some(live));
        assert!(rows.contains(&FloatItem::Reply("Hello".into())));
        assert_eq!(
            rows.last(),
            Some(&FloatItem::Header {
                ticker: Some("Thinking…".into())
            })
        );
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
    fn a_reaction_lands_on_the_message_it_answers() {
        let blocks = [
            Block::new("u1", "user", "fix it"),
            Block::new("a1", "assistant", "Fixed."),
            Block::new("u2", "user", "thanks!"),
            tool("read"),
            notice("Stopped."),
            Block::new("a2", "assistant", "<reaction>👍</reaction>"),
        ];
        let rows = items(&blocks, None);
        let at = rows.iter().position(|item| {
            *item
                == FloatItem::User {
                    text: "thanks!".into(),
                    files: Vec::new(),
                    reaction: Some("👍".into()),
                    delivery: Delivery::Sent,
                }
        });
        // No pill or footer: the notice alone stays.
        assert_eq!(
            rows[at.expect("the message") + 1..],
            [FloatItem::Notice("Stopped.".into())]
        );
        // With no message to sit on, the emoji stands as the reply.
        let alone = [Block::new("a", "assistant", "<reaction>🎉</reaction>")];
        assert!(items(&alone, None).contains(&FloatItem::Reply("🎉".into())));
    }

    #[test]
    fn status_lines_and_internal_prompts_stay_out_of_the_chat() {
        let mut internal = Block::new("i", "user", "report");
        internal
            .extra
            .insert("internal".into(), serde_json::json!(true));
        let blocks = [
            Block::new("u", "user", "hi"),
            internal,
            Block::new("st", "system", "Compacting…"),
            Block::new("a", "assistant", "Hi"),
        ];
        let rows = items(&blocks, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[2], FloatItem::Reply("Hi".into()));
    }

    #[test]
    fn the_footer_says_when_the_turn_finished() {
        let mut asked = Block::new("u", "user", "hi");
        asked.started_at = Some(1_000);
        asked.duration_ms = Some(500);
        let blocks = [asked, Block::new("a", "assistant", "Hi")];
        assert_eq!(
            items(&blocks, None).last(),
            Some(&FloatItem::Footer {
                copy: "Hi".into(),
                at: Some(1_500)
            })
        );
    }

    #[test]
    fn a_message_after_a_break_opens_a_new_stretch() {
        let at = |id: &str, ms: i64| {
            let mut block = Block::new(id, "user", id);
            block.started_at = Some(ms);
            block
        };
        // Noon UTC, so the two first stay on one day in most time zones.
        let noon = 1_760_443_200_000;
        let blocks = [
            at("a", noon),
            at("b", noon + 10 * 60 * 1000),
            at("c", noon + 2 * STRETCH_GAP_MS),
        ];
        let days = items(&blocks, None)
            .iter()
            .filter(|item| matches!(item, FloatItem::Day(_)))
            .count();
        assert_eq!(days, 2);
        // A page that starts mid-conversation knows the message before it.
        let later = float_rows(&blocks, 1, None);
        assert_eq!(later[0].item, FloatItem::Earlier);
        // Its rows are the same ones the whole conversation has.
        let all = float_rows(&blocks, 0, None);
        assert_eq!(later[1..], all[all.len() - (later.len() - 1)..]);
        assert!(
            !later
                .iter()
                .any(|row| row.item == FloatItem::Day(noon + 10 * 60 * 1000))
        );
    }

    #[test]
    fn pages_count_turns_from_the_newest() {
        let blocks: Vec<Block> = (0..5)
            .flat_map(|ix| {
                [
                    Block::new(format!("u{ix}"), "user", "q"),
                    Block::new(format!("a{ix}"), "assistant", "a"),
                ]
            })
            .collect();
        assert_eq!(window_start(&blocks, 2), 6);
        assert_eq!(window_start(&blocks, 5), 0);
        assert_eq!(window_start(&blocks, 10), 0);
        let turns = float_rows(&blocks, 6, None)
            .iter()
            .filter(|row| matches!(row.item, FloatItem::User { .. }))
            .count();
        assert_eq!(turns, 2);
    }

    #[test]
    fn a_message_keeps_its_files() {
        let mut block = Block::new("u", "user", "");
        block.extra.insert(
            "attachments".into(),
            serde_json::json!([{"id": "f1", "name": "a.png", "path": "/tmp/a.png", "kind": "image"}]),
        );
        let rows = items(&[block], None);
        let [FloatItem::User { text, files, .. }] = rows.as_slice() else {
            panic!("one message expected, got {rows:?}");
        };
        assert!(text.is_empty());
        assert_eq!(files[0].name, "a.png");
        assert!(files[0].image);
    }

    #[test]
    fn the_rows_key_follows_what_the_rows_show() {
        let blocks = vec![
            Block::new("u", "user", "hello"),
            tool("read"),
            Block::new("a", "assistant", "Hi"),
        ];
        let key = rows_key(&blocks, 0);
        assert_eq!(rows_key(&blocks.clone(), 0), key);
        let mut streamed = blocks.clone();
        streamed[2].text = Some("Hi there".into());
        assert_ne!(rows_key(&streamed, 0), key);
        let mut rekinded = blocks.clone();
        if let Some(tool) = rekinded[1].tool.as_mut().and_then(|t| t.as_object_mut()) {
            tool.insert("kind".into(), serde_json::json!("edit"));
        }
        assert_ne!(rows_key(&rekinded, 0), key);
        let mut finished = blocks.clone();
        finished[0].duration_ms = Some(10);
        assert_ne!(rows_key(&finished, 0), key);
        assert_ne!(rows_key(&blocks, 1), key);
    }
}
