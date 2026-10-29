//! BenCode's own: several items picked in the Inbox's list, to send to
//! agents at once (`batch.rs`). ⌘-click toggles an item, ⇧-click picks
//! the items from the last pick (else the open item) to it in list order
//! (⌘⇧ adds them to the picks), `x` toggles the open item and Esc lets
//! them all go. The picks are apart from the item open in the detail,
//! which keeps its own highlight; once two are picked, a bar under the
//! list offers Send to agents.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::menus::{Menu, MenuItem, SplitButton};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use super::StartWith;
use super::grouping::Entry;
use crate::app::BenCodeApp;
use crate::app::session_list::Selection;
use crate::ui::app_callback::app_callback;
use crate::ui::tokens::{Ink, Text, Tone as _};

/// ⌘-click or `x` on `key`. With nothing picked yet, the open item is
/// picked first: ⌘-clicking a second item while reading one picks both.
pub(super) fn toggle_pick(picks: &mut Selection, key: &str, open: Option<&str>) {
    if picks.ids.is_empty()
        && let Some(open) = open.filter(|open| *open != key)
    {
        picks.toggle(open);
    }
    picks.toggle(key);
}

/// The picks in list order, then those the filters hide, in `all`'s order.
pub(super) fn ordered_picks(picks: &Selection, order: &[String], all: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = (order.iter())
        .filter(|key| picks.ids.contains(*key))
        .cloned()
        .collect();
    for key in all {
        if picks.ids.contains(key) && !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    keys
}

impl BenCodeApp {
    /// The listed items' keys in list order (folded groups left out).
    fn inbox_order(&self, cx: &gpui::App) -> Vec<String> {
        let inbox = self.inbox(cx);
        (self.inbox_entries(cx).into_iter())
            .filter_map(|entry| match entry {
                Entry::Item(ix) => Some(inbox.items[ix].key()),
                Entry::Header(_) => None,
            })
            .collect()
    }

    /// The picked items' keys, in list order.
    pub(super) fn inbox_picked_keys(&self, cx: &gpui::App) -> Vec<String> {
        let inbox = self.inbox(cx);
        let all: Vec<String> = inbox.items.iter().map(|item| item.key()).collect();
        ordered_picks(&inbox.picks, &self.inbox_order(cx), &all)
    }

    /// ⌘-click: item `key` picked, or let go.
    pub(super) fn toggle_inbox_pick(&mut self, key: &str, cx: &mut Context<Self>) {
        self.inbox_state.update(cx, |inbox, cx| {
            toggle_pick(&mut inbox.picks, key, inbox.selected.as_deref());
            cx.notify();
        });
    }

    /// ⇧-click: the items from the last pick (else the open one) to `key`,
    /// in list order; with ⌘ (`add`) added to the picks.
    pub(super) fn pick_inbox_range(&mut self, key: &str, add: bool, cx: &mut Context<Self>) {
        let order = self.inbox_order(cx);
        self.inbox_state.update(cx, |inbox, cx| {
            let open = inbox.selected.clone();
            inbox.picks.select_range(key, &order, open.as_deref(), add);
            cx.notify();
        });
    }

    /// `x`: the open item picked, or let go.
    pub(crate) fn toggle_open_inbox_pick(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.inbox(cx).selected.clone() {
            self.toggle_inbox_pick(&key, cx);
        }
    }

    /// Esc in the list: the picks let go; `false` when there were none,
    /// so Esc goes on to close the Inbox.
    pub(crate) fn clear_inbox_picks(&mut self, cx: &mut Context<Self>) -> bool {
        if self.inbox(cx).picks.ids.is_empty() {
            return false;
        }
        self.inbox_state.update(cx, |inbox, cx| {
            inbox.picks.clear();
            cx.notify();
        });
        true
    }

    /// The bar under the list once two or more items are picked: how many,
    /// Send to agents (its menu: in new worktrees, planning first) and
    /// Clear.
    pub(super) fn render_inbox_pick_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let count = self.inbox(cx).picks.ids.len();
        if count < 2 {
            return None;
        }
        let colors = &cx.theme().colors;
        let send = |label: &'static str, icon: IconName, start: StartWith| {
            MenuItem::new(label)
                .icon(icon)
                .on_click(app_callback(cx, move |this, cx| {
                    this.open_inbox_batch(start, cx)
                }))
        };
        let menu = Menu::new()
            .item(send(
                "In new worktrees",
                IconName::GitBranch,
                StartWith {
                    worktree: true,
                    ..Default::default()
                },
            ))
            .item(send(
                "Plan first",
                IconName::Lightbulb,
                StartWith {
                    plan: true,
                    ..Default::default()
                },
            ));
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(colors.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(Text::Base.px())
                        .text_color(colors.fg.ink(Ink::Secondary))
                        .child(format!("{count} selected")),
                )
                .child(
                    SplitButton::new("inbox-batch-send", "Send to agents", menu)
                        .variant(ButtonVariant::Primary)
                        .size(ControlSize::Sm)
                        .icon(IconName::Sparkles)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_inbox_batch(StartWith::default(), cx)
                        })),
                )
                .child(
                    Button::new("inbox-picks-clear", "Clear")
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_inbox_picks(cx);
                        })),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(picks: &Selection, order: &[String]) -> Vec<String> {
        ordered_picks(picks, order, order)
    }

    fn order() -> Vec<String> {
        ["a", "b", "c", "d", "e"].map(String::from).to_vec()
    }

    #[test]
    fn a_first_toggle_picks_the_open_item_too() {
        let mut picks = Selection::default();
        toggle_pick(&mut picks, "c", Some("a"));
        assert_eq!(keys(&picks, &order()), ["a", "c"]);
        toggle_pick(&mut picks, "d", Some("a"));
        assert_eq!(keys(&picks, &order()), ["a", "c", "d"]);
        toggle_pick(&mut picks, "c", Some("a"));
        assert_eq!(keys(&picks, &order()), ["a", "d"]);

        // `x` on the open item picks it alone.
        let mut picks = Selection::default();
        toggle_pick(&mut picks, "b", Some("b"));
        assert_eq!(keys(&picks, &order()), ["b"]);
        toggle_pick(&mut picks, "b", Some("b"));
        assert!(picks.ids.is_empty());
    }

    #[test]
    fn a_range_runs_from_the_last_pick_else_the_open_item() {
        let order = order();
        let mut picks = Selection::default();
        picks.select_range("d", &order, Some("b"), false);
        assert_eq!(keys(&picks, &order), ["b", "c", "d"]);

        // From the last ⌘-pick, replacing the range unless ⌘ is held.
        let mut picks = Selection::default();
        toggle_pick(&mut picks, "e", None);
        picks.select_range("c", &order, Some("a"), false);
        assert_eq!(keys(&picks, &order), ["c", "d", "e"]);
        toggle_pick(&mut picks, "a", None);
        picks.select_range("b", &order, None, true);
        assert_eq!(keys(&picks, &order), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn picks_the_filters_hide_come_last() {
        let mut picks = Selection::default();
        for key in ["e", "x", "a"] {
            toggle_pick(&mut picks, key, None);
        }
        let all: Vec<String> = ["x", "a", "e"].map(String::from).to_vec();
        assert_eq!(ordered_picks(&picks, &order(), &all), ["a", "e", "x"]);
    }
}
