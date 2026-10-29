//! MonoCode `resolveLinkedWorkItem` (`sessionWorkItem.ts`): a thread's
//! first message links it to the GitHub issue or pull request it names,
//! so the sidebar shows the link without the Link dialog. A pasted URL
//! wins (a Backlog issue's too, on the connected space); then "PR #12" or
//! "issue 7" in the working copy's repository; then "this PR", the current
//! branch's pull request. A link already there is never replaced, and a
//! lookup that fails links nothing. `gh` runs only when the message names
//! one of these.
//!
//! Threads started from a Backlog issue before "Send to agent" linked them
//! are linked once, as they load: their first message holds the issue's
//! URL (`backfill_backlog_links`).

use std::path::Path;

use gpui::Context;
use serde::Deserialize;

use crate::app::BenCodeApp;
use crate::db::{Block, LinkedWorkItem, WorkItemKind};
use crate::github::WorkItem;
use crate::work_items::Provider;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` lower-cased with each run of whitespace made one space, so
/// "pull\n request" reads as "pull request".
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The number after `keyword` at a word start: `keyword`, spaces, an
/// optional `#`, spaces, digits ending at a word boundary.
fn number_after(text: &str, keyword: &str) -> Option<i64> {
    let mut from = 0;
    while let Some(found) = text[from..].find(keyword) {
        let start = from + found;
        from = start + keyword.len();
        if text[..start].chars().next_back().is_some_and(is_word) {
            continue;
        }
        let rest = text[from..].trim_start();
        let rest = rest.strip_prefix('#').unwrap_or(rest).trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || rest[digits.len()..].chars().next().is_some_and(is_word) {
            continue;
        }
        if let Some(number) = digits.parse::<i64>().ok().filter(|n| *n > 0) {
            return Some(number);
        }
    }
    None
}

/// MonoCode `explicitHint`: "PR #12", "pull request 12", "issue 7"; a pull
/// request is looked for first.
fn explicit_hint(message: &str) -> Option<(WorkItemKind, i64)> {
    let text = normalized(message);
    ["pull request", "pr"]
        .iter()
        .find_map(|keyword| number_after(&text, keyword))
        .map(|number| (WorkItemKind::Pr, number))
        .or_else(|| number_after(&text, "issue").map(|number| (WorkItemKind::Issue, number)))
}

/// MonoCode `referencesCurrentPr`: "this PR", "the pull request", "current PR".
fn references_current_pr(message: &str) -> bool {
    let text = normalized(message);
    let words: Vec<&str> = text.split(|c: char| !is_word(c)).collect();
    words.windows(2).enumerate().any(|(ix, pair)| {
        ["this", "the", "current"].contains(&pair[0])
            && (pair[1] == "pr" || (pair[1] == "pull" && words.get(ix + 2) == Some(&"request")))
    })
}

/// The item a first message names that needs `gh` to resolve: blocking,
/// for the background executor. `None` when the message names none.
fn resolve_with_gh(message: &str, cwd: &str) -> Option<LinkedWorkItem> {
    let cwd = Path::new(cwd);
    if let Some((kind, number)) = explicit_hint(message) {
        let repo = crate::github::repositories(cwd)
            .map_err(|err| log::debug!("no GitHub repository for a link: {err}"))
            .ok()?
            .into_iter()
            .next()?;
        return LinkedWorkItem::new(kind, &repo, number);
    }
    if !references_current_pr(message) {
        return None;
    }
    #[derive(Deserialize)]
    struct View {
        url: String,
    }
    let json = crate::github::gh(cwd, &["pr", "view", "--json", "url"])
        .map_err(|err| log::debug!("no pull request for the branch: {err}"))
        .ok()?;
    let view: View = serde_json::from_str(&json)
        .map_err(|err| log::warn!("unreadable `gh pr view`: {err}"))
        .ok()?;
    LinkedWorkItem::parse_url(&view.url)
}

/// The link for a listed item: its URL, checked and rebuilt.
pub(crate) fn link_for(item: &WorkItem) -> Option<LinkedWorkItem> {
    match item.provider {
        Provider::Backlog => LinkedWorkItem::parse_backlog_url(&item.url, None),
        Provider::GitHub => {
            let kind = match item.kind {
                crate::github::Kind::Pr => WorkItemKind::Pr,
                crate::github::Kind::Issue => WorkItemKind::Issue,
            };
            LinkedWorkItem::new(kind, &item.repo, item.number)
        }
    }
}

/// The Backlog issue each first message names on `host`, by thread id:
/// for the background executor.
fn backlog_links(
    firsts: Vec<(String, std::sync::Arc<Block>)>,
    host: &str,
) -> Vec<(String, LinkedWorkItem)> {
    firsts
        .into_iter()
        .filter_map(|(id, block)| {
            let text = block.text.as_deref()?;
            Some((id, LinkedWorkItem::parse_backlog_url(text, Some(host))?))
        })
        .collect()
}

impl BenCodeApp {
    /// The connected Backlog space's host, once known.
    fn backlog_host(&self) -> Option<String> {
        self.backlog.account.as_ref().map(|a| a.space.clone())
    }

    /// Unlinked threads in memory whose first message holds an issue URL
    /// of the connected Backlog space are linked to it. Each thread is
    /// looked at once a launch, the search runs off the UI thread, and a
    /// link made meanwhile (or the Link dialog open on it) wins.
    pub(crate) fn backfill_backlog_links(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.backlog_host() else {
            return;
        };
        let mut firsts = Vec::new();
        for session in &self.sessions {
            if session.linked_work_item.is_some()
                || !self.item_threads.searched.insert(session.id.clone())
            {
                continue;
            }
            if let Some(block) = session.blocks.iter().find(|b| b.role == "user") {
                firsts.push((session.id.clone(), block.clone()));
            }
        }
        if firsts.is_empty() {
            return;
        }
        let task = cx
            .background_executor()
            .spawn(async move { backlog_links(firsts, &host) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let landed = this.update(cx, |app, cx| {
                for (id, item) in found {
                    let unlinked = (app.sessions.iter())
                        .find(|s| s.id == id)
                        .is_some_and(|s| s.linked_work_item.is_none());
                    let editing = app.link_dialog.as_ref().is_some_and(|d| d.session_id == id);
                    if unlinked && !editing && !app.is_mono_session(&id) {
                        app.set_session_link(&id, Some(item), cx);
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("backlog links found after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// After a thread's first message is sent: links what it names, unless
    /// the thread is linked already (from the Inbox, or by hand).
    pub(crate) fn link_first_work_item(
        &mut self,
        session_id: &str,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let first = session.blocks.iter().filter(|b| b.role == "user").count() == 1;
        if !first || session.linked_work_item.is_some() || self.is_mono_session(session_id) {
            return;
        }
        let host = self.backlog_host();
        if let Some(item) = LinkedWorkItem::parse_any_url(message, host.as_deref()) {
            self.set_session_link(session_id, Some(item), cx);
            return;
        }
        if explicit_hint(message).is_none() && !references_current_pr(message) {
            return;
        }
        let (id, message, cwd) = (
            session_id.to_string(),
            message.to_string(),
            session.work_dir().to_string(),
        );
        let task = cx
            .background_executor()
            .spawn(async move { resolve_with_gh(&message, &cwd) });
        cx.spawn(async move |this, cx| {
            let Some(item) = task.await else {
                return;
            };
            let landed = this.update(cx, |app, cx| {
                let unlinked = app
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .is_some_and(|s| s.linked_work_item.is_none());
                // The Link dialog open on the thread has the last word.
                let editing = app.link_dialog.as_ref().is_some_and(|d| d.session_id == id);
                if unlinked && !editing {
                    app.set_session_link(&id, Some(item), cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("work item resolved after app drop: {err:#}");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_messages_link_to_the_connected_spaces_issues() {
        let block = |text: &str| std::sync::Arc::new(Block::new("u", "user", text));
        let firsts = vec![
            (
                "a".to_string(),
                block("Implement https://acme.backlog.com/view/WEB-42 please"),
            ),
            (
                "b".to_string(),
                block("See https://other.backlog.com/view/WEB-43"),
            ),
            ("c".to_string(), block("Fix the rail")),
        ];
        let found = backlog_links(firsts, "acme.backlog.com");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "a");
        assert_eq!(found[0].1.item_key(), "backlog:web-42");
    }

    #[test]
    fn listed_items_link_to_themselves() {
        let backlog = WorkItem {
            provider: Provider::Backlog,
            identifier: "WEB-42".into(),
            url: "https://acme.backlog.com/view/WEB-42".into(),
            ..Default::default()
        };
        assert_eq!(link_for(&backlog).unwrap().item_key(), backlog.key());
        let github = WorkItem {
            kind: crate::github::Kind::Pr,
            repo: "Acme/App".into(),
            number: 7,
            ..Default::default()
        };
        assert_eq!(link_for(&github).unwrap().item_key(), github.key());
    }

    #[test]
    fn numbered_references_name_their_kind() {
        assert_eq!(
            explicit_hint("look at PR #42 please"),
            Some((WorkItemKind::Pr, 42))
        );
        assert_eq!(
            explicit_hint("pull\n  request 7"),
            Some((WorkItemKind::Pr, 7))
        );
        assert_eq!(
            explicit_hint("see Issue#3, then PR 4"),
            Some((WorkItemKind::Pr, 4)),
            "a pull request wins"
        );
        assert_eq!(
            explicit_hint("fix issue 12."),
            Some((WorkItemKind::Issue, 12))
        );
        for text in [
            "PR #0",
            "issue tracker",
            "approach 12",
            "prs 5",
            "spr 5",
            "issue 7a",
            "Fix the sidebar",
        ] {
            assert_eq!(explicit_hint(text), None, "{text}");
        }
    }

    #[test]
    fn the_current_pull_request_is_named_by_a_determiner() {
        assert!(references_current_pr("review the current PR"));
        assert!(references_current_pr("land this pull request"));
        assert!(references_current_pr("What does the PR change?"));
        assert!(!references_current_pr("open a PR for this"));
        assert!(!references_current_pr("this practice"));
        assert!(!references_current_pr("the pull of gravity"));
    }
}
