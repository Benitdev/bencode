//! The selected item's properties as name / value rows (Linear's issue
//! sidebar): a Backlog issue's fields (`backlog_fields.rs`), edited in
//! place, then its type and categories; a GitHub item's assignees,
//! labels, review, branches and the tickets it is named after. A column
//! beside the description when the detail is wide enough
//! (`detail::two_columns`), wrapped under the actions otherwise. Empty
//! values read as a quiet "—". A ticket's pull requests follow as a
//! section of their own (`render_item_prs`).

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};

use super::comments::review_tint;
use super::{DetailView, InboxState, short_key, status_mark, status_name};
use crate::app::BenCodeApp;
use crate::backlog::FieldChoice;
use crate::github::{Kind, WorkItem};
use crate::ui::composer::inbox_card::label_chip;
use crate::ui::rows::ListRow;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::work_items::tickets::{ticket_item_key, title_without_key};
use crate::work_items::{PrStatus, Provider, Review};

/// The properties column's width beside the main one.
pub(super) const PROPERTIES_WIDTH: f32 = 296.0;
/// The names' column: "Resolution" and "Milestone" fit.
const NAME_WIDTH: f32 = 72.0;
/// A row's least height: a control's, so plain values line up with them.
const ROW_HEIGHT: f32 = 28.0;

/// A property: its name, then its value.
pub(super) fn property(name: &'static str, value: impl IntoElement, cx: &gpui::App) -> AnyElement {
    property_with(name, None, value, cx)
}

/// A property with a shortcut at the end of its name (Backlog's "Assign
/// to myself"), so the value keeps its whole width.
pub(super) fn property_with(
    name: &'static str,
    shortcut: Option<AnyElement>,
    value: impl IntoElement,
    cx: &gpui::App,
) -> AnyElement {
    let fg = cx.theme().colors.fg;
    div()
        .flex()
        .items_start()
        .gap_1()
        .child(
            div()
                .flex_none()
                .w(px(NAME_WIDTH))
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .justify_between()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .child(name)
                .children(shortcut),
        )
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

/// A value read, not edited: inset like a quiet control's text.
pub(super) fn plain(cx: &gpui::App) -> gpui::Div {
    let theme = cx.theme();
    div()
        .min_h(px(ROW_HEIGHT))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .px(theme.control_padding(ControlSize::Md))
        .text_size(Text::Base.px())
        .text_color(theme.colors.fg.ink(Ink::Body))
}

/// A value that is not set.
pub(super) fn none(cx: &gpui::App) -> gpui::Div {
    plain(cx)
        .text_color(cx.theme().colors.fg.ink(Ink::Faint))
        .child("—")
}

impl BenCodeApp {
    /// The rows the item's properties take, in order; empty while
    /// nothing of them is known.
    pub(super) fn render_inbox_properties(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        match item.provider {
            Provider::Backlog => {
                let milestones = (self.inbox(cx).details.get(&item.key()))
                    .and_then(|d| d.as_ref().ok())
                    .and_then(|d| d.details.backlog.as_ref())
                    .map(|f| f.milestones.as_slice());
                let mut rows = (self.render_backlog_fields(item, view, cx)).unwrap_or_default();
                rows.extend(backlog_kinds(item, milestones, cx));
                rows
            }
            Provider::GitHub => self.github_properties(item, view, cx),
        }
    }

    /// Assignees, labels and, for a pull request, its review and branches;
    /// a priority or due date where the item has one.
    fn github_properties(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = &cx.theme().colors;
        let loaded = (self.inbox(cx).details.get(&item.key()))
            .and_then(|d| d.as_ref().ok())
            .map(|d| &d.details);
        let mut rows = vec![property(
            "Assignees",
            if item.assignees.is_empty() {
                none(cx)
            } else {
                plain(cx).child(item.assignees.join(", "))
            },
            cx,
        )];
        rows.push(property(
            "Labels",
            if item.labels.is_empty() {
                none(cx)
            } else {
                plain(cx).children(item.labels.iter().map(|l| label_chip(l, cx)))
            },
            cx,
        ));
        if !item.priority.is_empty() {
            rows.push(property(
                "Priority",
                plain(cx).child(item.priority.clone()),
                cx,
            ));
        }
        if !item.due_date.is_empty() {
            rows.push(property(
                "Due",
                self.due_value(item, &item.due_date, cx),
                cx,
            ));
        }
        if item.kind == Kind::Pr {
            let decision = loaded.map_or("", |d| d.review_decision.as_str());
            let review = match decision {
                "APPROVED" => "Approved",
                "CHANGES_REQUESTED" => "Changes requested",
                "REVIEW_REQUIRED" => "Review required",
                _ => "",
            };
            rows.push(property(
                "Review",
                if review.is_empty() {
                    none(cx)
                } else {
                    let tint = review_tint(decision, colors).unwrap_or(colors.fg.ink(Ink::Body));
                    plain(cx).text_color(tint).child(review)
                },
                cx,
            ));
            let (base, head) = loaded.map_or(("", ""), |d| (&d.base_ref, &d.head_ref));
            if !base.is_empty() && !head.is_empty() {
                rows.push(property("Branches", branches(head, base, cx), cx));
            }
            rows.extend(self.ticket_property(item, view, cx));
        }
        rows
    }

    /// The tickets a pull request is named after (`work_items/tickets.rs`):
    /// each one's status, short key and title, opened by a click in the
    /// same view. One the Inbox has not got shows its key, and is fetched
    /// on the click.
    pub(super) fn ticket_property(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let inbox = self.inbox(cx);
        let tickets = inbox.tickets.tickets(&item.key());
        if item.kind != Kind::Pr || tickets.is_empty() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let hover = fg.fill(Fill::Hover);
        let pad = cx.theme().control_padding(ControlSize::Md);
        let rows = tickets.iter().map(|identifier| {
            let key = ticket_item_key(identifier);
            let short = short_key(identifier, &inbox.key_prefix).to_string();
            let ticket = inbox.item(&key);
            let (mark, status, title, tip) = match ticket {
                Some(ticket) => {
                    let (icon, tint, status) = status_mark(ticket, cx);
                    let title = title_shown(inbox, ticket);
                    let tip = format!("{identifier} {}\n{status}", ticket.title);
                    (
                        Icon::new(icon).color(tint),
                        Some((status, tint)),
                        title,
                        tip,
                    )
                }
                None => (
                    Icon::new(IconName::CircleDot).color(fg.ink(Ink::Faint)),
                    None,
                    SharedString::from("Not in the Inbox"),
                    format!("{identifier}\nNot in the Inbox's list: a click fetches it"),
                ),
            };
            let open = key.clone();
            div()
                .id(SharedString::from(format!("inbox-ticket-{key}")))
                .min_w_0()
                .px(pad)
                .py_1()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(Tooltip::text(tip))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_related_item(&open, view, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_size(Text::Base.px())
                        .child(mark.size(IconSize::Xs))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg.ink(Ink::Body))
                                .child(short),
                        )
                        .children(status.map(|(status, tint)| {
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(Text::Sm.px())
                                .text_color(tint)
                                .child(status)
                        })),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(if ticket.is_some() {
                            Ink::Secondary
                        } else {
                            Ink::Faint
                        }))
                        .child(title),
                )
        });
        Some(property("Ticket", div().children(rows), cx))
    }

    /// A ticket's Pull requests section: the listed pull requests named
    /// after it, a row each (number and title over state and repository),
    /// opened by a click in the same view; `None` while it has none.
    pub(super) fn render_item_prs(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let inbox = self.inbox(cx);
        let prs = inbox.ticket_prs(&item.key());
        if prs.is_empty() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let heading = div()
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .child(format!("Pull requests · {}", prs.len()));
        let rows = prs.into_iter().map(|pr| {
            let (icon, tint, status) = status_mark(pr, cx);
            let key = pr.key();
            let open = key.clone();
            // The ticket is the one shown: its key is not repeated.
            let title = title_shown(inbox, pr);
            let title = title_without_key(&title, &item.identifier);
            ListRow::new(
                format!("inbox-pr-link-{key}"),
                format!("#{} {title}", pr.number),
            )
            .tile(Icon::new(icon).size(IconSize::Xs).color(tint))
            .line(pr_line(&status, &pr.pr_status, &pr.repo))
            .tooltip(format!(
                "#{} {}\n{}",
                pr.number,
                pr.title,
                pr_line(&status, &pr.pr_status, &pr.repo)
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.open_related_item(&open, view, cx)))
        });
        Some(
            div()
                .child(heading)
                .child(div().mt_1().children(rows))
                .into_any_element(),
        )
    }

    /// A due date with how far it is (`due_note`); closed items' dates
    /// are history.
    fn due_value(&self, item: &WorkItem, due: &str, cx: &gpui::App) -> gpui::Div {
        let today = jiff::Zoned::now().date();
        let shown = jiff::civil::Date::strptime("%Y-%m-%d", due).map_or_else(
            |_| due.to_string(),
            |d| d.strftime("%b %-d, %Y").to_string(),
        );
        plain(cx)
            .child(shown)
            .children((!item.closed).then(|| due_note(due, today, cx)).flatten())
    }
}

/// An item's title as the Inbox shows it: its translation while that
/// shows, else a kept translation of the list's, else the original.
pub(super) fn title_shown(inbox: &InboxState, item: &WorkItem) -> SharedString {
    let key = item.key();
    (inbox.translations.shown(&key).map(|t| t.title.clone()))
        .or_else(|| inbox.translations.list_title(&key).cloned())
        .unwrap_or_else(|| SharedString::from(item.title.clone()))
}

/// A pull request's line in a ticket's section: its state, checks and
/// review as listed, and its repository.
pub(super) fn pr_line(status: &str, pr: &PrStatus, repo: &str) -> String {
    let checks = pr.checks_summary();
    let mut parts = vec![status.to_string()];
    if !checks.is_empty() {
        parts.push(format!("checks {checks}"));
    }
    if pr.review != Review::None {
        parts.push(pr.review.label().to_string());
    }
    parts.push(repo.to_string());
    parts.join(" · ")
}

/// The row mark of a ticket's pull requests: the first's state and number,
/// and how many more.
pub(super) fn prs_mark(prs: &[&WorkItem]) -> Option<String> {
    let first = prs.first()?;
    Some(match prs.len() {
        1 => format!("#{}", first.number),
        n => format!("#{} +{}", first.number, n - 1),
    })
}

/// The row mark's tooltip: the pull requests by number, state and title.
pub(super) fn prs_tip(prs: &[&WorkItem]) -> String {
    prs.iter()
        .map(|pr| format!("#{} · {}: {}", pr.number, status_name(pr), pr.title))
        .collect::<Vec<_>>()
        .join("\n")
}

/// "· in 5d" after a due date, toned as `row::Due` says.
pub(super) fn due_note(due: &str, today: jiff::civil::Date, cx: &gpui::App) -> Option<gpui::Div> {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    let distance = super::row::due_distance(due, today)?;
    let tone = match super::row::Due::of(due, today) {
        Some(super::row::Due::Overdue(_)) => colors.danger,
        Some(super::row::Due::Soon(_)) => crate::ui::tokens::Status::Attention.color(fg),
        None => fg.ink(Ink::Muted),
    };
    Some(
        div()
            .flex_none()
            .text_size(Text::Base.px())
            .text_color(tone)
            .child(SharedString::from(format!("· {distance}"))),
    )
}

/// A Backlog issue's type and categories, read only (Backlog's issue
/// page edits them). `labels` lists the type, the categories and the
/// milestones: the categories are known once the issue's milestones are.
fn backlog_kinds(
    item: &WorkItem,
    milestones: Option<&[FieldChoice]>,
    cx: &gpui::App,
) -> Vec<AnyElement> {
    let kind = item.issue_type.as_ref();
    let mut rows = vec![property(
        "Type",
        match kind {
            Some(kind) => plain(cx).child(label_chip(kind, cx)),
            None => none(cx),
        },
        cx,
    )];
    if let Some(milestones) = milestones {
        let categories: Vec<_> = (item.labels.iter())
            .skip(usize::from(kind.is_some()))
            .filter(|l| !milestones.iter().any(|m| m.name == l.name))
            .collect();
        rows.push(property(
            "Category",
            if categories.is_empty() {
                none(cx)
            } else {
                plain(cx).children(categories.into_iter().map(|l| label_chip(l, cx)))
            },
            cx,
        ));
    }
    rows
}

/// A pull request's head into its base, in code type.
fn branches(head: &str, base: &str, cx: &gpui::App) -> gpui::Div {
    let fg = cx.theme().colors.fg;
    let chip = |name: &str| {
        div()
            .min_w_0()
            .max_w_full()
            .truncate()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.0))
            .bg(fg.fill(Fill::Line))
            .font_family(cx.theme().mono_family.clone())
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Body))
            .child(SharedString::from(name.to_string()))
    };
    plain(cx)
        .py_1()
        .child(chip(head))
        .child(
            Icon::new(IconName::ArrowRight)
                .size(IconSize::Xs)
                .color(fg.ink(Ink::Muted)),
        )
        .child(chip(base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Kind;

    #[test]
    fn a_tickets_pull_requests_read_by_number_and_state() {
        let pr = |number, draft| WorkItem {
            kind: Kind::Pr,
            number,
            title: format!("PROJ-1 change {number}"),
            state: "OPEN".into(),
            draft,
            ..Default::default()
        };
        let (open, draft) = (pr(182, false), pr(190, true));
        assert_eq!(prs_mark(&[]), None);
        assert_eq!(prs_mark(&[&open]).as_deref(), Some("#182"));
        assert_eq!(prs_mark(&[&open, &draft]).as_deref(), Some("#182 +1"));
        assert_eq!(
            prs_tip(&[&open, &draft]),
            "#182 · Open: PROJ-1 change 182\n#190 · Draft: PROJ-1 change 190"
        );
    }

    #[test]
    fn a_tickets_pull_request_reads_with_its_checks_and_review() {
        let status = PrStatus {
            passed: 2,
            failed: 1,
            review: Review::ChangesRequested,
            ..Default::default()
        };
        assert_eq!(
            pr_line("Open", &status, "o/r"),
            "Open · checks 1 failed, 2 passed · Changes requested · o/r"
        );
        assert_eq!(pr_line("Draft", &PrStatus::default(), "o/r"), "Draft · o/r");
    }
}
