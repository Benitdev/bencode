//! MonoCode `InboxDetail`: identity (its state, a Backlog issue's status
//! picker), title, author and times, the actions (Send to agent, or Open
//! thread once the item has one, its menu holding the project; pull
//! request actions; Open in its tracker), the body with its
//! Translate control, then a pull request's checks and the conversation.
//! The properties (`properties.rs`) and the item's threads (`threads.rs`)
//! sit in a column beside it when the detail is wide enough
//! (`two_columns`), under the actions otherwise. A
//! pull request's Files changed section shows its patch in the review list
//! (`ui/diff_viewer.rs`).
//!
//! The rest is a virtual list of rows (`DetailRow`): a scroll frame draws
//! the rows in view, the description a row per block (`blocks.rs`). Once
//! the head has scrolled away a compact bar keeps the item's key, title
//! and actions over the list.
//!
//! The same detail shows an item in a tab beside the chat (`tab.rs`): each
//! view (`DetailView`) keeps its own list (`DetailList`) and is built for
//! an item key, not for the Inbox's selection.

use std::cell::RefCell;
use std::ops::Range;

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::feedback::EmptyState;
use bencode_ui::forms::Choice;
use bencode_ui::menus::{DropdownMenu, Menu, MenuItem, SplitButton};
use bencode_ui::navigation::Tabs;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, IntoElement, ParentElement, SharedString,
    Styled, div, list, prelude::*,
};

use crate::ui::scale::px;

use super::comments::{author_name, byline, byline_dot, card_fill, review_tint};
use super::properties::PROPERTIES_WIDTH;
use super::{
    InboxState, PrTab, StartWith, kind_label, project_name, relative_time, source_label,
    status_mark,
};
use crate::app::BenCodeApp;
use crate::app::item_threads::{PrimaryAction, primary_action};
use crate::github::{Kind, Thread, WorkItem};
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::composer::inbox_card::CardTask;
use crate::ui::composer::inbox_card::label_chip;
use crate::ui::diff_viewer::pr_doc_key;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::work_items::Provider;
use crate::work_items::Review;

/// The main column's widest: the description's measure.
const MAIN_MAX: f32 = 1024.0;
/// The main column's narrowest beside the properties; below it they
/// stack under the actions.
const MAIN_MIN: f32 = 560.0;
/// The compact bar's height over the list.
const COMPACT_HEAD_HEIGHT: f32 = 44.0;

/// The box a row's content (and the compact bar) keeps to across the
/// detail: the main column, centred; wide, the main column and the
/// properties beside it centred as one, the properties' room left empty.
fn group(wide: bool) -> gpui::Div {
    let room = if wide { PROPERTIES_WIDTH } else { 0.0 };
    div()
        .w_full()
        .max_w(px(MAIN_MAX + room))
        .mx_auto()
        .pr(px(room))
}

/// Whether a detail `width` wide (at 100%) takes the properties as a
/// column beside the main one.
pub(super) fn two_columns(width: f32) -> bool {
    width >= MAIN_MIN + PROPERTIES_WIDTH
}

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

/// Where an item's detail is drawn: the Inbox's, for its selected item, or
/// a tab beside the chat (`tab.rs`). The tab has no comment box (the
/// Inbox's one input serves its selected item; the tab offers Open in
/// Inbox), no Files changed, and keeps to one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailView {
    Inbox,
    Tab,
}

impl DetailView {
    fn scrollbar_id(self) -> &'static str {
        match self {
            Self::Inbox => "inbox-detail-scrollbar",
            Self::Tab => "work-item-detail-scrollbar",
        }
    }

    /// The region the view is drawn in.
    pub(super) fn region(self) -> crate::ui::region::Region {
        match self {
            Self::Inbox => crate::ui::region::Region::Surface,
            Self::Tab => crate::ui::region::Region::FilePane,
        }
    }
}

/// One view's virtual list of an item's rows.
pub(crate) struct DetailList {
    pub(super) list: gpui::ListState,
    /// The item and rows `list` was last told of.
    rows: RefCell<(String, Vec<DetailRow>)>,
}

impl Default for DetailList {
    fn default() -> Self {
        Self {
            list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.0)),
            rows: RefCell::default(),
        }
    }
}

impl DetailList {
    /// Tells the list of item `key`'s `rows`: another item starts over, the
    /// same item keeps the rows unchanged at either end, measured.
    /// `handles` gives a row the focus that keeps it drawn.
    fn sync(
        &self,
        key: String,
        rows: Vec<DetailRow>,
        handles: impl Fn(&[DetailRow]) -> Vec<Option<gpui::FocusHandle>>,
    ) {
        let mut shown = self.rows.borrow_mut();
        if shown.0 != key {
            self.list.reset(0);
            self.list.splice_focusable(0..0, handles(&rows));
            *shown = (key, rows);
        } else if shown.1 != rows {
            let (range, count) = changed(&shown.1, &rows);
            let added = range.start..range.start + count;
            self.list.splice_focusable(range, handles(&rows[added]));
            shown.1 = rows;
        }
    }

    /// Scrolls item `key`'s `row` into view, when the list shows it.
    pub(super) fn reveal(&self, key: &str, row: DetailRow) {
        let at = {
            let shown = self.rows.borrow();
            (shown.0 == key)
                .then(|| shown.1.iter().position(|r| *r == row))
                .flatten()
        };
        if let Some(at) = at {
            self.list.scroll_to_reveal_item(at);
        }
    }
}

impl InboxState {
    pub(super) fn detail_list(&self, view: DetailView) -> &DetailList {
        match view {
            DetailView::Inbox => &self.detail,
            DetailView::Tab => &self.tab_detail,
        }
    }
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
    /// The selected item, in a detail `width` wide (at 100%, from the
    /// window's layout: `render_inbox_body`).
    pub(super) fn render_inbox_detail(&self, width: f32, cx: &Context<Self>) -> AnyElement {
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
        self.render_item_detail(item, DetailView::Inbox, width, cx)
    }

    /// Item `item` in `view`, `width` wide (at 100%).
    pub(super) fn render_item_detail(
        &self,
        item: &WorkItem,
        view: DetailView,
        width: f32,
        cx: &Context<Self>,
    ) -> AnyElement {
        // The review scrolls on its own (a virtual list), under a header
        // that stays; the properties make way for the code.
        let colors = &cx.theme().colors;
        let files = view == DetailView::Inbox && self.inbox(cx).pr_tab == PrTab::Files;
        if item.kind == Kind::Pr && files {
            let column = || div().w_full().max_w(px(MAIN_MAX)).mx_auto().px_8();
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    column()
                        .flex_none()
                        .pt_4()
                        .child(self.render_inbox_head(item, false, view, cx)),
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

        let wide = two_columns(width);
        let rows = self.inbox_detail_rows(item, view, cx);
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
        let detail = inbox.detail_list(view);
        detail.sync(key, rows, handles);
        // A switch between one column and two changes the list's width,
        // and the list measures its rows again (the head with them).
        let state = &detail.list;
        let rows = list(
            state.clone(),
            cx.processor(move |this, ix: usize, _, cx| {
                this.render_inbox_detail_row(ix, wide, view, cx)
            }),
        )
        .size_full()
        .pr(crate::ui::scrollbar::gutter(state));
        let framed = crate::ui::scrollbar::framed(view.scrollbar_id(), state, rows);
        // The list redraws the region it is in as it scrolls, so this is
        // read fresh each time; the bar adds no redraw of its own.
        let head_hidden = state.logical_scroll_top().item_ix > 0;
        // The list spans the whole detail, so the wheel scrolls it from
        // the margins too; its rows and the bar keep to the main column,
        // and the properties sit over the list's right side, scrolling on
        // their own.
        let properties = wide.then(|| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right(crate::ui::scrollbar::gutter(state))
                .child(
                    div()
                        .w_full()
                        .max_w(px(MAIN_MAX + PROPERTIES_WIDTH))
                        .mx_auto()
                        .h_full()
                        .flex()
                        .justify_end()
                        .child(
                            div()
                                .id("inbox-properties")
                                .flex_none()
                                .w(px(PROPERTIES_WIDTH))
                                .h_full()
                                .overflow_y_scroll()
                                .border_l_1()
                                .border_color(colors.border)
                                .px_3()
                                .pt_4()
                                .pb_6()
                                .children(self.render_inbox_properties(item, view, cx))
                                .children(
                                    self.render_item_prs(item, view, cx)
                                        .map(|prs| div().mt_5().child(prs)),
                                )
                                .children(
                                    self.render_inbox_threads(&selected_key, cx)
                                        .map(|threads| div().mt_5().child(threads)),
                                ),
                        ),
                )
        });
        div()
            .size_full()
            .relative()
            .child(self.with_inbox_selection(div().size_full().child(framed), &selected_key, cx))
            .children(properties)
            .when(head_hidden, |el| {
                el.child(
                    self.render_inbox_compact_head(item, wide, cx)
                        .right(crate::ui::scrollbar::gutter(state)),
                )
            })
            .into_any_element()
    }

    /// The rows `item` shows in `view`, as far as it has loaded.
    fn inbox_detail_rows(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Vec<DetailRow> {
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
        let form = view == DetailView::Inbox;
        match inbox.threads.get(&key) {
            None => {}
            Some(Err(_)) => rows.extend(form.then_some(DetailRow::CommentForm)),
            Some(Ok(thread)) => {
                rows.extend((0..thread.comments.len()).map(DetailRow::Comment));
                if pr && !thread.commits.is_empty() {
                    rows.push(DetailRow::Commits);
                }
                rows.extend(form.then_some(DetailRow::CommentForm));
            }
        }
        rows
    }

    fn render_inbox_detail_row(
        &self,
        ix: usize,
        wide: bool,
        view: DetailView,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (row, last, item) = {
            let inbox = self.inbox(cx);
            let shown = inbox.detail_list(view).rows.borrow();
            match (shown.1.get(ix), inbox.item(&shown.0)) {
                (Some(row), Some(item)) => (*row, ix + 1 == shown.1.len(), item),
                _ => return div().into_any_element(),
            }
        };
        let column = div().w_full().px_8().when(last, |el| el.pb_8());
        let row = match row {
            DetailRow::Head => column
                .pt_4()
                .child(self.render_inbox_head(item, wide, view, cx)),
            DetailRow::DescriptionTop => column.pt_5().child(self.render_description_top(item, cx)),
            DetailRow::Description(block) => {
                column.child(self.render_description_block(item, block, view, cx))
            }
            DetailRow::Checks => column.pt_3().child(self.render_pr_checks(item, cx)),
            DetailRow::ConversationHead => {
                column.pt_7().child(self.render_conversation_head(item, cx))
            }
            DetailRow::Comment(comment) => column
                .pt_3()
                .children(self.render_thread_comment(item, comment, view, cx)),
            DetailRow::Commits => column.pt_3().children(self.render_thread_commits(item, cx)),
            DetailRow::CommentForm => column.pt_3().child(self.render_comment_form(cx)),
        };
        // A list row is laid out as a root, where auto margins do nothing:
        // the column centres inside a full-width box.
        div()
            .w_full()
            .child(group(wide).child(row))
            .into_any_element()
    }

    /// Identity, title, author and times, actions and, for a pull request,
    /// its tabs. Narrow (`wide` false), the properties follow the actions:
    /// a Backlog issue's as rows, a GitHub item's in the byline, with the
    /// branches and labels.
    fn render_inbox_head(
        &self,
        item: &WorkItem,
        wide: bool,
        view: DetailView,
        cx: &Context<Self>,
    ) -> gpui::Div {
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
        // Backlog's clipboard button (also `y` in the list).
        let ticket = super::keyboard::ticket_line(item);
        let github = item.provider == Provider::GitHub;

        // The Inbox's item opens beside the chat; the tab's in the Inbox,
        // where its comment box is.
        let elsewhere = {
            let key = key.clone();
            match view {
                DetailView::Inbox => IconButton::new("inbox-open-beside", IconName::Columns2)
                    .tooltip("Open beside the chat")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_work_item_beside(&key, cx)),
                    ),
                DetailView::Tab => IconButton::new("work-item-open-inbox", IconName::Inbox)
                    .tooltip("Open in the Inbox, to comment")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_inbox_item(&key, cx))),
            }
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
        };
        // State, number and repository, with Open in GitHub at the far end.
        let identity = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(Text::Base.px())
            .text_color(fg.ink(Ink::Secondary))
            .child(self.render_state_control(item, cx))
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
            .child(elsewhere)
            .child(open_in_tracker("inbox-open", item));

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
        // Wide, these are properties; a Backlog issue's always are.
        let in_byline = github && !wide;
        if in_byline && !item.assignees.is_empty() {
            meta = meta
                .child(dot())
                .child(format!("assigned to {}", item.assignees.join(", ")));
        }
        if in_byline && !item.priority.is_empty() {
            meta = meta
                .child(dot())
                .child(format!("priority {}", item.priority.to_lowercase()));
        }
        if in_byline && !item.due_date.is_empty() {
            meta = meta.child(dot()).child(format!("due {}", item.due_date));
        }
        let decision = loaded.map_or("", |d| d.review_decision.as_str());
        let review = match decision {
            "APPROVED" => "Approved",
            "CHANGES_REQUESTED" => "Changes requested",
            "REVIEW_REQUIRED" => "Review required",
            _ => "",
        };
        if in_byline && !review.is_empty() {
            let color = review_tint(decision, colors).unwrap_or(fg.ink(Ink::Secondary));
            meta = meta
                .child(dot())
                .child(div().text_color(color).child(review));
        }

        let branches = (in_byline && !base.is_empty() && !head.is_empty()).then(|| {
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
        let labels: Vec<_> = if in_byline {
            item.labels.iter().collect()
        } else {
            Vec::new()
        };
        // Narrow, a Backlog issue's properties wrap under the actions; a
        // pull request's ticket goes with them.
        let properties = (!wide)
            .then(|| match github {
                true => self.ticket_property(item, view, cx).into_iter().collect(),
                false => self.render_inbox_properties(item, view, cx),
            })
            .filter(|rows| !rows.is_empty())
            .map(|rows| {
                div().flex().flex_wrap().gap_x_6().children(
                    rows.into_iter()
                        .map(|row| div().w(px(PROPERTIES_WIDTH)).child(row)),
                )
            });

        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .children(self.render_send_to_agent(item, "inbox-start", ControlSize::Md, cx))
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
                            .child(self.inbox_title_shown(item, cx)),
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
            .children(properties)
            .children(
                (!wide)
                    .then(|| self.render_item_prs(item, view, cx))
                    .flatten()
                    .map(|prs| div().max_w(px(PROPERTIES_WIDTH * 2.0)).child(prs)),
            )
            .children(
                (!wide)
                    .then(|| self.render_inbox_threads(&key, cx))
                    .flatten()
                    .map(|threads| div().max_w(px(PROPERTIES_WIDTH * 2.0)).child(threads)),
            )
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
            .when(item.kind == Kind::Pr && view == DetailView::Inbox, |el| {
                el.child(self.render_pr_tabs(item, cx))
            })
    }

    /// The title as shown: its translation while that shows.
    pub(super) fn inbox_title_shown(&self, item: &WorkItem, cx: &Context<Self>) -> SharedString {
        match self.inbox(cx).translations.shown(&item.key()) {
            Some(t) => t.title.clone(),
            None => SharedString::from(item.title.clone()),
        }
    }

    /// "Send to agent" for an issue, "Review with agent" for a pull
    /// request, or "Open thread" once the item has one (its newest): the
    /// one place it is built, for the actions and the compact bar alike
    /// (`id` tells them apart). Its menu starts another thread (a pull
    /// request's also to address its review, in a new worktree, once
    /// reviewers asked for something), in a new worktree or in Plan mode,
    /// and picks the project threads start in (MonoCode
    /// `InboxProjectPicker`).
    fn render_send_to_agent(
        &self,
        item: &WorkItem,
        id: &'static str,
        size: ControlSize,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let key = item.key();
        let main = StartWith::main(item.kind);
        let pr = item.kind == Kind::Pr;
        let (label, icon) = match (primary_action(&self.item_threads(&key)), pr) {
            (PrimaryAction::Open(_), _) => ("Open thread", IconName::MessageSquare),
            (PrimaryAction::Send, false) => ("Send to agent", IconName::Sparkles),
            (PrimaryAction::Send, true) => ("Review with agent", IconName::Sparkles),
        };
        let start = |label: &'static str, icon: IconName, with: StartWith| {
            let key = key.clone();
            MenuItem::new(label)
                .icon(icon)
                .on_click(app_callback(cx, move |this, cx| {
                    this.start_inbox_item(&key, with, cx)
                }))
        };
        let feedback = pr && {
            let inbox = self.inbox(cx);
            let thread = inbox.threads.get(&key).and_then(|t| t.as_ref().ok());
            has_review_feedback(item.pr_status.review, thread)
        };
        let mut menu = Menu::new().item(start(
            if pr {
                "New review thread"
            } else {
                "New thread"
            },
            IconName::Plus,
            main,
        ));
        if feedback {
            menu = menu.item(start(
                "Address review comments",
                IconName::Wrench,
                StartWith {
                    worktree: true,
                    task: CardTask::AddressReview,
                    ..main
                },
            ));
        }
        let menu = menu
            .item(start(
                "In a new worktree",
                IconName::GitBranch,
                StartWith {
                    worktree: true,
                    ..main
                },
            ))
            .item(start(
                "Plan first",
                IconName::Lightbulb,
                StartWith { plan: true, ..main },
            ))
            .separator();
        let menu = self.inbox_project_choices(item, menu, cx);
        Some(
            SplitButton::new(id, label, menu)
                .variant(ButtonVariant::Primary)
                .size(size)
                .icon(icon)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_or_start_inbox_item(&key, cx)),
                )
                .into_any_element(),
        )
    }

    /// The rail's projects, the one threads start in checked; a Backlog
    /// project keeps the folder picked for one of its issues.
    fn inbox_project_choices(&self, item: &WorkItem, menu: Menu, cx: &Context<Self>) -> Menu {
        let chosen = self.inbox_start_project_shown(item, cx);
        let key = item.key();
        let backlog_project = (item.provider == Provider::Backlog).then(|| item.repo.clone());
        self.inbox_projects_shown(cx)
            .into_iter()
            .fold(menu, |menu, path| {
                let (key, target) = (key.clone(), path.clone());
                let backlog_project = backlog_project.clone();
                menu.item(
                    MenuItem::check(
                        format!("In {}", project_name(&path)),
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
            })
    }

    /// The bar over the list once the head has scrolled away: the state's
    /// mark, the key, the title on one line, Send to agent and Open in its
    /// tracker.
    fn render_inbox_compact_head(
        &self,
        item: &WorkItem,
        wide: bool,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (icon, tint, status) = status_mark(item, cx);
        div().absolute().top_0().left_0().child(
            group(wide).child(
                div()
                    .w_full()
                    .h(px(COMPACT_HEAD_HEIGHT))
                    .bg(colors.bg)
                    .border_b_1()
                    .border_color(colors.border)
                    .px_8()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("inbox-compact-status")
                            .flex_none()
                            .flex()
                            .items_center()
                            .tooltip(Tooltip::text(status))
                            .child(Icon::new(icon).size(IconSize::Sm).color(tint)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(Text::Base.px())
                            .text_color(fg.ink(Ink::Secondary))
                            .child(SharedString::from(item.identifier.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(Text::Md.px())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(self.inbox_title_shown(item, cx)),
                    )
                    .children(self.render_send_to_agent(
                        item,
                        "inbox-compact-start",
                        ControlSize::Sm,
                        cx,
                    ))
                    .child(open_in_tracker("inbox-compact-open", item)),
            ),
        )
    }

    /// The description card's top: "Description", and the Translate
    /// control on the right, beside the text it changes. The byline names
    /// the author.
    fn render_description_top(&self, item: &WorkItem, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        card_fill(fg)
            .rounded_t(px(6.0))
            .border_t_1()
            .border_x_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pl_3()
                    .pr_1p5()
                    .py_1()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_size(Text::Base.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg.ink(Ink::Secondary))
                            .child("Description"),
                    )
                    .child(self.render_translate_control(item, cx)),
            )
    }

    /// One block of the description in its card; the last closes the card.
    fn render_description_block(
        &self,
        item: &WorkItem,
        block: usize,
        view: DetailView,
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
                .selectable(self.inbox_seg_ctx(&key, block, view, cx))
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

    /// The item's state at the head. A Backlog issue's pill opens its
    /// project's workflow (its one status control); a GitHub item's only
    /// shows.
    fn render_state_control(&self, item: &WorkItem, cx: &Context<Self>) -> AnyElement {
        let statuses = (item.provider == Provider::Backlog)
            .then(|| self.backlog.statuses.get(&item.container_id))
            .flatten();
        let Some(statuses) = statuses else {
            return state_pill(item, None, cx).into_any_element();
        };
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
        let updating = self.backlog.issue_busy.as_deref() == Some(key.as_str());
        let tint = status_mark(item, cx).1;
        let pill = state_pill(item, updating.then_some("Updating…"), cx)
            .id("inbox-status-pill")
            .hover(move |s| s.bg(tint.opacity(0.24)))
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(IconSize::Xs)
                    .color(tint),
            );
        DropdownMenu::new("inbox-backlog-status", item.state.clone(), menu)
            .trigger(pill)
            .into_any_element()
    }
}

/// Reviewers asked for something: changes requested, or a review comment
/// still open in the conversation as loaded.
fn has_review_feedback(review: Review, thread: Option<&Thread>) -> bool {
    review == Review::ChangesRequested
        || thread.is_some_and(|thread| {
            (thread.comments.iter()).any(|c| c.kind == "review_comment" && !c.resolved)
        })
}

/// Open in the item's tracker.
fn open_in_tracker(id: &'static str, item: &WorkItem) -> IconButton {
    let url = item.url.clone();
    IconButton::new(id, IconName::ExternalLink)
        .variant(ButtonVariant::Ghost)
        .size(ControlSize::Sm)
        .tooltip(format!("Open in {}", item.provider.label()))
        .disabled(url.is_empty())
        .on_click(move |_, _, cx| cx.open_url(&url))
}

/// The item's state as its tracker shows it: icon and word on a tinted
/// pill, or `label` in the word's place. Backlog's status names stand
/// alone.
fn state_pill(item: &WorkItem, label: Option<&str>, cx: &gpui::App) -> gpui::Div {
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
        .child(match (label, item.provider) {
            (Some(label), _) => label.to_string(),
            (None, Provider::GitHub) => {
                format!("{status} {}", kind_label(item.kind).to_lowercase())
            }
            (None, Provider::Backlog) => status.to_string(),
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
    fn properties_take_a_column_once_the_main_one_keeps_its_least() {
        assert_eq!(MAIN_MIN + PROPERTIES_WIDTH, 856.0);
        assert!(two_columns(856.0));
        assert!(!two_columns(855.0));
        // 1920 points at 90%, beside the navigator and the list.
        assert!(two_columns(1920.0 / 0.9 - 240.0 - 340.0));
        // 1280 points at 90%: stacked.
        assert!(!two_columns(1280.0 / 0.9 - 240.0 - 340.0));
    }

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

    #[test]
    fn a_review_is_addressed_once_reviewers_asked_for_something() {
        let comment = |kind: &str, resolved| crate::github::Comment {
            kind: kind.into(),
            resolved,
            ..Default::default()
        };
        let thread = |comments| Thread {
            comments,
            ..Default::default()
        };
        assert!(has_review_feedback(Review::ChangesRequested, None));
        assert!(!has_review_feedback(Review::Approved, None));
        let open = thread(vec![
            comment("comment", false),
            comment("review_comment", false),
        ]);
        assert!(has_review_feedback(Review::Required, Some(&open)));
        let settled = thread(vec![
            comment("review_comment", true),
            comment("review", false),
        ]);
        assert!(!has_review_feedback(Review::None, Some(&settled)));
    }
}
