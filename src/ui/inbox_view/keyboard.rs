//! The Inbox from the keyboard: opening it hands its list the keyboard and
//! opens the first item to read, and the list's keys (`InboxList` in
//! `app/commands.rs`) act on the selected item as the detail's buttons do.

use gpui::{ClipboardItem, Context, Window};

use super::detail::DetailRow;
use super::grouping::Entry;
use super::list;
use super::{DetailView, PrTab, WorkItem};
use crate::app::BenCodeApp;
use crate::backlog::Change;
use crate::github::Kind;
use crate::ui::composer::focus_later;
use crate::work_items::Provider;

/// The first of `items` (in list order, each with whether it is unread)
/// that is unread, else the first.
fn first_to_read<T>(items: impl IntoIterator<Item = (T, bool)>) -> Option<T> {
    let mut first = None;
    for (item, unseen) in items {
        if unseen {
            return Some(item);
        }
        first.get_or_insert(item);
    }
    first
}

/// An item's key and title, ready to paste into a commit or a chat.
pub(super) fn ticket_line(item: &WorkItem) -> String {
    format!("{} {}", item.identifier, item.title)
}

impl BenCodeApp {
    /// The list takes the keyboard once the Inbox is drawn.
    pub(crate) fn focus_inbox_list(&self, cx: &mut gpui::App) {
        focus_later(self.inbox_focus.clone(), cx);
    }

    /// Opens the first item in list order that needs the user or is
    /// unread, else the first one, scrolled to the top with its header.
    pub(crate) fn select_first_inbox_item(&mut self, cx: &mut Context<Self>) {
        let entries = self.inbox_entries(cx);
        let inbox = self.inbox(cx);
        let picked = first_to_read(entries.iter().enumerate().filter_map(|(line, entry)| {
            let Entry::Item(ix) = entry else {
                return None;
            };
            let item = &inbox.items[*ix];
            Some((
                (line, item.key()),
                inbox.needs_you(item) || inbox.is_unseen(item),
            ))
        }));
        if let Some((line, key)) = picked {
            list::scroll_to_line(&entries, line, &inbox.scroll);
            self.select_inbox_item(key, cx);
        }
    }

    /// `m`: only items assigned to the user, or all again.
    pub(crate) fn toggle_inbox_mine(&mut self, cx: &mut Context<Self>) {
        self.inbox_mut(cx, |inbox| inbox.assigned_to_me = !inbox.assigned_to_me);
        self.refresh_inbox(cx);
    }

    /// `⇧U`: only unread items, or all again.
    pub(crate) fn toggle_inbox_unread(&mut self, cx: &mut Context<Self>) {
        self.inbox_state.as_mut(cx).unread_only = !self.inbox(cx).unread_only;
        cx.notify();
    }

    fn selected_inbox_item(&self, cx: &gpui::App) -> Option<WorkItem> {
        let inbox = self.inbox(cx);
        inbox
            .selected
            .as_deref()
            .and_then(|key| inbox.item(key))
            .cloned()
    }

    /// ⌘↩: the detail's main button. Open thread when the item has one;
    /// else Send to agent, or Review with agent for a pull request. With
    /// items picked, the bar's Send to agents.
    pub(crate) fn send_selected_inbox_item(&mut self, cx: &mut Context<Self>) {
        if self.inbox(cx).picks.ids.len() >= 2 {
            self.open_inbox_batch(super::StartWith::default(), cx);
        } else if let Some(item) = self.selected_inbox_item(cx) {
            self.open_or_start_inbox_item(&item.key(), cx);
        }
    }

    pub(crate) fn open_selected_inbox_item(&self, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_inbox_item(cx)
            && !item.url.is_empty()
        {
            cx.open_url(&item.url);
        }
    }

    /// `b`: the selected item in a tab beside the chat.
    pub(crate) fn open_selected_inbox_item_beside(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_inbox_item(cx) {
            self.open_work_item_beside(&item.key(), cx);
        }
    }

    pub(crate) fn copy_selected_inbox_ticket(&self, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_inbox_item(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(ticket_line(&item)));
        }
    }

    /// `/`: the filter field takes the keyboard.
    pub(crate) fn focus_inbox_filter(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = gpui::Focusable::focus_handle(self.inbox_search_input.read(cx), cx);
        window.focus(&handle, cx);
    }

    /// Esc in the filter field: back to the list.
    pub(crate) fn return_to_inbox_list(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.inbox_focus, cx);
    }

    /// `u`: the selected item read or unread again.
    pub(crate) fn toggle_selected_inbox_read(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_inbox_item(cx) else {
            return;
        };
        if self.inbox_state.as_mut(cx).toggle_seen(&item) {
            self.save_settings(cx);
            // The rail's unread mark follows the app.
            cx.notify();
        }
    }

    /// `a`: a Backlog issue assigned to the connected user, as the
    /// Assignee field's "Assign to me". GitHub items have no such action.
    pub(crate) fn assign_selected_inbox_item(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self
            .selected_inbox_item(cx)
            .filter(|item| item.provider == Provider::Backlog)
        else {
            return;
        };
        let Some(Ok(options)) = self.backlog.field_options.get(&item.container_id) else {
            return;
        };
        let me = options.me;
        let key = item.key();
        let mine = (self.inbox(cx).details.get(&key))
            .and_then(|d| d.as_ref().ok())
            .and_then(|d| d.details.backlog.as_ref())
            .and_then(|fields| fields.assignee.as_ref())
            .is_some_and(|assignee| assignee.id == me);
        if me != 0 && !mine {
            self.update_backlog_issue(&key, Change::Assignee(Some(me)), cx);
        }
    }

    /// `c`: the comment box takes the keyboard, scrolled into view.
    pub(crate) fn focus_inbox_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected_inbox_item(cx) else {
            return;
        };
        if item.kind == Kind::Pr && self.inbox(cx).pr_tab == PrTab::Files {
            self.set_inbox_pr_tab(PrTab::Summary, cx);
        }
        self.inbox(cx)
            .detail_list(DetailView::Inbox)
            .reveal(&item.key(), DetailRow::CommentForm);
        let handle = gpui::Focusable::focus_handle(self.inbox_comment_input.read(cx), cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// `t`: the description in the language last picked, or the original
    /// while a translation shows.
    pub(crate) fn toggle_selected_inbox_translation(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_inbox_item(cx).map(|item| item.key()) else {
            return;
        };
        let translations = &self.inbox(cx).translations;
        if translations.shown(&key).is_some() {
            self.toggle_inbox_original(&key, cx);
        } else {
            let language = translations.language;
            self.translate_inbox_item(&key, language, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_unread_item_opens_else_the_first() {
        assert_eq!(
            first_to_read([("a", false), ("b", true), ("c", true)]),
            Some("b")
        );
        assert_eq!(first_to_read([("a", false), ("b", false)]), Some("a"));
        assert_eq!(first_to_read(Vec::<(&str, bool)>::new()), None);
    }
}
