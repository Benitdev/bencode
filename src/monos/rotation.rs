//! MonoCode `monoRotation.ts`: a Mono's chat never ends, but the provider
//! session behind it does. Once it fills most of the model's context
//! window, the next turn quietly starts a fresh provider session, given the
//! soul and memory again plus a brief of where the conversation was: recent
//! exchanges word for word and a line for each earlier one. Nothing is
//! summarized by a model, so a rotation costs nothing and the brief never
//! drifts into a summary of a summary.
//!
//! BenCode briefs a fresh provider session the same way after the account
//! behind it changed (MonoCode's `pendingSwitch` recap on a usage limit).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Block;

/// MonoCode `ROTATE_RATIO`: rotate at 80% of the reported window.
pub const ROTATE_RATIO: f64 = 0.8;
const VERBATIM_EXCHANGES: usize = 2;
const VERBATIM_USER_CHARS: usize = 2_000;
const VERBATIM_REPLY_CHARS: usize = 3_000;
const LINE_CHARS: usize = 280;
const EARLIER_CHARS: usize = 3_000;

/// MonoCode `MonoRotation`: what the latest rotation started the provider
/// session with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rotation {
    pub at: i64,
    pub reason: String,
    /// The last block the replaced session saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_block_id: Option<String>,
    /// One line per exchange before the verbatim ones, oldest first.
    #[serde(default)]
    pub earlier: Vec<String>,
}

/// MonoCode `rotationReason`: whether the next turn should start a fresh
/// provider session.
pub fn should_rotate(provider: Option<&str>, used: Option<i64>, window: Option<i64>) -> bool {
    let (Some(used), Some(window)) = (used, window) else {
        return false;
    };
    provider.is_some_and(|id| !id.is_empty())
        && window > 0
        && used >= 0
        && (used as f64 / window as f64).min(1.0) >= ROTATE_RATIO
}

struct Exchange {
    at: Option<i64>,
    user: String,
    reply: String,
    habit: Option<String>,
    completion: Option<String>,
}

fn flag(block: &Block, key: &str) -> bool {
    block.extra.get(key).and_then(Value::as_bool) == Some(true)
}

fn text(block: &Block) -> &str {
    block.text.as_deref().unwrap_or("").trim()
}

/// MonoCode `exchanges`: the user's turns and what the Mono finally said.
fn exchanges(blocks: &[Block], after: Option<&str>) -> Vec<Exchange> {
    let start = after
        .and_then(|id| blocks.iter().position(|b| b.id == id))
        .map_or(0, |ix| ix + 1);
    let mut out: Vec<Exchange> = Vec::new();
    for block in &blocks[start..] {
        if block.role == "user"
            && let Some(meta) = block.extra.get("monoSessionCompletion")
        {
            out.push(Exchange {
                at: block.started_at,
                user: String::new(),
                reply: String::new(),
                habit: None,
                completion: Some(meta.get("title").and_then(Value::as_str).unwrap_or("").to_string()),
            });
            continue;
        }
        if flag(block, "internal") || flag(block, "draft") {
            continue;
        }
        if block.role == "user" {
            out.push(Exchange {
                at: block.started_at,
                user: text(block).to_string(),
                reply: String::new(),
                habit: None,
                completion: None,
            });
            continue;
        }
        if let Some((_, name, at)) = crate::ui::transcript::mono::habit_post(block)
            && !text(block).is_empty()
        {
            out.push(Exchange {
                at: Some(at),
                user: String::new(),
                reply: text(block).to_string(),
                habit: Some(name.to_string()),
                completion: None,
            });
            continue;
        }
        if let Some(current) = out.last_mut()
            && current.habit.is_none()
            && block.role == "assistant"
            && block.tool.is_none()
            && !text(block).is_empty()
        {
            // The last prose of a turn is its answer.
            current.reply = text(block).to_string();
        }
    }
    out.retain(|e| !e.user.is_empty() || !e.reply.is_empty());
    out
}

/// MonoCode `clip`: the start and the end, with what is between cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let half = (max - 20) / 2;
    let chars: Vec<char> = text.chars().collect();
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[chars.len() - half..].iter().collect();
    format!("{} […] {}", head.trim_end(), tail.trim_start())
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max - 1).collect::<String>().trim_end())
    }
}

fn stamp(at: Option<i64>) -> String {
    at.and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%b %-d, %H:%M").to_string())
        .unwrap_or_default()
}

fn when(at: Option<i64>) -> String {
    match stamp(at) {
        s if s.is_empty() => String::new(),
        s => format!("{s} · "),
    }
}

fn exchange_line(e: &Exchange) -> String {
    if let Some(title) = &e.completion {
        return format!("- {}You, reviewing session \"{title}\": {}", when(e.at), one_line(&e.reply, LINE_CHARS));
    }
    if let Some(habit) = &e.habit {
        return format!("- {}You, from your habit \"{habit}\": {}", when(e.at), one_line(&e.reply, LINE_CHARS));
    }
    let reply = if e.reply.is_empty() {
        String::new()
    } else {
        format!(" → You: {}", one_line(&e.reply, LINE_CHARS))
    };
    format!("- {}User: {}{reply}", when(e.at), one_line(&e.user, LINE_CHARS))
}

/// MonoCode `newestWithin`: the newest lines that fit, oldest first.
fn newest_within(lines: &[String], budget: usize) -> Vec<String> {
    let mut kept = Vec::new();
    let mut used = 0;
    for line in lines.iter().rev() {
        used += line.chars().count() + 1;
        if used > budget {
            break;
        }
        kept.insert(0, line.clone());
    }
    kept
}

/// MonoCode `planRotation`: from the chat as it stands before the turn that
/// rotates, what the fresh session is told, and the record to keep.
pub fn plan_rotation(
    blocks: &[Block],
    previous: Option<&Rotation>,
    reason: &str,
    now: i64,
    session_id: &str,
) -> (Rotation, String) {
    let segment = exchanges(blocks, previous.and_then(|p| p.after_block_id.as_deref()));
    let split = segment.len().saturating_sub(VERBATIM_EXCHANGES);
    let (older, verbatim) = segment.split_at(split);
    let lines: Vec<String> = previous
        .map(|p| p.earlier.clone())
        .unwrap_or_default()
        .into_iter()
        .chain(older.iter().map(exchange_line))
        .collect();
    let earlier = newest_within(&lines, EARLIER_CHARS);
    let carried: Vec<String> = earlier
        .iter()
        .cloned()
        .chain(verbatim.iter().map(exchange_line))
        .collect();
    let rotation = Rotation {
        at: now,
        reason: reason.to_string(),
        after_block_id: blocks.last().map(|b| b.id.clone()),
        earlier: newest_within(&carried, EARLIER_CHARS),
    };
    let mut parts = vec![format!(
        "This is the same long-lived conversation with the user. It continues in a fresh \
         session so your context stays small; nothing was lost. Pick up where it left off and \
         do not mention the switch unless asked. For anything older or more detailed than \
         this, read the chat itself with `app sessions.read {{\"sessionId\":\"{session_id}\"}}` \
         (pass nextBefore for older pages), and check memory with `app memory.search`."
    )];
    if !earlier.is_empty() {
        parts.push(format!("Earlier, oldest first:\n{}", earlier.join("\n")));
    }
    if !verbatim.is_empty() {
        let at = |e: &Exchange| match stamp(e.at) {
            s if s.is_empty() => "earlier".to_string(),
            s => s,
        };
        let shown: Vec<String> = verbatim
            .iter()
            .map(|e| {
                if let Some(habit) = &e.habit {
                    format!("You, on your own from your habit \"{habit}\" ({}):\n{}", at(e), clip(&e.reply, VERBATIM_REPLY_CHARS))
                } else if let Some(title) = &e.completion {
                    format!("You, reviewing session \"{title}\" ({}):\n{}", at(e), clip(&e.reply, VERBATIM_REPLY_CHARS))
                } else {
                    let reply = if e.reply.is_empty() {
                        String::new()
                    } else {
                        format!("\n\nYou:\n{}", clip(&e.reply, VERBATIM_REPLY_CHARS))
                    };
                    format!("User ({}):\n{}{reply}", at(e), clip(&e.user, VERBATIM_USER_CHARS))
                }
            })
            .collect();
        parts.push(format!("Most recent, word for word:\n\n{}", shown.join("\n\n---\n\n")));
    }
    let brief = format!("<previous_conversation>\n{}\n</previous_conversation>", parts.join("\n\n"));
    (rotation, brief)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, role: &str, text: &str) -> Block {
        Block::new(id, role, text)
    }

    #[test]
    fn rotation_starts_at_eighty_percent() {
        assert!(should_rotate(Some("p"), Some(80), Some(100)));
        assert!(!should_rotate(Some("p"), Some(79), Some(100)));
        assert!(!should_rotate(None, Some(90), Some(100)));
        assert!(!should_rotate(Some("p"), Some(90), None));
    }

    #[test]
    fn the_brief_carries_recent_words_and_earlier_lines() {
        let blocks = vec![
            block("u1", "user", "first question"),
            block("a1", "assistant", "first answer"),
            block("u2", "user", "second"),
            block("a2a", "assistant", "narration"),
            block("a2b", "assistant", "second answer"),
            block("u3", "user", "third"),
            block("a3", "assistant", "third answer"),
        ];
        let (rotation, brief) = plan_rotation(&blocks, None, "context", 1, "mono-1");
        assert_eq!(rotation.after_block_id.as_deref(), Some("a3"));
        assert!(brief.contains("Earlier, oldest first:\n- User: first question → You: first answer"));
        assert!(brief.contains("User (earlier):\nsecond\n\nYou:\nsecond answer"));
        assert!(brief.contains("{\"sessionId\":\"mono-1\"}"));
        assert!(!brief.contains("narration"));
        // The next rotation starts after this one, with its lines.
        let more = [blocks.clone(), vec![block("u4", "user", "fourth")]].concat();
        let (_, next) = plan_rotation(&more, Some(&rotation), "context", 2, "mono-1");
        assert!(next.contains("first question"));
        assert!(next.contains("User (earlier):\nfourth"));
    }

    #[test]
    fn long_text_keeps_both_ends() {
        let long = format!("{}{}", "a".repeat(3000), "z".repeat(3000));
        let clipped = clip(&long, 100);
        assert!(clipped.starts_with("aaa") && clipped.ends_with("zzz") && clipped.contains("[…]"));
    }
}
