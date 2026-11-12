//! A message's GitHub issue or pull request link as a chip with a hover
//! card (MonoCode `UserLinkPreview.tsx`, GPUI `transcript/user_links.rs`):
//! "PR #12" or "Issue #7" over the message; hovering shows its state,
//! title, a summary of its body, its author, a pull request's branches and
//! its labels. A click opens the item beside the thread (as the sidebar's
//! badge does), ⌘-click on GitHub. A message that is the link alone shows
//! the chip only.
//!
//! The card is a tooltip view, so GPUI waits before showing it and draws
//! it over everything; it asks `gh` only once hovered, and what it learns
//! is kept for the app's life (`Cards`), as MonoCode's `workItemByKey`.

use std::collections::HashMap;
use std::path::PathBuf;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Global, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::db::{LinkedWorkItem, WorkItemKind};
use crate::github::{Details, Kind, WorkItem};
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode's card is 360px wide.
const CARD_WIDTH: f32 = 360.0;
/// The summary's length before it is cut.
const SUMMARY_CHARS: usize = 280;
/// A chart hue for a merged pull request (`HUE_NAMES`'s Violet).
const VIOLET: usize = 4;

/// What one item's card knows.
#[derive(Clone)]
enum Load {
    Loading,
    Ready(Box<(WorkItem, Details)>),
    /// What `gh` said, and when: a card shown again after `RETRY` asks
    /// again (signed in since, back online).
    Unavailable(String, std::time::Instant),
}

/// How long a failed lookup stands before a card asks `gh` again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// Every card's item, by its key, for the app's life.
#[derive(Default)]
struct Cards(HashMap<String, Load>);

impl Global for Cards {}

/// The link a message's chip shows, and whether the message is that link
/// alone.
/// Drawn with every message row: most messages name no github.com, and
/// learn it from one pass without copying the text.
pub(super) fn message_link(text: &str) -> Option<(LinkedWorkItem, bool)> {
    const HOST: &[u8] = b"github.com/";
    if !text
        .as_bytes()
        .windows(HOST.len())
        .any(|window| window.eq_ignore_ascii_case(HOST))
    {
        return None;
    }
    let (range, item) = LinkedWorkItem::github_link_in(text)?;
    let alone = text.trim() == &text[range];
    Some((item, alone))
}

/// MonoCode `plainTextSummary`: a body's Markdown as one run of prose,
/// cut to `SUMMARY_CHARS`.
fn plain_summary(body: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        // Code, HTML comments and tags, tables and rules say little here.
        if fenced || line.starts_with("<!--") || line.starts_with('<') || line.starts_with('|') {
            continue;
        }
        let line = line.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
        if line.chars().all(|c| matches!(c, '-' | '=' | '_' | '*')) {
            continue;
        }
        for word in line.split_whitespace() {
            let word: String = word
                .chars()
                .filter(|c| !matches!(c, '*' | '_' | '`'))
                .collect();
            if word.is_empty() || word.starts_with("![") {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&word);
        }
    }
    if out.chars().count() > SUMMARY_CHARS {
        let cut: String = out.chars().take(SUMMARY_CHARS).collect();
        let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
        return format!("{}…", cut.trim_end_matches(['.', ',', ';', ':']));
    }
    out
}

/// The item's state as the card names and colours it.
fn state_label(item: &WorkItem) -> &'static str {
    match (item.kind, item.state.as_str(), item.draft) {
        (Kind::Pr, "MERGED", _) => "Merged",
        (Kind::Pr, "OPEN", true) => "Draft",
        (_, "OPEN", _) => "Open",
        (Kind::Issue, "CLOSED", _) if item.state_reason == "NOT_PLANNED" => "Not planned",
        (_, "CLOSED", _) => "Closed",
        _ => "Unknown",
    }
}

fn state_color(label: &str, cx: &App) -> gpui::Hsla {
    let colors = &cx.theme().colors;
    match label {
        "Open" => colors.success,
        "Merged" => colors.hue(VIOLET, "merged pull request"),
        "Closed" | "Not planned" => colors.danger,
        _ => colors.fg.ink(Ink::Muted),
    }
}

/// Where `gh` runs for the card: the thread's folder, or home when that is
/// gone (MonoCode `gh_root`).
fn gh_root(cwd: &str) -> PathBuf {
    if !crate::remote::is_remote(cwd) && !std::path::Path::new(cwd).is_dir() {
        return std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from);
    }
    PathBuf::from(cwd)
}

/// The hover card; it loads its item the first time any card shows it.
struct WorkItemCard {
    link: LinkedWorkItem,
}

impl WorkItemCard {
    fn new(link: LinkedWorkItem, cwd: String, cx: &mut Context<Self>) -> Self {
        let key = link.item_key();
        let known = cx
            .try_global::<Cards>()
            .is_some_and(|cards| match cards.0.get(&key) {
                Some(Load::Unavailable(_, at)) => at.elapsed() < RETRY,
                found => found.is_some(),
            });
        if !known {
            cx.default_global::<Cards>()
                .0
                .insert(key.clone(), Load::Loading);
            let (repo, number) = (link.repo.clone(), link.number);
            let kind = match link.kind {
                WorkItemKind::Pr => Kind::Pr,
                WorkItemKind::Issue => Kind::Issue,
            };
            let root = gh_root(&cwd);
            let fetch = cx.background_executor().spawn(async move {
                let item = crate::github::work_item(&root, &repo, kind, number)?;
                let details = crate::github::details(&root, &repo, kind, number)?;
                Ok::<_, String>((item, details))
            });
            cx.spawn(async move |card, cx| {
                let load = match fetch.await {
                    Ok(found) => Load::Ready(Box::new(found)),
                    Err(err) => {
                        log::warn!("link card {key}: {err}");
                        Load::Unavailable(err, std::time::Instant::now())
                    }
                };
                let landed = card.update(cx, |_, cx| {
                    cx.default_global::<Cards>().0.insert(key, load);
                    cx.notify();
                });
                if landed.is_err() {
                    // The card closed first: what was found is kept anyway.
                    log::debug!("link card closed before its item came");
                }
            })
            .detach();
        }
        Self { link }
    }
}

impl Render for WorkItemCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let load = cx
            .try_global::<Cards>()
            .and_then(|cards| cards.0.get(&self.link.item_key()).cloned())
            .unwrap_or(Load::Loading);
        let heading = format!("{}#{}", self.link.repo, self.link.number);
        let frame = crate::ui::sidebar_popovers::popover_frame(cx)
            .w(px(CARD_WIDTH))
            .p(px(12.0))
            .text_size(Text::Base.px())
            .child(
                div()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(heading),
            );
        let (item, details) = match load {
            Load::Ready(found) => *found,
            Load::Loading => {
                return frame.child(
                    div()
                        .mt_1()
                        .text_color(fg.ink(Ink::Muted))
                        .child("Loading…"),
                );
            }
            Load::Unavailable(err, _) => {
                return frame.child(
                    div()
                        .mt_1()
                        .text_color(fg.ink(Ink::Muted))
                        .line_clamp(3)
                        .child(format!("Could not read it from GitHub: {err}")),
                );
            }
        };
        let state = state_label(&item);
        let tint = state_color(state, cx);
        let summary = plain_summary(&details.body);
        let branches = (item.kind == Kind::Pr && !details.head_ref.is_empty())
            .then(|| format!("{} → {}", details.head_ref, details.base_ref));
        let labels = item.labels.iter().take(6).map(|label| {
            div()
                .px_1p5()
                .rounded(px(4.0))
                .bg(fg.fill(Fill::Strong))
                .text_size(Text::Xs.px())
                .text_color(fg.ink(Ink::Body))
                .child(label.name.clone())
        });
        frame
            .child(
                div()
                    .mt_1()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .mt(px(1.0))
                            .px_1p5()
                            .rounded(px(4.0))
                            .bg(tint.opacity(0.15))
                            .text_size(Text::Xs.px())
                            .text_color(tint)
                            .child(state),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(fg.ink(Ink::Strong))
                            .line_clamp(2)
                            .child(item.title.clone()),
                    ),
            )
            .when(!summary.is_empty(), |el| {
                el.child(
                    div()
                        .mt_2()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Secondary))
                        .line_clamp(4)
                        .child(summary),
                )
            })
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_2()
                    .gap_y_1()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .when(!details.author.is_empty(), |el| {
                        el.child(format!("by {}", details.author))
                    })
                    .children(branches.map(|branches| {
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().mono_family.clone())
                            .child(branches)
                    })),
            )
            .when(!item.labels.is_empty(), |el| {
                el.child(div().mt_2().flex().flex_wrap().gap_1().children(labels))
            })
    }
}

impl BenCodeApp {
    /// GithubWorkItemPreview's chip: "PR #12" or "Issue #7", the card on
    /// hover; `key` keeps each message's chip apart.
    pub(super) fn work_item_chip(
        &self,
        session_id: &str,
        key: &str,
        cwd: &str,
        link: LinkedWorkItem,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (icon, label) = match link.kind {
            WorkItemKind::Pr => (IconName::GitPullRequest, format!("PR #{}", link.number)),
            WorkItemKind::Issue => (IconName::CircleDot, format!("Issue #{}", link.number)),
        };
        let (tip_link, tip_cwd) = (link.clone(), cwd.to_string());
        let (url, sid) = (link.url.clone(), session_id.to_string());
        div()
            .id(SharedString::from(format!("user-link-{key}")))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(24.0))
            .px_2()
            .rounded(px(6.0))
            .bg(fg.fill(Fill::Hover))
            .border_1()
            .border_color(fg.fill(Fill::Strong))
            .cursor_pointer()
            .hover(|s| s.bg(fg.fill(Fill::Strong)))
            .text_size(Text::Base.px())
            .text_color(fg.ink(Ink::Strong))
            .tooltip(move |_, cx: &mut App| {
                let (link, cwd) = (tip_link.clone(), tip_cwd.clone());
                cx.new(|cx| WorkItemCard::new(link, cwd, cx)).into()
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                let mods = event.modifiers();
                if mods.platform || mods.control {
                    cx.open_url(&url);
                } else {
                    this.open_linked_work_item(&sid, &link, cx);
                }
            }))
            .child(Icon::new(icon).size(IconSize::Sm).color(fg.ink(Ink::Body)))
            .child(label)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_of_the_link_alone_shows_only_the_chip() {
        let url = "https://github.com/acme/app/pull/12";
        assert!(message_link(&format!("  {url}\n")).unwrap().1);
        let (item, alone) = message_link(&format!("review {url} please")).unwrap();
        assert!(!alone);
        assert_eq!(item.number, 12);
        assert!(message_link("no link here").is_none());
        assert!(message_link("HTTPS://GitHub.com/acme/app/issues/3").is_some());
    }

    #[test]
    fn summaries_read_as_prose() {
        let body = "## Summary\n\nFixes the **sidebar** `peek`.\n\n```rust\nlet x = 1;\n```\n\
                    <!-- template -->\n- one\n- two\n---\n![shot](a.png)";
        assert_eq!(
            plain_summary(body),
            "Summary Fixes the sidebar peek. one two"
        );
        let long = "word ".repeat(100);
        let cut = plain_summary(&long);
        assert!(cut.ends_with('…') && cut.chars().count() <= SUMMARY_CHARS + 1);
    }

    #[test]
    fn states_name_drafts_merges_and_unplanned_issues() {
        let item = |kind, state: &str, draft, reason: &str| WorkItem {
            kind,
            state: state.into(),
            draft,
            state_reason: reason.into(),
            ..Default::default()
        };
        assert_eq!(state_label(&item(Kind::Pr, "OPEN", true, "")), "Draft");
        assert_eq!(state_label(&item(Kind::Pr, "MERGED", false, "")), "Merged");
        assert_eq!(state_label(&item(Kind::Issue, "OPEN", false, "")), "Open");
        assert_eq!(
            state_label(&item(Kind::Issue, "CLOSED", false, "NOT_PLANNED")),
            "Not planned"
        );
        assert_eq!(
            state_label(&item(Kind::Issue, "CLOSED", false, "COMPLETED")),
            "Closed"
        );
    }
}
