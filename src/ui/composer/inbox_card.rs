//! MonoCode `InboxComposerCard` / `InboxMiniCard`: the GitHub or Backlog
//! issue a thread started from in the Inbox. It waits above the prompt; on
//! send its prompt ("Work on this GitHub issue: …") goes first and the
//! typed note after it (`composeInboxMessage`).

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use crate::ui::scale::px;

use super::cards::card_frame;
use crate::app::BenCodeApp;
use crate::github::{Kind, Label, WorkItem};
use crate::ui::attachment_chip::OnRemove;
use crate::work_items::Provider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxCard {
    pub provider: Provider,
    pub kind: Kind,
    /// `#42`, `PROJ-42`.
    pub identifier: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    /// `owner/name`, or the Backlog project key.
    pub repo: String,
    /// At most two, as MonoCode shows.
    pub labels: Vec<Label>,
    /// What the agent reads before the typed note.
    pub prompt: String,
}

impl InboxCard {
    /// MonoCode `inboxComposerCard`.
    #[cfg(test)]
    pub fn from_item(item: &WorkItem) -> Self {
        Self::from_item_with_body(item, None)
    }

    /// The card with the issue's description in its prompt, for a tracker
    /// whose pages the agent cannot open.
    pub fn from_item_with_body(item: &WorkItem, body: Option<&str>) -> Self {
        Self {
            provider: item.provider,
            kind: item.kind,
            identifier: item.identifier.clone(),
            number: item.number,
            title: if item.title.trim().is_empty() {
                item.identifier.clone()
            } else {
                item.title.trim().to_string()
            },
            url: item.url.trim().to_string(),
            repo: item.repo.clone(),
            labels: item.labels.iter().take(2).cloned().collect(),
            prompt: start_draft(item, body).trim_end().to_string(),
        }
    }

    fn kind_label(&self) -> &'static str {
        match self.kind {
            Kind::Pr => "Pull request",
            Kind::Issue => "Issue",
        }
    }
}

/// MonoCode `inboxStartDraft`: what the item is, its title and its URL,
/// then `body` (the description) when there is one to give.
pub fn start_draft(item: &WorkItem, body: Option<&str>) -> String {
    let kind = match item.kind {
        Kind::Pr => "pull request",
        Kind::Issue => "issue",
    };
    let source = item.provider.label();
    let title = match item.title.trim() {
        "" => format!("{source} {kind} {}", item.identifier),
        title => title.to_string(),
    };
    let mut lines = vec![
        format!("Work on this {source} {kind}:"),
        String::new(),
        format!("{} {title}", item.identifier),
    ];
    let url = item.url.trim();
    if !url.is_empty() {
        lines.push(url.to_string());
    }
    if let Some(body) = body.map(str::trim).filter(|body| !body.is_empty()) {
        lines.extend([String::new(), "Description:".to_string(), body.to_string()]);
    }
    format!("{}\n", lines.join("\n"))
}

/// MonoCode `composeInboxMessage`: the card's prompt, then the note.
pub fn compose_inbox_message(card: &InboxCard, text: &str) -> String {
    let (prompt, note) = (card.prompt.trim(), text.trim());
    match (prompt.is_empty(), note.is_empty()) {
        (true, _) => note.to_string(),
        (_, true) => prompt.to_string(),
        _ => format!("{prompt}\n\n{note}"),
    }
}

/// MonoCode `labelColor`: a 6-digit hex colour.
pub fn label_color(value: &str) -> Option<gpui::Hsla> {
    let hex = value.trim().trim_start_matches('#');
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
        .map(|rgb| gpui::rgb(rgb).into())
}

/// A small label chip (MonoCode `InboxMiniLabel`).
pub fn label_chip(label: &Label, cx: &gpui::App) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    div()
        .flex()
        .flex_none()
        .min_w_0()
        .max_w(px(80.0))
        .items_center()
        .gap_1()
        .px_1p5()
        .rounded(px(4.0))
        .bg(fg.opacity(0.08))
        .text_size(px(10.0))
        .text_color(fg.opacity(0.5))
        .children(
            label_color(&label.color)
                .map(|color| div().size(px(6.0)).flex_none().rounded_full().bg(color)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(SharedString::from(label.name.clone())),
        )
}

/// MonoCode `InboxMiniCard` above the prompt; a click opens it on its
/// tracker.
pub fn inbox_mini_card(
    card: &InboxCard,
    on_dismiss: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let fg = cx.theme().colors.fg;
    let url = card.url.clone();
    let kind_icon = match card.kind {
        Kind::Pr => IconName::GitPullRequest,
        Kind::Issue => IconName::CircleDot,
    };
    let open_tip = format!("Open in {}", card.provider.label());
    let source = match card.provider {
        Provider::GitHub => card.repo.clone(),
        provider => format!("{} · {}", provider.label(), card.repo),
    };
    card_frame(&format!("inbox-{}", card.number), on_dismiss, cx)
        .child(
            div()
                .id("inbox-mini-card")
                .flex()
                .flex_col()
                .when(!url.is_empty(), |el| {
                    el.cursor_pointer()
                        .tooltip(Tooltip::text(open_tip))
                        .on_click(move |_, _, cx| cx.open_url(&url))
                })
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_1p5()
                        .child(
                            Icon::new(kind_icon)
                                .size(IconSize::Xs)
                                .color(fg.opacity(0.45)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(format!("{} · {}", card.kind_label(), card.identifier)),
                        ),
                )
                .child(
                    div()
                        .mt_1()
                        .truncate()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(SharedString::from(card.title.clone())),
                )
                .child(
                    div()
                        .mt_1()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.45))
                                .child(SharedString::from(source)),
                        )
                        .children(card.labels.iter().map(|label| label_chip(label, cx))),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> WorkItem {
        WorkItem {
            kind: Kind::Issue,
            identifier: "#42".into(),
            number: 42,
            title: " Crash on save ".into(),
            url: "https://github.com/o/r/issues/42".into(),
            state: "OPEN".into(),
            state_reason: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            labels: vec![
                Label {
                    name: "a".into(),
                    color: "ff0000".into(),
                },
                Label {
                    name: "b".into(),
                    color: String::new(),
                },
                Label {
                    name: "c".into(),
                    color: String::new(),
                },
            ],
            assignees: Vec::new(),
            draft: false,
            repo: "o/r".into(),
            project: "/p".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_card_carries_monocodes_start_prompt() {
        let card = InboxCard::from_item(&item());
        assert_eq!(
            card.prompt,
            "Work on this GitHub issue:\n\n#42 Crash on save\nhttps://github.com/o/r/issues/42"
        );
        assert_eq!(card.labels.len(), 2);
        assert_eq!(card.title, "Crash on save");
    }

    #[test]
    fn a_backlog_card_carries_the_description_the_agent_cannot_fetch() {
        let item = WorkItem {
            provider: Provider::Backlog,
            identifier: "WEB-42".into(),
            number: 42,
            title: "Login fails".into(),
            url: "https://acme.backlog.com/view/WEB-42".into(),
            repo: "WEB".into(),
            ..Default::default()
        };
        let card = InboxCard::from_item_with_body(&item, Some("  Steps…  "));
        assert_eq!(
            card.prompt,
            "Work on this Backlog issue:\n\nWEB-42 Login fails\nhttps://acme.backlog.com/view/WEB-42\n\nDescription:\nSteps…"
        );
        assert_eq!(
            InboxCard::from_item_with_body(&item, Some(" "))
                .prompt
                .lines()
                .count(),
            4
        );
    }

    #[test]
    fn the_note_follows_the_prompt() {
        let card = InboxCard::from_item(&item());
        assert_eq!(compose_inbox_message(&card, "  "), card.prompt);
        assert_eq!(
            compose_inbox_message(&card, "use the new API"),
            format!("{}\n\nuse the new API", card.prompt)
        );
    }

    #[test]
    fn label_colours_need_six_hex_digits() {
        assert!(label_color("d73a4a").is_some());
        assert!(label_color("#d73a4a").is_some());
        assert!(label_color("red").is_none());
    }
}
