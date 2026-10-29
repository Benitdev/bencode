//! The Inbox's list: the lines `grouping::arrange` returns, group headers
//! and item rows (`row.rs`), virtual. Every line's height is known, so
//! only those in view are built and spacers keep the rest
//! (`ui/virtual_rows.rs`); a header's click folds its group.

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    SharedString, Styled, div, prelude::*,
};

use super::grouping::{self, Entry, Header};
use super::{KindFilter, Status};
use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::virtual_rows;

/// The list's padding: its lines start this far into its content.
const LIST_PAD: f32 = 6.0;

/// Scrolls the least that shows line `line`, its group's header too when
/// it is the group's first item.
pub(super) fn reveal_line(entries: &[Entry], line: usize, scroll: &ScrollHandle) {
    let heights = grouping::heights(entries);
    virtual_rows::reveal(
        &heights,
        grouping::with_header(entries, line),
        scroll,
        LIST_PAD,
    );
    virtual_rows::reveal(&heights, line, scroll, LIST_PAD);
}

/// Scrolls line `line` (or its group's header) to the top. Before the
/// list was laid out once, no reveal can know its viewport.
pub(super) fn scroll_to_line(entries: &[Entry], line: usize, scroll: &ScrollHandle) {
    let top = grouping::top_of(entries, grouping::with_header(entries, line), LIST_PAD) - LIST_PAD;
    let offset = scroll.offset();
    scroll.set_offset(gpui::point(offset.x, -px(top)));
}

impl BenCodeApp {
    /// The list, or what stands for it, holding the list's keyboard
    /// focus either way: the Inbox gives it the keyboard as it opens,
    /// before its first list lands.
    pub(super) fn render_inbox_list(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .id("inbox-list-focus")
            .key_context("InboxList")
            .track_focus(&self.inbox_focus)
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_inbox_rows(cx))
            .into_any_element()
    }

    fn render_inbox_rows(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let message = |text: String| {
            div()
                .px_3()
                .py_3()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Secondary))
                .child(text)
                .into_any_element()
        };
        let inbox = self.inbox(cx);
        // Backlog alone fills the list too; GitHub's setup hint shows
        // only while nothing is connected.
        match inbox.status {
            None => return message("Loading…".into()),
            Some(Status::NotInstalled) if !inbox.backlog_connected => {
                return message(
                    "Install the GitHub CLI (`brew install gh`) to fill the Inbox, or connect Backlog in Settings › Integrations.".into(),
                );
            }
            Some(Status::SignedOut) if !inbox.backlog_connected => {
                return message(
                    "Run `gh auth login` in a terminal to connect GitHub, or connect Backlog in Settings › Integrations.".into(),
                );
            }
            Some(_) => {}
        }
        let entries = self.inbox_entries(cx);
        if entries.is_empty() {
            let narrowed = !self.inbox_search_input.read(cx).text().trim().is_empty()
                || inbox.kind != KindFilter::All
                || inbox.source_filter().is_some()
                || !inbox.statuses.is_empty()
                || inbox.assigned_to_me
                || inbox.unread_only;
            let text = if inbox.loading {
                "Loading…".to_string()
            } else if let Some(error) = inbox.errors.first() {
                error.clone()
            } else if self.inbox_projects_shown(cx).is_empty() && !inbox.backlog_connected {
                "Open a project to fill the inbox".to_string()
            } else if narrowed {
                "No issues or pull requests match these filters".to_string()
            } else {
                "No matching issues or pull requests".to_string()
            };
            return message(text);
        }
        let now = crate::app::now_ms();
        let today = jiff::Zoned::now().date();
        let scroll = &inbox.scroll;
        let window = virtual_rows::for_scroll(&grouping::heights(&entries), scroll, LIST_PAD);
        let lines: Vec<AnyElement> = entries[window.range.clone()]
            .iter()
            .map(|entry| match entry {
                Entry::Header(header) => self.render_inbox_group_header(header, cx),
                Entry::Item(ix) => self.render_inbox_row(*ix, now, today, cx),
            })
            .collect();
        crate::ui::scrollbar::framed(
            "inbox-list-scrollbar",
            scroll,
            div()
                .id("inbox-list")
                .track_scroll(scroll)
                .overflow_y_scroll()
                .size_full()
                .p(px(LIST_PAD))
                .pr(px(LIST_PAD) + crate::ui::scrollbar::gutter(scroll))
                .when(window.above > 0.0, |el| el.child(div().h(px(window.above))))
                .children(lines)
                .when(window.below > 0.0, |el| el.child(div().h(px(window.below)))),
        )
        .into_any_element()
    }

    /// A group's header: its fold mark, name and count; "Needs you" in
    /// the accent. A click folds the group or opens it.
    fn render_inbox_group_header(&self, header: &Header, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let hover = fg.fill(Fill::Hover);
        let tint = if header.pinned {
            colors.accent
        } else {
            fg.ink(Ink::Secondary)
        };
        let chevron = if header.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };
        let id = header.id.clone();
        div()
            .id(SharedString::from(format!("inbox-group-{}", header.id)))
            .h(px(grouping::HEADER_HEIGHT))
            .flex()
            .items_center()
            .gap_1p5()
            .px(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .text_size(Text::Sm.px())
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.inbox_focus, cx);
                this.toggle_inbox_group(&id, cx);
            }))
            .child(
                Icon::new(chevron)
                    .size(IconSize::Xs)
                    .color(fg.ink(Ink::Muted)),
            )
            .when(header.pinned, |el| {
                el.child(Icon::new(IconName::User).size(IconSize::Xs).color(tint))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tint)
                    .child(SharedString::from(header.label.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(fg.ink(Ink::Muted))
                    .child(header.count.to_string()),
            )
            .into_any_element()
    }

    /// Folds group `id` or opens it, kept in `settings.json`.
    fn toggle_inbox_group(&mut self, id: &str, cx: &mut Context<Self>) {
        self.inbox_state.as_mut(cx).list_prefs.toggle(id);
        self.save_settings(cx);
    }
}
