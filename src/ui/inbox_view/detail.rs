//! MonoCode `InboxDetail`: identity, title, people and times, the actions
//! (Send to agent with its project, pull request actions or a Backlog
//! issue's status, Open in its tracker), labels, the body, then a pull
//! request's checks and the conversation. A pull request's Files changed
//! section shows its patch in the review list (`ui/diff_viewer.rs`).
//!
//! The rest is a virtual list of rows (`DetailRow`): a scroll frame draws
//! the rows in view, the description a row per block (`blocks.rs`).

use std::ops::Range;

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::Choice;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::navigation::Tabs;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, IntoElement, ParentElement, SharedString,
    Styled, div, list, prelude::*,
};

use crate::ui::scale::px;

use super::comments::{author_name, byline, byline_dot, card_fill, review_tint};
use super::{PrTab, kind_label, project_name, relative_time, source_label, status_mark};
use crate::app::BenCodeApp;
use crate::github::{Kind, WorkItem};
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::composer::inbox_card::label_chip;
use crate::ui::diff_viewer::pr_doc_key;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::work_items::Provider;

/// A row of the detail's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailRow {
    /// Identity, title, people, actions and labels.
    Head,
    /// The description card's byline.
    DescriptionTop,
    /// A block of the description, or its one loading, error or empty note.
    Description(usize),
    Checks,
    /// The conversation's rule and count, or its loading or error.
    ConversationHead,
    Comment(usize),
    Commits,
    CommentForm,
}

/// What became of `old` in `new`: the range replaced and how many rows
/// took its place. Rows kept at either end keep their measured heights.
fn changed(old: &[DetailRow], new: &[DetailRow]) -> (Range<usize>, usize) {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = (old[head..].iter().rev())
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (head..old.len() - tail, new.len() - head - tail)
}

impl BenCodeApp {
    pub(super) fn render_inbox_detail(&self, cx: &Context<Self>) -> AnyElement {
        let Some(item) = self
            .inbox(cx)
            .selected
            .as_deref()
            .and_then(|key| self.inbox(cx).item(key))
        else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(EmptyState::new(
                    "inbox-none",
                    IconName::Inbox,
                    "Select an inbox item",
                ))
                .into_any_element();
        };
        // The review scrolls on its own (a virtual list), under a header
        // that stays.
        let colors = &cx.theme().colors;
        if item.kind == Kind::Pr && self.inbox(cx).pr_tab == PrTab::Files {
            let column = || div().w_full().max_w(px(1024.0)).mx_auto().px_8();
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    column()
                        .flex_none()
                        .pt_4()
                        .child(self.render_inbox_head(item, cx)),
                )
                .child(
                    column().flex_1().min_h_0().pt_3().pb_4().child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.border)
                            .overflow_hidden()
                            .child(self.render_diff_doc(&pr_doc_key(item), cx)),
                    ),
                )
                .into_any_element();
        }

        let rows = self.inbox_detail_rows(item, cx);
        let inbox = self.inbox(cx);
        let key = item.key();
        let selected_key = key.clone();
        // The comment box stays drawn while it has the keyboard, scrolled
        // out of view or not.
        let input = gpui::Focusable::focus_handle(self.inbox_comment_input.read(cx), cx);
        let handles = |rows: &[DetailRow]| {
            (rows.iter())
                .map(|row| (*row == DetailRow::CommentForm).then(|| input.clone()))
                .collect::<Vec<_>>()
        };
        {
            let mut shown = inbox.detail_rows.borrow_mut();
            if shown.0 != key {
                inbox.detail_list.reset(0);
                inbox.detail_list.splice_focusable(0..0, handles(&rows));
                *shown = (key, rows);
            } else if shown.1 != rows {
                let (range, count) = changed(&shown.1, &rows);
                let added = range.start..range.start + count;
                inbox
                    .detail_list
                    .splice_focusable(range, handles(&rows[added]));
                shown.1 = rows;
            }
        }
        let state = &inbox.detail_list;
        let rows = list(
            state.clone(),
            cx.processor(|this, ix: usize, _, cx| this.render_inbox_detail_row(ix, cx)),
        )
        .size_full()
        .pr(crate::ui::scrollbar::gutter(state));
        let framed = crate::ui::scrollbar::framed("inbox-detail-scrollbar", state, rows);
        self.with_inbox_selection(
            div().size_full().relative().child(framed),
            &selected_key,
            cx,
        )
        .into_any_element()
    }

    /// The rows the selected item shows, as far as it has loaded.
    fn inbox_detail_rows(&self, item: &WorkItem, cx: &Context<Self>) -> Vec<DetailRow> {
        let inbox = self.inbox(cx);
        let key = item.key();
        let pr = item.kind == Kind::Pr;
        let blocks = match inbox.details.get(&key) {
            Some(Ok(d)) => inbox.description_blocks(&key, d).len().max(1),
            _ => 1,
        };
        let mut rows = vec![DetailRow::Head, DetailRow::DescriptionTop];
        rows.extend((0..blocks).map(DetailRow::Description));
        if pr {
            rows.push(DetailRow::Checks);
        }
        rows.push(DetailRow::ConversationHead);
        match inbox.threads.get(&key) {
            None => {}
            Some(Err(_)) => rows.push(DetailRow::CommentForm),
            Some(Ok(thread)) => {
                rows.extend((0..thread.comments.len()).map(DetailRow::Comment));
                if pr && !thread.commits.is_empty() {
                    rows.push(DetailRow::Commits);
                }
                rows.push(DetailRow::CommentForm);
            }
        }
        rows
    }

    fn render_inbox_detail_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let (row, last) = {
            let shown = self.inbox(cx).detail_rows.borrow();
            match shown.1.get(ix) {
                Some(row) => (*row, ix + 1 == shown.1.len()),
                None => return div().into_any_element(),
            }
        };
        let Some(item) = self
            .inbox(cx)
            .selected
            .as_deref()
            .and_then(|key| self.inbox(cx).item(key))
        else {
            return div().into_any_element();
        };
        let column = div()
            .w_full()
            .max_w(px(1024.0))
            .mx_auto()
            .px_8()
            .when(last, |el| el.pb_8());
        let row = match row {
            DetailRow::Head => column.pt_4().child(self.render_inbox_head(item, cx)),
            DetailRow::DescriptionTop => column.pt_5().child(self.render_description_top(item, cx)),
            DetailRow::Description(block) => {
                column.child(self.render_description_block(item, block, cx))
            }
            DetailRow::Checks => column.pt_3().child(self.render_pr_checks(item, cx)),
            DetailRow::ConversationHead => {
                column.pt_7().child(self.render_conversation_head(item, cx))
            }
            DetailRow::Comment(comment) => column
                .pt_3()
                .children(self.render_thread_comment(item, comment, cx)),
            DetailRow::Commits => column.pt_3().children(self.render_thread_commits(item, cx)),
            DetailRow::CommentForm => column.pt_3().child(self.render_comment_form(cx)),
        };
        // A list row is laid out as a root, where auto margins do nothing:
        // the column centres inside a full-width box.
        div().w_full().child(row).into_any_element()
    }

    /// Identity, title, people and times, actions, labels and, for a pull
    /// request, its tabs.
    fn render_inbox_head(&self, item: &WorkItem, cx: &Context<Self>) -> gpui::Div {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let now = crate::app::now_ms();
        let loaded = (self.inbox(cx).details.get(&key))
            .and_then(|d| d.as_ref().ok())
            .map(|d| &d.details);
        let author = loaded.map(|d| d.author.clone()).filter(|a| !a.is_empty());
        let (base, head) = loaded
            .map(|d| (d.base_ref.clone(), d.head_ref.clone()))
            .unwrap_or_default();
        let url = item.url.clone();
        // Backlog's clipboard button: the key and the title, ready to paste
        // into a commit or a chat.
        let ticket = format!("{} {}", item.identifier, item.title);

        // State, number and repository, with Open in GitHub at the far end.
        let identity = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(Text::Base.px())
            .text_color(fg.ink(Ink::Secondary))
            .child(state_pill(item, cx))
            .child(div().min_w_0().truncate().child(format!(
                "{} · {}",
                item.identifier,
                source_label(item)
            )))
            .child(
                IconButton::new("inbox-copy-ticket", IconName::Copy)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Copy key and title")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(ticket.clone()))
                    }),
            )
            .child(div().flex_1())
            .child(
                IconButton::new("inbox-open", IconName::ExternalLink)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip(format!("Open in {}", item.provider.label()))
                    .disabled(url.is_empty())
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );

        let dot = || byline_dot(fg);
        let mut meta = byline(fg);
        if let Some(author) = author.clone() {
            meta = meta.child(author_name(author, fg));
        }
        let created = relative_time(&item.created_at, now);
        if !created.is_empty() {
            meta = meta.child(format!("opened {created}")).child(dot());
        }
        meta = meta.child(format!("updated {}", relative_time(&item.updated_at, now)));
        // A Backlog issue's fields show below, where they are edited.
        let fields = loaded.and_then(|d| d.backlog.as_ref());
        if fields.is_none() && !item.assignees.is_empty() {
            meta = meta
                .child(dot())
                .child(format!("assigned to {}", item.assignees.join(", ")));
        }
        if fields.is_none() && !item.priority.is_empty() {
            meta = meta
                .child(dot())
                .child(format!("priority {}", item.priority.to_lowercase()));
        }
        if fields.is_none() && !item.due_date.is_empty() {
            meta = meta.child(dot()).child(format!("due {}", item.due_date));
        }
        let decision = loaded.map_or("", |d| d.review_decision.as_str());
        let review = match decision {
            "APPROVED" => "Approved",
            "CHANGES_REQUESTED" => "Changes requested",
            "REVIEW_REQUIRED" => "Review required",
            _ => "",
        };
        if !review.is_empty() {
            let color = review_tint(decision, colors).unwrap_or(fg.ink(Ink::Secondary));
            meta = meta
                .child(dot())
                .child(div().text_color(color).child(review));
        }

        let branches = (!base.is_empty() && !head.is_empty()).then(|| {
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(branch_chip(&head, cx))
                .child(
                    Icon::new(IconName::ArrowRight)
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Muted)),
                )
                .child(branch_chip(&base, cx))
        });

        // Milestones are among the fields.
        let labels: Vec<_> = (item.labels.iter())
            .filter(|l| fields.is_none_or(|f| !f.milestones.iter().any(|m| m.name == l.name)))
            .collect();
        let start_key = key.clone();
        let actions =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                // MonoCode starts threads from issues only.
                .when(item.kind == Kind::Issue, |el| {
                    el.child(
                        Button::new("inbox-start", "Send to agent")
                            .primary()
                            .icon(IconName::Sparkles)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_inbox_item(&start_key, cx)
                            })),
                    )
                    .child(self.render_inbox_project_picker(item, cx))
                })
                .when(item.provider == Provider::Backlog, |el| {
                    el.children(self.render_backlog_status_picker(item, cx))
                })
                .child(self.render_translate_menu(item, cx))
                .when(item.kind == Kind::Pr, |el| {
                    el.children(self.render_pr_actions(item, &base, &head, cx))
                });

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(identity)
                    .child(
                        div()
                            .text_size(Text::Title.px())
                            .line_height(px(28.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .line_clamp(3)
                            .child(match self.inbox(cx).translations.shown(&key) {
                                Some(t) => t.title.clone(),
                                None => SharedString::from(item.title.clone()),
                            }),
                    )
                    .child(meta)
                    .children(branches)
                    .when(!labels.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .children(labels.into_iter().map(|l| label_chip(l, cx))),
                        )
                    }),
            )
            .child(actions)
            .when(item.provider == Provider::Backlog, |el| {
                el.children(self.render_backlog_fields(item, cx))
            })
            .children(self.render_pr_action_status(cx))
            .children((self.inbox(cx).translations.error(&key)).map(|err| {
                div()
                    .text_size(Text::Base.px())
                    .text_color(colors.danger.opacity(0.9))
                    .child(SharedString::from(err.to_string()))
            }))
            .children(
                self.backlog
                    .issue_error
                    .clone()
                    .filter(|_| item.provider == Provider::Backlog)
                    .map(|err| {
                        div()
                            .text_size(Text::Base.px())
                            .text_color(colors.danger.opacity(0.9))
                            .child(SharedString::from(err))
                    }),
            )
            .when(item.kind == Kind::Pr, |el| {
                el.child(self.render_pr_tabs(item, cx))
            })
    }

    /// The description card's top: who wrote it. The description reads as
    /// the opening comment.
    fn render_description_top(&self, item: &WorkItem, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let author = (self.inbox(cx).details.get(&item.key()))
            .and_then(|d| d.as_ref().ok())
            .map(|d| d.details.author.clone())
            .filter(|a| !a.is_empty());
        card_fill(fg)
            .rounded_t(px(6.0))
            .border_t_1()
            .border_x_1()
            .child(
                byline(fg)
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .when_some(author, |el, author| el.child(author_name(author, fg)))
                    .child("Description"),
            )
    }

    /// One block of the description in its card; the last closes the card.
    fn render_description_block(
        &self,
        item: &WorkItem,
        block: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let inbox = self.inbox(cx);
        let details = inbox.details.get(&key);
        let shown = |d| inbox.description_blocks(&key, d);
        let blocks = match details {
            Some(Ok(d)) => shown(d).len().max(1),
            _ => 1,
        };
        // A translation's blocks are other markdown than the original's.
        let language = (inbox.translations.shown(&key)).map_or("", |t| t.language.label());
        let body = match details {
            None => div()
                .text_size(Text::Md.px())
                .text_color(fg.ink(Ink::Muted))
                .child("Loading…")
                .into_any_element(),
            Some(Err(err)) => div()
                .text_size(Text::Base.px())
                .text_color(colors.danger.opacity(0.9))
                .child(SharedString::from(err.clone()))
                .into_any_element(),
            Some(Ok(d)) if shown(d).is_empty() => div()
                .text_size(Text::Md.px())
                .italic()
                .text_color(fg.ink(Ink::Muted))
                .child("No description provided.")
                .into_any_element(),
            Some(Ok(d)) => match shown(d).get(block) {
                Some(source) => AgentMarkdown::new(
                    SharedString::from(format!("inbox-body-{key}-{language}-{block}")),
                    source.clone(),
                )
                .selectable(self.inbox_seg_ctx(&key, block, cx))
                .into_any_element(),
                None => div().into_any_element(),
            },
        };
        card_fill(fg)
            .border_x_1()
            .px_4()
            .pt_3()
            .text_size(Text::Lg.px())
            .when(block + 1 == blocks, |el| {
                el.pb_3().border_b_1().rounded_b(px(6.0))
            })
            .child(body)
    }

    /// MonoCode's `Summary` / `Code` tabs, with the file count once read.
    fn render_pr_tabs(
        &self,
        item: &crate::github::WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let tab = self.inbox(cx).pr_tab;
        let files = self
            .diff_docs
            .get(&pr_doc_key(item))
            .filter(|doc| doc.status == crate::ui::diff_viewer::DocStatus::Ready)
            .map(|doc| doc.files.len());
        let mut files_tab = Choice::new("files", "Files changed");
        if let Some(n) = files {
            files_tab = files_tab.note(n.to_string());
        }
        Tabs::new(
            "inbox-pr-tabs",
            [Choice::new("summary", "Summary"), files_tab],
            if tab == PrTab::Files {
                "files"
            } else {
                "summary"
            },
        )
        .on_change(on_value(cx, |this, tab, cx| {
            let tab = if tab == "files" {
                PrTab::Files
            } else {
                PrTab::Summary
            };
            this.set_inbox_pr_tab(tab, cx)
        }))
    }

    /// MonoCode `InboxProjectPicker`: which rail project the agent works in.
    fn render_inbox_project_picker(
        &self,
        item: &crate::github::WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let chosen = self.inbox_start_project_shown(item, cx);
        let key = item.key();
        // A Backlog project keeps the folder picked for one of its issues.
        let backlog_project = (item.provider == Provider::Backlog).then(|| item.repo.clone());
        let menu = self
            .inbox_projects_shown(cx)
            .into_iter()
            .fold(Menu::new(), |menu, path| {
                let (key, target) = (key.clone(), path.clone());
                let backlog_project = backlog_project.clone();
                menu.item(
                    MenuItem::check(
                        project_name(&path),
                        crate::app::same_project_path(&path, &chosen),
                    )
                    .on_click(app_callback(cx, move |this, cx| {
                        this.inbox_state
                            .as_mut(cx)
                            .start_project
                            .insert(key.clone(), target.clone());
                        if let Some(project) = &backlog_project {
                            this.set_backlog_project_folder(project, &target, cx);
                        }
                        cx.notify();
                    })),
                )
            });
        DropdownMenu::new("inbox-start-project", project_name(&chosen), menu)
            .variant(ButtonVariant::Secondary)
            .icon(IconName::Folder)
    }

    /// A Backlog issue's status, as a menu of its project's workflow.
    fn render_backlog_status_picker(
        &self,
        item: &crate::github::WorkItem,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let statuses = self.backlog.statuses.get(&item.container_id)?;
        let key = item.key();
        let busy = self.backlog.issue_busy.is_some();
        let menu = statuses.iter().fold(Menu::new(), |menu, status| {
            let (key, id) = (key.clone(), status.id);
            menu.item(
                MenuItem::check(status.name.clone(), status.name == item.state)
                    .disabled(busy)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.update_backlog_issue(&key, crate::backlog::Change::Status(id), cx)
                    })),
            )
        });
        let label = if self.backlog.issue_busy.as_deref() == Some(key.as_str()) {
            "Updating…".to_string()
        } else {
            item.state.clone()
        };
        Some(
            DropdownMenu::new("inbox-backlog-status", label, menu)
                .variant(ButtonVariant::Secondary)
                .icon(IconName::CircleDot),
        )
    }
}

/// The item's state as its tracker shows it: icon and word on a tinted
/// pill. Backlog's status names stand alone.
fn state_pill(item: &WorkItem, cx: &gpui::App) -> impl IntoElement {
    let (icon, tint, status) = status_mark(item, cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(tint.opacity(0.14))
        .text_color(tint)
        .font_weight(FontWeight::MEDIUM)
        .child(Icon::new(icon).size(IconSize::Xs).color(tint))
        .child(match item.provider {
            Provider::GitHub => format!("{status} {}", kind_label(item.kind).to_lowercase()),
            Provider::Backlog => status.to_string(),
        })
}

/// A branch name in code type, cut short when the row is narrow.
fn branch_chip(name: &str, cx: &gpui::App) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    div()
        .min_w_0()
        .max_w(px(360.0))
        .truncate()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(fg.fill(Fill::Line))
        .font_family(cx.theme().mono_family.clone())
        .text_size(Text::Sm.px())
        .text_color(fg.ink(Ink::Body))
        .child(SharedString::from(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::DetailRow::*;
    use super::*;

    #[test]
    fn rows_kept_at_both_ends_are_not_replaced() {
        let loading = [Head, DescriptionTop, Description(0), ConversationHead];
        let loaded = [
            Head,
            DescriptionTop,
            Description(0),
            Description(1),
            Description(2),
            ConversationHead,
            Comment(0),
            CommentForm,
        ];
        assert_eq!(changed(&loading, &loaded), (3..4, 5));
        assert_eq!(changed(&loaded, &loaded), (8..8, 0));
        assert_eq!(changed(&loaded[..3], &loaded[..5]), (3..3, 2));
    }
}
