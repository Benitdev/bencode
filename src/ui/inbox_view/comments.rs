//! MonoCode `InboxComments` and `InboxCommentForm`: the item's latest
//! comments, reviews and review threads (with their replies), a pull
//! request's commits, and a box to comment or reply to a review thread.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use crate::ui::scale::px;

use super::relative_time;
use crate::app::BenCodeApp;
use crate::github::{self, Comment, Kind, WorkItem};

/// The review thread a reply goes to (MonoCode `InboxReplyTarget`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyTarget {
    pub thread_id: String,
    pub author: String,
}

/// MonoCode `githubReviewStateLabel`.
fn review_label(state: &str) -> &'static str {
    match state {
        "APPROVED" => "Approved",
        "CHANGES_REQUESTED" => "Requested changes",
        "COMMENTED" => "Commented",
        "DISMISSED" => "Dismissed",
        _ => "",
    }
}

impl BenCodeApp {
    /// Posts the comment box (MonoCode `onSubmit`); the thread reloads.
    pub fn post_inbox_comment(&mut self, cx: &mut Context<Self>) {
        let body = self.inbox_comment_input.read(cx).text().trim().to_string();
        if body.is_empty() || self.inbox.comment_posting {
            return;
        }
        let Some(item) = self
            .inbox
            .selected
            .as_deref()
            .and_then(|key| self.inbox.item(key))
            .cloned()
        else {
            return;
        };
        let reply = self.inbox.reply_to.as_ref().map(|r| r.thread_id.clone());
        self.inbox.comment_posting = true;
        self.inbox.comment_error = None;
        let task = cx.background_executor().spawn(async move {
            github::post_comment(
                std::path::Path::new(&item.project),
                &item.repo,
                item.kind,
                item.number,
                &body,
                reply.as_deref(),
            )
            .map(|_| item.key())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                app.inbox.comment_posting = false;
                match result {
                    Ok(key) => {
                        app.inbox.reply_to = None;
                        app.inbox_comment_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        app.load_inbox_thread(&key, true, cx);
                    }
                    Err(err) => {
                        log::warn!("could not post comment: {err}");
                        app.inbox.comment_error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("comment posted after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// The conversation section and the comment box.
    pub(super) fn render_inbox_conversation(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let loading = self.inbox.threads.is_loading(&key);
        let section = div()
            .mt_4()
            .pt_5()
            .border_t_1()
            .border_color(colors.border)
            .flex()
            .flex_col()
            .gap_3();
        let thread = match self.inbox.threads.get(&key) {
            Some(Ok(thread)) => Some(thread),
            Some(Err(err)) => {
                return section
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.45))
                            .child(SharedString::from(err.clone())),
                    )
                    .child(self.render_comment_form(cx))
                    .into_any_element();
            }
            None => None,
        };
        let Some(thread) = thread else {
            return section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.45))
                        .child(
                            Icon::new(IconName::LoaderCircle)
                                .size(IconSize::Xs)
                                .color(fg.opacity(0.45)),
                        )
                        .child("Loading comments"),
                )
                .into_any_element();
        };
        let count: usize = thread.comments.iter().map(|c| 1 + c.replies.len()).sum();
        let label = if count == 1 {
            "1 comment".to_string()
        } else {
            format!("{count} comments")
        };
        let now = crate::app::now_ms();
        let pr = item.kind == Kind::Pr;
        section
            .when(!thread.comments.is_empty() || thread.truncated, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.5))
                        .child(div().text_color(fg.opacity(0.7)).child(label))
                        .when(thread.truncated, |el| {
                            el.child("Latest comments · more on GitHub")
                        })
                        .when(loading, |el| {
                            el.child(
                                Icon::new(IconName::LoaderCircle)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.35)),
                            )
                        }),
                )
                .children(
                    thread
                        .comments
                        .iter()
                        .enumerate()
                        .map(|(ix, c)| self.render_comment(ix, c, false, pr, now, cx)),
                )
            })
            .when(pr && !thread.commits.is_empty(), |el| {
                el.child(self.render_commits(&thread.commits, now, cx))
            })
            .child(self.render_comment_form(cx))
            .into_any_element()
    }

    fn render_comment(
        &self,
        ix: usize,
        comment: &Comment,
        nested: bool,
        pr: bool,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let time = relative_time(&comment.created_at, now);
        let review = review_label(&comment.state);
        let location = match (comment.path.is_empty(), comment.line) {
            (true, _) => String::new(),
            (false, Some(line)) => format!("{}:{line}", comment.path),
            (false, None) => comment.path.clone(),
        };
        let dot = || div().text_color(fg.opacity(0.35)).child("·");
        let mut header = div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .items_center()
            .gap_2()
            .text_size(px(12.0))
            .text_color(fg.opacity(0.5))
            .when(!nested, |el| el.px_3().py_2())
            .child(
                div()
                    .text_color(fg.opacity(0.8))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if comment.author.is_empty() {
                        "ghost".to_string()
                    } else {
                        comment.author.clone()
                    }),
            );
        if !review.is_empty() {
            let tint = match comment.state.as_str() {
                "APPROVED" => colors.success,
                "CHANGES_REQUESTED" => colors.danger,
                _ => fg.opacity(0.5),
            };
            header = header
                .child(dot())
                .child(div().text_color(tint).child(review));
        }
        if !location.is_empty() {
            header = header.child(dot()).child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.theme().mono_family.clone())
                    .text_size(px(11.0))
                    .child(location),
            );
        }
        if comment.resolved && !nested {
            header = header.child(dot()).child(
                div()
                    .text_color(colors.success.opacity(0.8))
                    .child("Resolved"),
            );
        }
        if !time.is_empty() {
            let url = comment.url.clone();
            header = header.child(dot()).child(
                div()
                    .id(("comment-time", ix))
                    .when(!url.is_empty(), |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.text_color(fg))
                            .tooltip(Tooltip::text("Open on GitHub"))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    })
                    .child(time),
            );
        }
        // MonoCode replies in a review thread on GitHub.
        if pr && !nested && !comment.thread_id.is_empty() {
            let target = ReplyTarget {
                thread_id: comment.thread_id.clone(),
                author: comment.author.clone(),
            };
            header = header.child(dot()).child(
                div()
                    .id(("comment-reply", ix))
                    .cursor_pointer()
                    .hover(move |s| s.text_color(fg))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.inbox.reply_to = Some(target.clone());
                        crate::ui::composer::focus_later(
                            gpui::Focusable::focus_handle(this.inbox_comment_input.read(cx), cx),
                            cx,
                        );
                        cx.notify();
                    }))
                    .child("Reply"),
            );
        }
        let has_body = !comment.body.trim().is_empty();
        let body = has_body.then(|| {
            div()
                .when(!nested, |el| el.px_3().py_2())
                .text_size(px(13.0))
                .child(MarkdownRenderer::new(
                    SharedString::from(format!("comment-{}", comment.id)),
                    comment.body.clone(),
                ))
        });
        let replies = (!nested && !comment.replies.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(colors.border)
                .children(comment.replies.iter().enumerate().map(|(r, reply)| {
                    self.render_comment(ix * 100 + r + 1, reply, true, pr, now, cx)
                }))
        });
        if nested {
            return div()
                .flex()
                .flex_col()
                .gap_1()
                .child(header)
                .children(body)
                .into_any_element();
        }
        div()
            .rounded(px(6.0))
            .border_1()
            .border_color(fg.opacity(0.10))
            .bg(fg.opacity(0.02))
            .flex()
            .flex_col()
            .child(header.when(has_body || replies.is_some(), |el| {
                el.border_b_1().border_color(colors.border)
            }))
            .children(body)
            .children(replies)
            .into_any_element()
    }

    fn render_commits(
        &self,
        commits: &[github::Commit],
        now: i64,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(px(12.0)).text_color(fg.opacity(0.7)).child(
                if commits.len() == 1 {
                    "1 commit".to_string()
                } else {
                    format!("{} commits", commits.len())
                },
            ))
            .children(commits.iter().enumerate().map(|(ix, commit)| {
                let url = commit.url.clone();
                let hover = fg.opacity(0.05);
                div()
                    .id(("commit", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .when(!url.is_empty(), |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    })
                    .child(
                        Icon::new(IconName::GitCommitHorizontal)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.45)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(fg.opacity(0.85))
                            .child(SharedString::from(commit.headline.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(cx.theme().mono_family.clone())
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .child(commit.oid.chars().take(7).collect::<String>()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(fg.opacity(0.45))
                            .child(format!(
                                "{} · {}",
                                commit.author,
                                relative_time(&commit.committed_at, now)
                            )),
                    )
            }))
    }

    /// MonoCode `InboxCommentForm`.
    fn render_comment_form(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let posting = self.inbox.comment_posting;
        let typed = !self.inbox_comment_input.read(cx).text().trim().is_empty();
        let reply = self.inbox.reply_to.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(reply.as_ref().map(|reply| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.55))
                    .child(
                        Icon::new(IconName::Reply)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.5)),
                    )
                    .child(format!(
                        "Replying to {}",
                        if reply.author.is_empty() {
                            "comment"
                        } else {
                            &reply.author
                        }
                    ))
                    .child(
                        div()
                            .id("comment-cancel-reply")
                            .cursor_pointer()
                            .text_color(fg.opacity(0.45))
                            .hover(move |s| s.text_color(fg))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inbox.reply_to = None;
                                cx.notify();
                            }))
                            .child("Cancel"),
                    )
            }))
            .child(
                div()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(fg.opacity(0.10))
                    .bg(fg.opacity(0.05))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(13.0))
                            .when(posting, |el| el.opacity(0.4))
                            .child(self.inbox_comment_input.clone()),
                    )
                    .child(
                        div().flex().justify_end().px_2().pb_2().child(
                            Button::new(
                                "inbox-comment-post",
                                if posting {
                                    "Posting..."
                                } else if reply.is_some() {
                                    "Reply"
                                } else {
                                    "Comment"
                                },
                            )
                            .variant(ButtonVariant::Primary)
                            .size(ControlSize::Sm)
                            .disabled(posting || !typed)
                            .on_click(cx.listener(|this, _, _, cx| this.post_inbox_comment(cx))),
                        ),
                    ),
            )
            .children(self.inbox.comment_error.clone().map(|error| {
                div()
                    .text_size(px(12.0))
                    .text_color(colors.danger.opacity(0.9))
                    .child(error)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_states_read_like_monocode() {
        assert_eq!(review_label("CHANGES_REQUESTED"), "Requested changes");
        assert_eq!(review_label("APPROVED"), "Approved");
        assert_eq!(review_label(""), "");
    }
}
