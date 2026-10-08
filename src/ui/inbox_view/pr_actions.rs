//! MonoCode `GithubPrActions`: merge (with its method), convert to draft
//! or mark ready, close or reopen a pull request. Each asks first, runs
//! through `gh`, then the item is read back into the list.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::menus::{Menu, MenuItem, SplitButton};
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div};

use crate::app::BenCodeApp;
use crate::github::{self, PrAction, WorkItem};
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// MonoCode `GITHUB_PR_MERGE_OPTIONS`.
const MERGE_OPTIONS: [(PrAction, &str); 3] = [
    (PrAction::Merge, "Create a merge commit"),
    (PrAction::Squash, "Squash and merge"),
    (PrAction::Rebase, "Rebase and merge"),
];

/// The selected pull request's action state.
#[derive(Clone, Debug)]
pub struct PrActionUi {
    pub merge_method: PrAction,
    /// The action waiting for confirmation, with the item it is for.
    pub confirm: Option<(String, PrAction)>,
    pub busy: Option<PrAction>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl Default for PrActionUi {
    fn default() -> Self {
        Self {
            merge_method: PrAction::Merge,
            confirm: None,
            busy: None,
            error: None,
            notice: None,
        }
    }
}

/// MonoCode `githubPrActionCopy`: title, detail, confirm and progress.
fn action_copy(
    action: PrAction,
    base: &str,
    head: &str,
) -> (&'static str, String, &'static str, &'static str) {
    let source = if head.is_empty() {
        "this branch".to_string()
    } else {
        format!("“{head}”")
    };
    let destination = if base.is_empty() {
        "the base branch".to_string()
    } else {
        format!("“{base}”")
    };
    match action {
        PrAction::Merge => (
            "Merge this pull request?",
            format!(
                "Every commit from {source} will be added to {destination} with a merge commit."
            ),
            "Merge pull request",
            "Merging…",
        ),
        PrAction::Squash => (
            "Squash and merge?",
            format!("The commits from {source} will be combined into one commit on {destination}."),
            "Squash and merge",
            "Merging…",
        ),
        PrAction::Rebase => (
            "Rebase and merge?",
            format!("The commits from {source} will be rebased individually onto {destination}."),
            "Rebase and merge",
            "Merging…",
        ),
        PrAction::Draft => (
            "Convert to draft?",
            "Reviewers will see that this pull request is not ready to merge.".into(),
            "Convert to draft",
            "Converting…",
        ),
        PrAction::Ready => (
            "Mark as ready for review?",
            "Reviewers will see that this pull request is ready for feedback.".into(),
            "Ready for review",
            "Updating…",
        ),
        PrAction::Close => (
            "Close this pull request?",
            "The pull request will close without merging. You can reopen it later.".into(),
            "Close pull request",
            "Closing…",
        ),
        PrAction::Reopen => (
            "Reopen this pull request?",
            "The pull request will return to the open state.".into(),
            "Reopen pull request",
            "Reopening…",
        ),
    }
}

impl BenCodeApp {
    fn ask_pr_action(&mut self, key: &str, action: PrAction, cx: &mut Context<Self>) {
        let pr = &mut self.inbox.pr;
        if pr.busy.is_some() {
            return;
        }
        pr.error = None;
        pr.notice = None;
        pr.confirm = Some((key.to_string(), action));
        cx.notify();
    }

    fn run_pr_action(&mut self, cx: &mut Context<Self>) {
        let Some((key, action)) = self.inbox.pr.confirm.take() else {
            return;
        };
        let Some(item) = self.inbox.item(&key).cloned() else {
            return;
        };
        self.inbox.pr.busy = Some(action);
        let task = cx.background_executor().spawn(async move {
            github::pr_action(
                std::path::Path::new(&item.project),
                &item.repo,
                item.number,
                action,
            )
            .map(|next| WorkItem {
                project: item.project.clone(),
                ..next
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                app.inbox.pr.busy = None;
                match result {
                    Ok(next) => {
                        // MonoCode: a merge that did not land was queued.
                        app.inbox.pr.notice = (action.is_merge()
                            && !next.state.eq_ignore_ascii_case("merged"))
                        .then(|| "Merge queued or auto-merge enabled.".to_string());
                        if let Some(slot) = app.inbox.items.iter_mut().find(|i| i.key() == key) {
                            *slot = next;
                        }
                        app.inbox.details.value.remove(&key);
                        app.inbox.checks.value.remove(&key);
                        app.load_inbox_item(&key, cx);
                    }
                    Err(err) => {
                        log::warn!("pull request action failed: {err}");
                        app.inbox.pr.error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("pull request action after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// The buttons for the pull request's state (MonoCode shows merge and
    /// draft for an open one, ready for a draft, reopen for a closed one).
    pub(super) fn render_pr_actions(
        &self,
        item: &WorkItem,
        _base: &str,
        _head: &str,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let state = item.state.to_lowercase();
        let busy = self.inbox.pr.busy.is_some();
        let key = item.key();
        let button = |id: &'static str, label: &'static str, icon: IconName, action: PrAction| {
            let key = key.clone();
            Button::new(id, label)
                .variant(ButtonVariant::Ghost)
                .icon(icon)
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.ask_pr_action(&key, action, cx)))
                .into_any_element()
        };
        let mut out = Vec::new();
        if state == "open" && !item.draft {
            let method = self.inbox.pr.merge_method;
            let label = match method {
                PrAction::Merge => "Merge pull request",
                PrAction::Squash => "Squash and merge",
                _ => "Rebase and merge",
            };
            let merge_key = key.clone();
            let methods = MERGE_OPTIONS
                .iter()
                .fold(Menu::new(), |menu, (action, label)| {
                    let action = *action;
                    menu.item(
                        MenuItem::radio(*label, action == method).on_click(app_callback(
                            cx,
                            move |this, cx| {
                                this.inbox.pr.merge_method = action;
                                cx.notify();
                            },
                        )),
                    )
                });
            // The method sits under the merge button's arrow, as on GitHub;
            // a press while busy is ignored by `ask_pr_action`.
            out.push(
                SplitButton::new("inbox-pr-merge", label, methods)
                    .variant(ButtonVariant::Primary)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_pr_action(&merge_key, this.inbox.pr.merge_method, cx)
                    }))
                    .into_any_element(),
            );
            out.push(button(
                "inbox-pr-draft",
                "Convert to draft",
                IconName::GitPullRequestDraft,
                PrAction::Draft,
            ));
        }
        if state == "open" && item.draft {
            out.push(button(
                "inbox-pr-ready",
                "Ready for review",
                IconName::GitPullRequest,
                PrAction::Ready,
            ));
        }
        if state == "open" {
            out.push(button(
                "inbox-pr-close",
                "Close pull request",
                IconName::GitPullRequestClosed,
                PrAction::Close,
            ));
        }
        if state == "closed" {
            out.push(button(
                "inbox-pr-reopen",
                "Reopen pull request",
                IconName::GitPullRequest,
                PrAction::Reopen,
            ));
        }
        out
    }

    /// "Merging…", the failure, or MonoCode's merge-queued notice.
    pub(super) fn render_pr_action_status(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let colors = &cx.theme().colors;
        let pr = &self.inbox.pr;
        let (text, color) = if let Some(action) = pr.busy {
            (
                action_copy(action, "", "").3.to_string(),
                colors.fg.opacity(0.55),
            )
        } else if let Some(error) = &pr.error {
            (error.clone(), colors.danger.opacity(0.9))
        } else {
            (pr.notice.clone()?, colors.fg.opacity(0.55))
        };
        Some(
            div()
                .text_size(px(11.0))
                .text_color(color)
                .child(SharedString::from(text))
                .into_any_element(),
        )
    }

    /// The confirmation, over the window (MonoCode's confirm popover).
    pub fn render_pr_action_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (key, action) = self.inbox.pr.confirm.clone()?;
        let details = self.inbox.details.get(&key).and_then(|d| d.as_ref().ok());
        let (base, head) = details
            .map(|d| (d.base_ref.as_str(), d.head_ref.as_str()))
            .unwrap_or_default();
        let (title, detail, confirm, _) = action_copy(action, base, head);
        let cancel = app_callback(cx, |this, cx| {
            this.inbox.pr.confirm = None;
            cx.notify();
        });
        let run = app_callback(cx, |this, cx| this.run_pr_action(cx));
        let dialog = ConfirmDialog::new("inbox-pr-confirm", title, detail, cancel)
            .confirm(confirm)
            .on_confirm(run);
        let dialog = if action == PrAction::Close {
            dialog.destructive()
        } else {
            dialog
        };
        Some(dialog.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names_the_branches() {
        let (title, detail, confirm, progress) = action_copy(PrAction::Squash, "main", "fix");
        assert_eq!(title, "Squash and merge?");
        assert_eq!(
            detail,
            "The commits from “fix” will be combined into one commit on “main”."
        );
        assert_eq!((confirm, progress), ("Squash and merge", "Merging…"));
        assert!(
            action_copy(PrAction::Merge, "", "")
                .1
                .contains("this branch")
        );
    }
}
