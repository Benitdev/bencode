//! Ask a bot about a thread (BenCode's own): the thread, and the words
//! picked from it if any, as a card in the bot's composer. On send the bot
//! reads which thread it is, where, and its latest exchange (no hunting for
//! it, nothing pasted by hand), then the typed question; a bot that works
//! on the thread's project is also told how to read further with
//! `sessions.read`. The sent message keeps a small card naming the thread.

use std::sync::Arc;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div};
use serde_json::{Value, json};

use super::cards::card_frame;
use crate::app::BenCodeApp;
use crate::db::Block;
use crate::ui::attachment_chip::OnRemove;
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Text, Tone as _};

/// How much of the latest exchange the bot is handed: the user's message,
/// then the reply.
const ASKED_CHARS: usize = 600;
const REPLY_CHARS: usize = 1_200;
/// How much of the picked words the card shows (the bot gets them all).
const QUOTE_PREVIEW: usize = 140;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadCard {
    pub session_id: String,
    pub title: String,
    /// The project's folder, and the name it goes by on the rail.
    pub cwd: String,
    pub project: String,
    /// The words picked from the thread.
    pub quote: Option<String>,
    /// Its latest exchange: what the user asked and the reply, cut short.
    pub asked: String,
    pub reply: String,
    /// The bot works on the thread's project, so `sessions.read` reaches it.
    pub readable: bool,
}

/// `text` cut to `max` characters, an ellipsis for the rest.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// The thread's latest exchange: its last message from the user and the
/// last reply after it, each cut short.
pub fn latest_exchange(blocks: &[Arc<Block>]) -> (String, String) {
    let text = |block: &Block| block.text.as_deref().unwrap_or("").trim().to_string();
    let asked_at = blocks.iter().rposition(|block| {
        block.role == "user" && block.extra.get("internal").and_then(Value::as_bool) != Some(true)
    });
    let Some(at) = asked_at else {
        return (String::new(), String::new());
    };
    let reply = blocks[at + 1..]
        .iter()
        .rev()
        .find(|block| block.role == "assistant" && block.tool.is_none())
        .map(|block| text(block))
        .unwrap_or_default();
    (
        clip(&text(&blocks[at]), ASKED_CHARS),
        clip(&reply, REPLY_CHARS),
    )
}

impl ThreadCard {
    /// What the bot reads: the thread, then the typed question.
    pub fn agent_prompt(&self, text: &str) -> String {
        let mut lines = vec![
            "<thread_ref>".to_string(),
            "The user is asking about a thread in BenCode.".to_string(),
            format!("Title: {}", self.title),
            format!("Project: {} ({})", self.project, self.cwd),
        ];
        if self.readable {
            lines.push(format!(
                "Read more of it with app sessions.read {{\"sessionId\":{},\"project\":{}}}",
                Value::from(self.session_id.as_str()),
                Value::from(self.cwd.as_str()),
            ));
        } else {
            lines.push(
                "It is in a project you do not work on, so this is all you can see of it."
                    .to_string(),
            );
        }
        if !self.asked.is_empty() {
            lines.extend([
                "Latest message from the user:".to_string(),
                self.asked.clone(),
            ]);
        }
        if !self.reply.is_empty() {
            lines.extend(["The agent's latest reply:".to_string(), self.reply.clone()]);
        }
        lines.push("</thread_ref>".to_string());
        if let Some(quote) = self.quote.as_deref() {
            lines.extend([
                String::new(),
                "<quote>".to_string(),
                quote.trim().to_string(),
                "</quote>".to_string(),
            ]);
        }
        let question = text.trim();
        let question = if question.is_empty() {
            "What is going on in this thread, and does it need anything from me?"
        } else {
            question
        };
        format!("{}\n\n{question}", lines.join("\n"))
    }

    /// The sent message's mark, read back by [`turn_card`].
    pub fn meta(&self) -> Value {
        json!({
            "sessionId": self.session_id,
            "title": self.title,
            "project": self.project,
            "quote": self.quote,
        })
    }
}

/// What a sent message's mark holds: the thread's title, its project and
/// the picked words.
fn mark(block: &Block) -> Option<(String, String, Option<String>)> {
    let meta = block.extra.get("threadCard")?;
    let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_string);
    Some((
        text("title")?,
        text("project").unwrap_or_default(),
        text("quote"),
    ))
}

fn mini_card(
    id: &str,
    title: &str,
    project: &str,
    quote: Option<&str>,
    on_dismiss: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let fg = cx.theme().colors.fg;
    card_frame(&format!("thread-{id}"), on_dismiss, cx)
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::MessageSquare)
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Muted)),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Secondary))
                        .child(format!("Thread · {project}")),
                ),
        )
        .child(
            div()
                .mt_1()
                .truncate()
                .text_size(Text::Md.px())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(title.to_string()),
        )
        .children(quote.map(|quote| {
            div()
                .mt_1()
                .pl_2()
                .border_l_2()
                .border_color(fg.ink(Ink::Faint))
                .max_h(px(36.0))
                .overflow_hidden()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Secondary))
                .child(clip(quote, QUOTE_PREVIEW))
        }))
        .into_any_element()
}

/// The card above the prompt, removable.
pub fn thread_mini_card(
    card: &ThreadCard,
    on_dismiss: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    mini_card(
        &card.session_id,
        &card.title,
        &card.project,
        card.quote.as_deref(),
        on_dismiss,
        cx,
    )
}

/// The card on a sent message that carried one.
pub fn turn_card(block: &Block, cx: &Context<BenCodeApp>) -> Option<AnyElement> {
    let (title, project, quote) = mark(block)?;
    Some(mini_card(
        &block.id,
        &title,
        &project,
        quote.as_deref(),
        None,
        cx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(role: &str, text: &str) -> Arc<Block> {
        Arc::new(Block::new(format!("{role}-{text}"), role, text))
    }

    fn card(readable: bool, quote: Option<&str>) -> ThreadCard {
        ThreadCard {
            session_id: "s1".into(),
            title: "Fix the rail".into(),
            cwd: "/src/app".into(),
            project: "app".into(),
            quote: quote.map(str::to_string),
            asked: "Why is it slow?".into(),
            reply: "The list redraws.".into(),
            readable,
        }
    }

    #[test]
    fn the_latest_exchange_is_the_last_question_and_its_reply() {
        let blocks = [
            block("user", "first"),
            block("assistant", "old reply"),
            block("user", "  second  "),
            block("reasoning", "thinking"),
            block("assistant", "new reply"),
        ];
        assert_eq!(
            latest_exchange(&blocks),
            ("second".to_string(), "new reply".to_string())
        );
        assert_eq!(latest_exchange(&[]), (String::new(), String::new()));
        let long = "x".repeat(2_000);
        let (asked, _) = latest_exchange(&[block("user", &long)]);
        assert_eq!(asked.chars().count(), ASKED_CHARS);
        assert!(asked.ends_with('…'));
    }

    #[test]
    fn the_bot_reads_the_thread_then_the_question() {
        let prompt = card(true, Some("the rail redraws")).agent_prompt("  why?  ");
        assert!(prompt.starts_with("<thread_ref>\n"));
        assert!(prompt.contains("Project: app (/src/app)"));
        assert!(prompt.contains(r#"app sessions.read {"sessionId":"s1","project":"/src/app"}"#));
        assert!(prompt.contains("<quote>\nthe rail redraws\n</quote>"));
        assert!(prompt.ends_with("\n\nwhy?"));
        let elsewhere = card(false, None).agent_prompt("");
        assert!(elsewhere.contains("a project you do not work on"));
        assert!(!elsewhere.contains("<quote>"));
        assert!(elsewhere.ends_with("does it need anything from me?"));
    }

    #[test]
    fn a_sent_message_keeps_the_thread_it_asked_about() {
        let mut sent = Block::new("u1", "user", "why?");
        sent.extra
            .insert("threadCard".into(), card(true, None).meta());
        assert_eq!(
            mark(&sent),
            Some(("Fix the rail".to_string(), "app".to_string(), None))
        );
        assert_eq!(mark(&Block::new("u2", "user", "hi")), None);
    }
}
