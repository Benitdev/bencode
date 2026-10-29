//! A row of the Inbox's list (MonoCode `InboxCard`, reshaped): a meta line
//! (status, short key, type or labels, due and priority marks, a pull
//! request's checks and settled review, a ticket's pull requests, its
//! threads' state (`threads.rs`), CI repairs, age and the unread dot)
//! over a title of up to two lines, bold only while unread, in a kept
//! translation when there is one (`translate.rs`).
//! Rows keep one height, so the virtual list (`list.rs`) knows every
//! line's height without measuring.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, div, prelude::*,
};
use jiff::civil::Date;

use super::properties::{prs_mark, prs_tip};
use super::{status_mark, updated_ms};
use crate::app::BenCodeApp;
use crate::app::session_list::format_age;
use crate::github::WorkItem;
use crate::ui::composer::inbox_card::label_chip;
use crate::ui::scale::px;
use crate::ui::spinner::terminal_spinner;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};
use crate::work_items::{ChecksMark, PrStatus, Provider, Review};

/// The meta line's and the title's line heights: a row is the meta line
/// over two title lines, whatever the title's length.
const META_LINE: f32 = 16.0;
const TITLE_LINE: f32 = 18.0;
const TITLE_GAP: f32 = 3.0;
const ROW_PAD_Y: f32 = 8.0;
/// Below the card, between rows.
const ROW_GAP: f32 = 2.0;
pub(super) const ROW_HEIGHT: f32 =
    ROW_PAD_Y * 2.0 + META_LINE + TITLE_GAP + TITLE_LINE * 2.0 + ROW_GAP;
/// A due date this close shows on the row.
const DUE_SOON_DAYS: i64 = 5;

/// What the listed Backlog issues' keys share up to a `_` or `-`
/// (`ACME_SHOP_`), which the rows leave out: the rest still tells
/// the projects apart. Taken when the list lands.
pub(super) fn backlog_key_prefix(items: &[WorkItem]) -> String {
    common_key_prefix(
        items
            .iter()
            .filter(|item| item.provider == Provider::Backlog)
            .map(|item| item.repo.as_str()),
    )
}

/// The longest common prefix of `projects`, cut back to just after its
/// last `_` or `-`; empty when they share no such part.
fn common_key_prefix<'a>(projects: impl IntoIterator<Item = &'a str>) -> String {
    let mut projects = projects.into_iter();
    let Some(first) = projects.next() else {
        return String::new();
    };
    let mut common = first;
    for project in projects {
        let len = common
            .char_indices()
            .zip(project.chars())
            .find(|((_, a), b)| a != b)
            .map_or(common.len().min(project.len()), |((at, _), _)| at);
        common = &common[..len];
    }
    common
        .rfind(['_', '-'])
        .map_or_else(String::new, |at| common[..=at].to_string())
}

/// Issue key `identifier` without `prefix` (`C-289`), or whole where the
/// prefix is not its own.
pub(crate) fn short_key<'a>(identifier: &'a str, prefix: &str) -> &'a str {
    match identifier.strip_prefix(prefix) {
        Some(rest) if !prefix.is_empty() && !rest.is_empty() => rest,
        _ => identifier,
    }
}

/// A due date worth showing on a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    /// Past it by this many days.
    Overdue(i64),
    /// Due in this many days (0: today).
    Soon(i64),
}

/// Days from `today` to `due_date` (`YYYY-MM-DD`); negative once past.
pub(super) fn days_until(due_date: &str, today: Date) -> Option<i64> {
    let due = Date::strptime("%Y-%m-%d", due_date).ok()?;
    Some(i64::from((due - today).get_days()))
}

/// How far any due date is, as the properties put it after the date:
/// "in 12d", "tomorrow", "today", "2d overdue".
pub(super) fn due_distance(due_date: &str, today: Date) -> Option<String> {
    Some(match days_until(due_date, today)? {
        days @ ..0 => format!("{}d overdue", -days),
        0 => "today".into(),
        1 => "tomorrow".into(),
        days => format!("in {days}d"),
    })
}

impl Due {
    /// `due_date` (`YYYY-MM-DD`) seen from `today`: overdue, due soon, or
    /// nothing to show.
    pub(super) fn of(due_date: &str, today: Date) -> Option<Self> {
        let days = days_until(due_date, today)?;
        match days {
            ..0 => Some(Self::Overdue(-days)),
            0..=DUE_SOON_DAYS => Some(Self::Soon(days)),
            _ => None,
        }
    }

    pub(super) fn label(self) -> String {
        match self {
            Self::Overdue(days) => format!("Overdue {days}d"),
            Self::Soon(0) => "Due today".into(),
            Self::Soon(1) => "Due tomorrow".into(),
            Self::Soon(days) => format!("Due in {days}d"),
        }
    }
}

/// A GitHub repository's name without its owner.
fn repo_name(repo: &str) -> &str {
    repo.rsplit_once('/').map_or(repo, |(_, name)| name)
}

impl BenCodeApp {
    pub(super) fn render_inbox_row(
        &self,
        ix: usize,
        now: i64,
        today: Date,
        cx: &Context<Self>,
    ) -> AnyElement {
        let inbox = self.inbox(cx);
        let item = &inbox.items[ix];
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let active = inbox.selected.as_deref() == Some(key.as_str());
        let picked = inbox.picks.ids.contains(&key);
        let unseen = inbox.is_unseen(item);
        let (icon, tint, status) = status_mark(item, cx);
        let dark = cx.theme().is_dark();
        let selection = fg.opacity(if dark { 0.10 } else { 0.06 });
        // Picked rows take the accent; the open one among them more of it
        // and a bar at its edge, so it still reads as the one open.
        let picked_fill = colors.accent.opacity(match (active, dark) {
            (true, true) => 0.26,
            (true, false) => 0.18,
            (false, true) => 0.14,
            (false, false) => 0.09,
        });
        let hover = fg.fill(Fill::Hover);
        let repairs = inbox.repairs.iter().filter(|r| r.item_key == key).count();
        let id = |part: &str| SharedString::from(format!("inbox-row-{part}-{key}"));

        let mark = div()
            .id(id("status"))
            .flex_none()
            .flex()
            .items_center()
            .tooltip(Tooltip::text(status))
            .child(Icon::new(icon).size(IconSize::Xs).color(tint));
        let key_text = |text: String| {
            div()
                .flex_none()
                .text_color(fg.ink(Ink::Secondary))
                .child(SharedString::from(text))
        };
        let mut lead = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_1p5()
            .when(picked, |el| {
                el.child(
                    Icon::new(IconName::SquareCheck)
                        .size(IconSize::Xs)
                        .color(colors.accent),
                )
            })
            .child(mark);
        lead = match item.provider {
            Provider::Backlog => {
                let short = short_key(&item.identifier, &inbox.key_prefix);
                lead.child(
                    div()
                        .id(id("key"))
                        .flex_none()
                        .when(short != item.identifier, |el| {
                            el.tooltip(Tooltip::text(item.identifier.clone()))
                        })
                        .child(key_text(short.to_string())),
                )
                .children(item.issue_type.as_ref().map(|kind| label_chip(kind, cx)))
            }
            Provider::GitHub => lead
                .child(key_text(format!("#{}", item.number)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(fg.ink(Ink::Muted))
                        .child(SharedString::from(format!("· {}", repo_name(&item.repo)))),
                )
                // One label: the repository's name needs the room more.
                .children(item.labels.first().map(|l| label_chip(l, cx))),
        };
        let due = (!item.closed)
            .then(|| Due::of(&item.due_date, today))
            .flatten();
        lead = lead
            .children(due.map(|due| {
                let tone = match due {
                    Due::Overdue(_) => colors.danger,
                    Due::Soon(_) => Status::Attention.color(fg),
                };
                div()
                    .flex_none()
                    .text_color(tone)
                    .child(SharedString::from(due.label()))
            }))
            .when(item.high_priority, |el| {
                el.child(
                    div()
                        .id(id("priority"))
                        .flex_none()
                        .flex()
                        .items_center()
                        .tooltip(Tooltip::text(format!("Priority: {}", item.priority)))
                        .child(
                            Icon::new(IconName::ArrowUp)
                                .size(IconSize::Xs)
                                .color(colors.danger),
                        ),
                )
            });

        let meta = div()
            .h(px(META_LINE))
            .flex()
            .items_center()
            .gap_1p5()
            .text_size(Text::Sm.px())
            .child(lead)
            .children(pr_status_marks(&key, &item.pr_status, cx))
            .children(self.render_inbox_row_prs(&key, cx))
            .children(self.render_inbox_row_threads(&key, cx))
            .when(repairs > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_0p5()
                        .text_color(colors.accent)
                        .child(
                            Icon::new(IconName::MessageSquare)
                                .size(IconSize::Xs)
                                .color(colors.accent),
                        )
                        .child(repairs.to_string()),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_color(fg.ink(Ink::Muted))
                    .child(format_age(updated_ms(item), now)),
            )
            .when(unseen, |el| {
                el.child(
                    div()
                        .size(px(6.0))
                        .flex_none()
                        .rounded_full()
                        .bg(colors.accent),
                )
            });
        // A kept translation of the title as listed, unless the item was
        // turned back to its original; the original is the tooltip.
        let translated = inbox.translations.list_title(&key).cloned();
        let title = div()
            .id(id("title"))
            .mt(px(TITLE_GAP))
            .text_size(Text::Md.px())
            .line_height(px(TITLE_LINE))
            .line_clamp(2)
            .text_ellipsis()
            .map(|el| {
                if unseen {
                    el.font_weight(FontWeight::SEMIBOLD).text_color(fg)
                } else {
                    el.text_color(fg.ink(Ink::Body))
                }
            })
            .map(|el| match translated {
                Some(translated) => el
                    .tooltip(Tooltip::text(item.title.clone()))
                    .child(translated),
                None => el.child(SharedString::from(item.title.clone())),
            });

        div()
            .id(SharedString::from(format!("inbox-row-{key}")))
            .h(px(ROW_HEIGHT))
            .pb(px(ROW_GAP))
            .child(
                div()
                    // Its own id: GPUI redraws on hover only for an
                    // element that keeps state.
                    .id(id("card"))
                    .size_full()
                    .px(px(10.0))
                    .py(px(ROW_PAD_Y))
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .cursor_pointer()
                    .map(|el| match (picked, active) {
                        (true, _) => el.bg(picked_fill),
                        (false, true) => el.bg(selection),
                        (false, false) => el.hover(move |s| s.bg(hover)),
                    })
                    .when(picked && active, |el| {
                        el.relative().child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(3.0))
                                .bg(colors.accent),
                        )
                    })
                    .child(meta)
                    .child(title),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // The list takes the keyboard, so its keys go on from here.
                window.focus(&this.inbox_focus, cx);
                // ⌘ and ⇧ pick items to send together (`picks.rs`).
                let mods = event.modifiers();
                if mods.shift {
                    this.pick_inbox_range(&key, mods.platform, cx);
                } else if mods.platform {
                    this.toggle_inbox_pick(&key, cx);
                } else {
                    this.select_inbox_item(key.clone(), cx)
                }
            }))
            .into_any_element()
    }
}

/// A pull request's checks as one glyph: passed, failed, or the spinner
/// (a busy mark) while some still run.
pub(super) fn checks_glyph(mark: ChecksMark, cx: &gpui::App) -> AnyElement {
    let colors = &cx.theme().colors;
    let icon = |name: IconName, color| {
        Icon::new(name)
            .size(IconSize::Xs)
            .color(color)
            .into_any_element()
    };
    match mark {
        ChecksMark::Passed => icon(IconName::Check, colors.success),
        ChecksMark::Failed => icon(IconName::X, colors.danger),
        ChecksMark::Running => {
            terminal_spinner(Status::Attention.color(colors.fg), cx).into_any_element()
        }
    }
}

/// A settled review's tint: approved, or changes requested.
pub(super) fn review_color(review: Review, cx: &gpui::App) -> Option<gpui::Hsla> {
    let colors = &cx.theme().colors;
    match review {
        Review::Approved => Some(colors.success),
        Review::ChangesRequested => Some(colors.danger),
        Review::None | Review::Required => None,
    }
}

/// A pull request's marks on its row: its checks (the counts in the
/// tooltip) and its review once settled. Read from the list, never
/// loaded per row.
fn pr_status_marks(key: &str, status: &PrStatus, cx: &gpui::App) -> Vec<AnyElement> {
    let mut marks = Vec::new();
    if let Some(mark) = status.checks_mark() {
        marks.push(
            div()
                .id(SharedString::from(format!("inbox-row-checks-{key}")))
                .flex_none()
                .flex()
                .items_center()
                .tooltip(Tooltip::text(format!(
                    "Checks: {}",
                    status.checks_summary()
                )))
                .child(checks_glyph(mark, cx))
                .into_any_element(),
        );
    }
    if let Some(color) = review_color(status.review, cx) {
        let short = match status.review {
            Review::ChangesRequested => "Changes",
            review => review.label(),
        };
        marks.push(
            div()
                .id(SharedString::from(format!("inbox-row-review-{key}")))
                .flex_none()
                .text_color(color)
                .tooltip(Tooltip::text(status.review.label()))
                .child(short)
                .into_any_element(),
        );
    }
    marks
}

impl BenCodeApp {
    /// A ticket's mark for the pull requests named after it: the first's
    /// state and number, tinted as its status is; a click opens it.
    /// `None` when it has none.
    fn render_inbox_row_prs(&self, key: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let prs = self.inbox(cx).ticket_prs(key);
        let label = prs_mark(&prs)?;
        let first = prs.first()?;
        let (icon, tint, _) = status_mark(first, cx);
        let hover = cx.theme().colors.fg.fill(Fill::Hover);
        let open = first.key();
        Some(
            div()
                .id(SharedString::from(format!("inbox-row-prs-{key}")))
                .flex_none()
                .flex()
                .items_center()
                .gap_0p5()
                .px(px(3.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .text_color(tint)
                .tooltip(Tooltip::text(prs_tip(&prs)))
                // The row's own click selects the ticket: this one selects
                // the pull request instead.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.inbox_focus, cx);
                    this.select_inbox_item(open.clone(), cx);
                }))
                .child(Icon::new(icon).size(IconSize::Xs).color(tint))
                .child(label)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backlog(key: &str) -> WorkItem {
        WorkItem {
            provider: Provider::Backlog,
            identifier: key.into(),
            repo: key.rsplit_once('-').unwrap().0.into(),
            ..Default::default()
        }
    }

    fn shortened(keys: &[&str]) -> Vec<String> {
        let items: Vec<WorkItem> = keys.iter().map(|k| backlog(k)).collect();
        let prefix = backlog_key_prefix(&items);
        keys.iter()
            .map(|k| short_key(k, &prefix).to_string())
            .collect()
    }

    #[test]
    fn short_keys_drop_what_the_projects_share() {
        assert_eq!(
            shortened(&["ACME_SHOP_C-289", "ACME_SHOP_D-1147"]),
            ["C-289", "D-1147"]
        );
        assert_eq!(shortened(&["ACME_SHOP_C-289"]), ["C-289"]);
        assert_eq!(shortened(&["PROJ-42"]), ["PROJ-42"]);
        assert_eq!(
            shortened(&["BENCODE-1", "BACKLOG-2"]),
            ["BENCODE-1", "BACKLOG-2"]
        );
        assert_eq!(
            shortened(&["WEB_APP-1", "WEB_API-2"]),
            ["APP-1", "API-2"],
            "cut back to the last separator, not mid-word"
        );
        // GitHub items take no part.
        let mut items = vec![backlog("ACME_X-1")];
        items.push(WorkItem {
            repo: "owner/repo".into(),
            ..Default::default()
        });
        assert_eq!(backlog_key_prefix(&items), "ACME_");
        assert_eq!(backlog_key_prefix(&[]), "");
        assert_eq!(short_key("#12", "ACME_"), "#12");
    }

    #[test]
    fn due_dates_show_only_when_close_or_past() {
        let today = Date::new(2026, 10, 28).unwrap();
        assert_eq!(Due::of("2026-10-27", today), Some(Due::Overdue(1)));
        assert_eq!(Due::of("2026-10-28", today), Some(Due::Soon(0)));
        assert_eq!(Due::of("2026-11-02", today), Some(Due::Soon(5)));
        assert_eq!(Due::of("2026-11-03", today), None);
        assert_eq!(Due::of("", today), None);
        assert_eq!(Due::of("soon", today), None);
        assert_eq!(Due::Overdue(3).label(), "Overdue 3d");
        assert_eq!(Due::Soon(0).label(), "Due today");
        assert_eq!(Due::Soon(1).label(), "Due tomorrow");
        assert_eq!(Due::Soon(4).label(), "Due in 4d");
    }

    #[test]
    fn any_due_date_reads_as_a_distance() {
        let today = Date::new(2026, 10, 28).unwrap();
        let distance = |date| due_distance(date, today);
        assert_eq!(distance("2026-11-02").as_deref(), Some("in 5d"));
        assert_eq!(distance("2026-12-01").as_deref(), Some("in 34d"));
        assert_eq!(distance("2026-10-29").as_deref(), Some("tomorrow"));
        assert_eq!(distance("2026-10-28").as_deref(), Some("today"));
        assert_eq!(distance("2026-10-26").as_deref(), Some("2d overdue"));
        assert_eq!(distance(""), None);
    }

    #[test]
    fn repositories_lose_their_owner() {
        assert_eq!(repo_name("Acme/shop-backend"), "shop-backend");
        assert_eq!(repo_name("solo"), "solo");
    }
}
