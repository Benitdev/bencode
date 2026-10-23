//! MonoCode `InboxComments` and `InboxCommentForm`: the item's latest
//! comments, reviews and review threads (with their replies), a pull
//! request's commits, and a box to comment or reply to a review thread.

use super::selection::comment_block;
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::ui::transcript::selection::SegCtx;
use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*,
};

use crate::ui::scale::px;

use super::relative_time;
use crate::app::BenCodeApp;
use crate::github::{self, Comment, Kind, WorkItem};
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::work_items::Provider;

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

/// The colour of a review state (a comment's, or a pull request's
/// decision); `None` keeps the row's muted text.
pub(super) fn review_tint(
    state: &str,
    colors: &ely_gpui_component::theme::Palette,
) -> Option<Hsla> {
    match state {
        "APPROVED" => Some(colors.success),
        "CHANGES_REQUESTED" => Some(colors.danger),
        _ => None,
    }
}

/// The `·` between the parts of a byline.
pub(super) fn byline_dot(fg: Hsla) -> Div {
    div().text_color(fg.ink(Ink::Faint)).child("·")
}

/// A byline: muted 12px parts in a wrapping row.
pub(super) fn byline(fg: Hsla) -> Div {
    div()
        .flex()
        .flex_wrap()
        .min_w_0()
        .items_center()
        .gap_x_2()
        .gap_y_1()
        .text_size(Text::Base.px())
        .text_color(fg.ink(Ink::Secondary))
}

/// Who wrote it, stronger than the rest of the byline.
pub(super) fn author_name(name: impl Into<SharedString>, fg: Hsla) -> Div {
    div()
        .text_color(fg.ink(Ink::Strong))
        .font_weight(FontWeight::MEDIUM)
        .child(name.into())
}

/// The bordered card a comment sits in.
pub(super) fn comment_card(fg: Hsla) -> Div {
    card_fill(fg).rounded(px(6.0)).border_1()
}

/// A card's border colour and fill, for one drawn in parts (the
/// description, a row per block).
pub(super) fn card_fill(fg: Hsla) -> Div {
    div()
        .border_color(fg.fill(Fill::Strong))
        .bg(fg.opacity(0.02))
}

impl BenCodeApp {
    /// Posts the comment box (MonoCode `onSubmit`); the thread reloads.
    pub fn post_inbox_comment(&mut self, cx: &mut Context<Self>) {
        let body = self.inbox_comment_input.read(cx).text().trim().to_string();
        if body.is_empty() || self.inbox(cx).comment_posting {
            return;
        }
        let Some(item) = self
            .inbox(cx)
            .selected
            .as_deref()
            .and_then(|key| self.inbox(cx).item(key))
            .cloned()
        else {
            return;
        };
        let reply = self
            .inbox(cx)
            .reply_to
            .as_ref()
            .map(|r| r.thread_id.clone());
        self.inbox_state.as_mut(cx).comment_posting = true;
        self.inbox_state.as_mut(cx).comment_error = None;
        let task = cx.background_executor().spawn(async move {
            match item.provider {
                Provider::GitHub => github::post_comment(
                    std::path::Path::new(&item.project),
                    &item.repo,
                    item.kind,
                    item.number,
                    &body,
                    reply.as_deref(),
                ),
                Provider::Backlog => crate::backlog::post_comment(&item.identifier, &body),
            }
            .map(|_| item.key())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |app, cx| {
                app.inbox_state.as_mut(cx).comment_posting = false;
                match result {
                    Ok(key) => {
                        // The user's own comment is not news to them.
                        app.mark_inbox_key_seen(&key, cx);
                        app.inbox_state.as_mut(cx).reply_to = None;
                        app.inbox_comment_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        app.load_inbox_thread(&key, true, cx);
                    }
                    Err(err) => {
                        log::warn!("could not post comment: {err}");
                        app.inbox_state.as_mut(cx).comment_error = Some(err);
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

    /// The conversation's rule and its count, or its loading or error;
    /// the comments, commits and comment box follow as rows of their own.
    pub(super) fn render_conversation_head(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let loading = self.inbox(cx).threads.is_loading(&key);
        let section = div().border_t_1().border_color(colors.border);
        let thread = match self.inbox(cx).threads.get(&key) {
            Some(Ok(thread)) => thread,
            Some(Err(err)) => {
                return section.pt_5().child(
                    div()
                        .text_size(Text::Base.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(SharedString::from(err.clone())),
                );
            }
            None => {
                return section.pt_5().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(Text::Base.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(super::loading_icon(
                            "inbox-comments-loading",
                            fg.ink(Ink::Muted),
                        ))
                        .child("Loading comments"),
                );
            }
        };
        // Nothing to count: the comment box sits as close as a count would.
        if thread.comments.is_empty() && !thread.truncated {
            return section.pt_2();
        }
        let count: usize = thread.comments.iter().map(|c| 1 + c.replies.len()).sum();
        let label = if count == 1 {
            "1 comment".to_string()
        } else {
            format!("{count} comments")
        };
        section.pt_5().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Secondary))
                .child(div().text_color(fg.ink(Ink::Body)).child(label))
                .when(thread.truncated, |el| {
                    el.child(format!(
                        "Latest comments · more on {}",
                        item.provider.label()
                    ))
                })
                .when(loading, |el| {
                    el.child(super::loading_icon(
                        "inbox-comments-reloading",
                        fg.ink(Ink::Faint),
                    ))
                }),
        )
    }

    /// The thread's comment `ix`, once loaded.
    pub(super) fn render_thread_comment(
        &self,
        item: &WorkItem,
        ix: usize,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let Some(Ok(thread)) = self.inbox(cx).threads.get(&item.key()) else {
            return None;
        };
        let comment = thread.comments.get(ix)?;
        let now = crate::app::now_ms();
        let select = self.inbox_seg_ctx(&item.key(), comment_block(ix, None), cx);
        Some(self.render_comment(ix, comment, select, false, item.kind == Kind::Pr, now, cx))
    }

    /// A pull request's commits, once loaded.
    pub(super) fn render_thread_commits(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let Some(Ok(thread)) = self.inbox(cx).threads.get(&item.key()) else {
            return None;
        };
        let now = crate::app::now_ms();
        Some(self.render_commits(&thread.commits, now, cx))
    }

    fn render_comment(
        &self,
        ix: usize,
        comment: &Comment,
        select: SegCtx,
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
        let dot = || byline_dot(fg);
        let mut header = byline(fg)
            .when(!nested, |el| el.px_3().py_2())
            .child(author_name(
                if comment.author.is_empty() {
                    "ghost".to_string()
                } else {
                    comment.author.clone()
                },
                fg,
            ));
        if !review.is_empty() {
            let tint = review_tint(&comment.state, colors).unwrap_or(fg.ink(Ink::Secondary));
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
                    .text_size(Text::Sm.px())
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
                        this.inbox_state.as_mut(cx).reply_to = Some(target.clone());
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
                .text_size(Text::Md.px())
                .child(
                    AgentMarkdown::new(
                        SharedString::from(format!("comment-{}", comment.id)),
                        comment.body.clone(),
                    )
                    .selectable(select.clone()),
                )
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
                    let select = SegCtx {
                        block: comment_block(ix, Some(r)),
                        ..select.clone()
                    };
                    self.render_comment(ix * 100 + r + 1, reply, select, true, pr, now, cx)
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
        comment_card(fg)
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
            .child(
                div()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Body))
                    .child(if commits.len() == 1 {
                        "1 commit".to_string()
                    } else {
                        format!("{} commits", commits.len())
                    }),
            )
            .children(commits.iter().enumerate().map(|(ix, commit)| {
                let url = commit.url.clone();
                let hover = fg.fill(Fill::Hover);
                div()
                    .id(("commit", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(Text::Base.px())
                    .when(!url.is_empty(), |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    })
                    .child(
                        Icon::new(IconName::GitCommitHorizontal)
                            .size(IconSize::Xs)
                            .color(fg.ink(Ink::Muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(fg.ink(Ink::Strong))
                            .child(SharedString::from(commit.headline.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(cx.theme().mono_family.clone())
                            .text_size(Text::Sm.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child(commit.oid.chars().take(7).collect::<String>()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(fg.ink(Ink::Muted))
                            .child(format!(
                                "{} · {}",
                                commit.author,
                                relative_time(&commit.committed_at, now)
                            )),
                    )
            }))
    }

    /// MonoCode `InboxCommentForm`.
    pub(super) fn render_comment_form(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let posting = self.inbox(cx).comment_posting;
        let typed = !self.inbox_comment_input.read(cx).text().trim().is_empty();
        let reply = self.inbox(cx).reply_to.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(reply.as_ref().map(|reply| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(
                        Icon::new(IconName::Reply)
                            .size(IconSize::Xs)
                            .color(fg.ink(Ink::Secondary)),
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
                            .text_color(fg.ink(Ink::Muted))
                            .hover(move |s| s.text_color(fg))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inbox_state.as_mut(cx).reply_to = None;
                                cx.notify();
                            }))
                            .child("Cancel"),
                    )
            }))
            .child(
                div()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(fg.fill(Fill::Strong))
                    .bg(fg.fill(Fill::Hover))
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(Text::Md.px())
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
            .children(self.inbox(cx).comment_error.clone().map(|error| {
                div()
                    .text_size(Text::Base.px())
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
