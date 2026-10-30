//! The Inbox list's toolbar: the kind tabs with their counts and the
//! group-and-sort menu; the search, the filter menu (MonoCode
//! `InboxFiltersMenu`), Mark all as read and Refresh; then the chips:
//! the Mine and Unread toggles and every other filter in force, each
//! removable. It stays three short rows at the list's narrowest.

use bencode_ui::buttons::{ButtonVariant, IconButton, SegmentedControl};
use bencode_ui::data_display::Tag;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, prelude::*,
};

use super::grouping::{GroupBy, SortBy};
use super::{KindFilter, WorkItem, refreshing, status_choices};
use crate::app::BenCodeApp;
use crate::github::Kind;
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_frame;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::work_items::Provider;

/// MonoCode `INBOX_FILTER_MENU_WIDTH`, which the group-and-sort menu
/// shares.
const MENU_WIDTH: f32 = 228.0;

/// The toolbar's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    Filter,
    Arrange,
}

impl Menu {
    fn id(self) -> &'static str {
        match self {
            Self::Filter => "inbox-filters",
            Self::Arrange => "inbox-arrange",
        }
    }
}

/// The kind tabs' counts: every item the other filters leave, and the
/// issues and pull requests among them.
fn kind_counts(items: &[WorkItem], matches: &[usize]) -> [usize; 3] {
    let prs = (matches.iter())
        .filter(|ix| items[**ix].kind == Kind::Pr)
        .count();
    [matches.len(), matches.len() - prs, prs]
}

impl BenCodeApp {
    pub(super) fn render_inbox_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let inbox = self.inbox(cx);
        let counts = kind_counts(&inbox.items, &self.inbox_matches(cx));
        let kinds = (KindFilter::ALL.iter().zip(counts)).fold(
            SegmentedControl::new("inbox-kind", inbox.kind.key()).size(ControlSize::Sm),
            |control, ((_, key, label), count)| {
                control.segment(*key, format!("{label} {count}"), None)
            },
        );
        let any_unseen =
            (self.shown_inbox_items(cx).into_iter()).any(|ix| inbox.is_unseen(&inbox.items[ix]));
        let row = || div().flex().items_center().gap_1().h(px(36.0)).px_2();
        div()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .child(
                row()
                    .child(div().flex_1().min_w_0().child(kinds.on_change(cx.listener(
                        |this, key: &SharedString, _, cx| {
                            this.inbox_state.as_mut(cx).kind = KindFilter::from_key(key);
                            cx.notify();
                        },
                    ))))
                    .child(self.render_inbox_toolbar_button(Menu::Arrange, cx))
                    .children(self.render_inbox_toolbar_menu(Menu::Arrange, cx)),
            )
            .child(
                row()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Xs)
                            .color(colors.fg.ink(Ink::Secondary)),
                    )
                    .child(
                        div()
                            // Esc hands the keyboard back to the list.
                            .key_context("InboxFilter")
                            .flex_1()
                            .min_w_0()
                            .text_size(Text::Base.px())
                            .child(self.inbox_search_input.clone()),
                    )
                    .child(self.render_inbox_toolbar_button(Menu::Filter, cx))
                    .children(self.render_inbox_toolbar_menu(Menu::Filter, cx))
                    .child(
                        IconButton::new("inbox-mark-read", IconName::CheckCheck)
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("Mark all as read")
                            .disabled(!any_unseen)
                            .on_click(cx.listener(|this, _, _, cx| this.mark_inbox_read(cx))),
                    )
                    .child(if inbox.loading {
                        refreshing("inbox-refreshing", cx)
                    } else {
                        IconButton::new("inbox-refresh", IconName::RefreshCw)
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_inbox(cx)))
                            .into_any_element()
                    }),
            )
            .child(self.render_inbox_chips(cx))
    }

    /// The Mine and Unread toggles, then the source and statuses in force,
    /// each removable; they wrap when the list is narrow.
    fn render_inbox_chips(&self, cx: &Context<Self>) -> impl IntoElement {
        let inbox = self.inbox(cx);
        let app = cx.entity().downgrade();
        type Change = fn(&mut BenCodeApp, &mut Context<BenCodeApp>);
        let act = |change: Change| {
            let app = app.clone();
            move |_: &mut gpui::Window, cx: &mut gpui::App| {
                if let Err(err) = app.update(cx, change) {
                    log::debug!("inbox chip after app drop: {err:#}");
                }
            }
        };
        let toggle = |id: &'static str, label: &'static str, on: bool, change: Change| {
            let tag = Tag::new(id, label).selected(on).on_click(act(change));
            if on { tag.on_remove(act(change)) } else { tag }
        };
        let mut chips = vec![
            toggle(
                "inbox-chip-mine",
                "Mine",
                inbox.assigned_to_me,
                BenCodeApp::toggle_inbox_mine,
            ),
            toggle(
                "inbox-chip-unread",
                "Unread",
                inbox.unread_only,
                BenCodeApp::toggle_inbox_unread,
            ),
        ];
        if let Some(source) = inbox.source_filter() {
            chips.push(
                Tag::new("inbox-chip-source", source.label())
                    .selected(true)
                    .on_remove(act(|this, cx| {
                        this.inbox_state.as_mut(cx).source = None;
                        cx.notify();
                    })),
            );
        }
        for status in &inbox.statuses {
            let (app, status) = (app.clone(), status.clone());
            chips.push(
                Tag::new(
                    SharedString::from(format!("inbox-chip-status-{status}")),
                    status.clone(),
                )
                .selected(true)
                .on_remove(move |_, cx| {
                    let removed = app.update(cx, |this, cx| {
                        this.inbox_state
                            .as_mut(cx)
                            .statuses
                            .retain(|s| *s != status);
                        cx.notify();
                    });
                    if let Err(err) = removed {
                        log::debug!("inbox chip after app drop: {err:#}");
                    }
                }),
            );
        }
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_2()
            .pb(px(6.0))
            .children(chips)
    }

    /// The statuses the filter lists: those of the items the other
    /// filters leave, Backlog's in their workflow's order.
    fn inbox_status_choices(&self, cx: &gpui::App) -> Vec<String> {
        let inbox = self.inbox(cx);
        let source = inbox.source_filter();
        let items: Vec<&WorkItem> = (inbox.items.iter())
            .filter(|item| inbox.kind.admits(item.kind))
            .filter(|item| source.is_none_or(|only| item.provider == only))
            .collect();
        let ordered = self.inbox_workflow(items.iter().copied());
        let mut choices = status_choices(&items, &ordered);
        // A picked status stays listed, so it can be unpicked.
        for picked in &inbox.statuses {
            if !choices.contains(picked) {
                choices.push(picked.clone());
            }
        }
        choices
    }

    /// MonoCode's filter button (`ListFilter`, `size-6 rounded-md`), and
    /// the group-and-sort one beside the kinds: the glyph at half strength
    /// until hovered, opened or in force.
    fn render_inbox_toolbar_button(&self, menu: Menu, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let inbox = self.inbox(cx);
        let (in_force, tip) = match menu {
            Menu::Filter => (
                inbox.source_filter().is_some() || !inbox.statuses.is_empty(),
                SharedString::from("Filter inbox"),
            ),
            Menu::Arrange => {
                let prefs = &inbox.list_prefs;
                let group = GroupBy::ALL.iter().find(|(g, _)| *g == prefs.group);
                let sort = SortBy::ALL.iter().find(|(s, _)| *s == prefs.sort);
                (
                    prefs.group != GroupBy::None || prefs.sort != SortBy::Updated,
                    SharedString::from(format!(
                        "Group by {} · Sort by {}",
                        group.map_or("", |g| g.1).to_lowercase(),
                        sort.map_or("", |s| s.1).to_lowercase(),
                    )),
                )
            }
        };
        let active = in_force || inbox.menu.is_some_and(|(open, _)| open == menu);
        let group = SharedString::from(menu.id());
        let glyph = move |color: gpui::Hsla| match menu {
            Menu::Filter => ExtraIcon::ListFilter.icon().size(IconSize::Xs).color(color),
            Menu::Arrange => Icon::new(IconName::Rows2).size(IconSize::Xs).color(color),
        };
        div()
            .id(menu.id())
            .group(group.clone())
            .relative()
            .size(px(24.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .when(active, |el| el.bg(fg.fill(Fill::Selected)))
            .when(!active, |el| el.hover(move |s| s.bg(fg.fill(Fill::Strong))))
            .tooltip(Tooltip::text(tip))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    let open = this.inbox(cx).menu.is_some_and(|(open, _)| open == menu);
                    this.inbox_mut(cx, |inbox| {
                        // Only a menu already open hears this press as
                        // outside it; with none, the mark would linger.
                        inbox.menu_button_hit = inbox.menu.is_some();
                        inbox.menu = (!open).then(|| {
                            (
                                menu,
                                gpui::point(
                                    event.position.x - px(MENU_WIDTH - 12.0),
                                    event.position.y + px(14.0),
                                ),
                            )
                        });
                    });
                    cx.notify();
                }),
            )
            .map(|el| {
                if active {
                    el.child(glyph(fg))
                } else {
                    el.child(
                        div()
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(glyph(fg.ink(Ink::Secondary))),
                    )
                    .child(
                        div()
                            .absolute()
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .child(glyph(fg)),
                    )
                }
            })
            .into_any_element()
    }

    /// Menu `menu`, while it is open.
    fn render_inbox_toolbar_menu(&self, menu: Menu, cx: &Context<Self>) -> Option<AnyElement> {
        let (open, position) = self.inbox(cx).menu?;
        if open != menu {
            return None;
        }
        let body = match menu {
            Menu::Filter => self.render_inbox_filter_menu(cx),
            Menu::Arrange => self.render_inbox_arrange_menu(cx),
        };
        let menu = popover_frame(cx)
            .id(SharedString::from(format!("{}-menu", menu.id())))
            .occlude()
            .w(px(MENU_WIDTH))
            .max_h(px(480.0))
            .overflow_y_scroll()
            .p_1()
            // A press on the menu's button toggles the menu itself; only
            // once every handler ran is it known whether it was outside.
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    let hit =
                        this.inbox_mut(cx, |inbox| std::mem::take(&mut inbox.menu_button_hit));
                    if !hit {
                        this.inbox_state.as_mut(cx).menu = None;
                        cx.notify();
                    }
                });
            }))
            .child(body);
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(position)
                    .snap_to_window()
                    .child(menu),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    /// MonoCode `InboxFiltersMenu`: the source (once there is more than
    /// GitHub), the statuses to show, and Clear filters once anything is
    /// set.
    fn render_inbox_filter_menu(&self, cx: &Context<Self>) -> gpui::Div {
        let fg = cx.theme().colors.fg;
        let source = self.inbox(cx).source_filter();
        let mut body = div();
        if self.inbox(cx).backlog_connected {
            body = body.child(menu_section("Source", cx));
            let choices = [
                (None, "All sources"),
                (Some(Provider::GitHub), Provider::GitHub.label()),
                (Some(Provider::Backlog), Provider::Backlog.label()),
            ];
            for (choice, label) in choices {
                body = body.child(
                    menu_row(
                        SharedString::from(format!("inbox-filter-source-{label}")),
                        label.into(),
                        source == choice,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.inbox_state.as_mut(cx).source = choice;
                        cx.notify();
                    })),
                );
            }
        }
        body = body.child(menu_section("Status", cx));
        let statuses = self.inbox_status_choices(cx);
        if statuses.is_empty() {
            body = body.child(
                div()
                    .px_2()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .text_size(Text::Md.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child("No statuses yet"),
            );
        }
        for status in statuses {
            let on = self.inbox(cx).statuses.contains(&status);
            body = body.child(
                menu_row(
                    SharedString::from(format!("inbox-filter-status-{status}")),
                    status.clone().into(),
                    on,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.inbox_mut(cx, |inbox| {
                        let statuses = &mut inbox.statuses;
                        match statuses.iter().position(|s| *s == status) {
                            Some(ix) => {
                                statuses.remove(ix);
                            }
                            None => statuses.push(status.clone()),
                        }
                    });
                    cx.notify();
                })),
            );
        }
        if source.is_some() || !self.inbox(cx).statuses.is_empty() {
            body = body
                .child(div().my_1().h(px(1.0)).bg(fg.fill(Fill::Strong)))
                .child(
                    div()
                        .id("inbox-filter-clear")
                        .flex()
                        .items_center()
                        .h(px(28.0))
                        .px_2()
                        .rounded(px(8.0))
                        .text_size(Text::Md.px())
                        .line_height(gpui::relative(1.0))
                        .text_color(fg.ink(Ink::Body))
                        .cursor_pointer()
                        .hover(move |s| s.bg(fg.fill(Fill::Hover)).text_color(fg))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.inbox_state.as_mut(cx).source = None;
                            this.inbox_state.as_mut(cx).statuses.clear();
                            cx.notify();
                        }))
                        .child("Clear filters"),
                );
        }
        body
    }

    /// Group by and Sort by, each one choice, kept in `settings.json`.
    fn render_inbox_arrange_menu(&self, cx: &Context<Self>) -> gpui::Div {
        let prefs = &self.inbox(cx).list_prefs;
        let mut body = div().child(menu_section("Group by", cx));
        for (group, label) in GroupBy::ALL {
            body = body.child(
                menu_row(
                    SharedString::from(format!("inbox-group-by-{label}")),
                    label.into(),
                    prefs.group == group,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.inbox_state.as_mut(cx).list_prefs.group = group;
                    this.save_settings(cx);
                })),
            );
        }
        body = body.child(menu_section("Sort by", cx));
        for (sort, label) in SortBy::ALL {
            body = body.child(
                menu_row(
                    SharedString::from(format!("inbox-sort-by-{label}")),
                    label.into(),
                    prefs.sort == sort,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.inbox_state.as_mut(cx).list_prefs.sort = sort;
                    this.save_settings(cx);
                })),
            );
        }
        body
    }
}

/// A menu section's title.
fn menu_section(label: &'static str, cx: &gpui::App) -> gpui::Div {
    div()
        .px_2()
        .pt_2()
        .pb(px(2.0))
        .text_size(Text::Xs.px())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().colors.fg.ink(Ink::Muted))
        .child(label.to_uppercase())
}

/// A menu's choice, checked while on.
fn menu_row(
    id: SharedString,
    label: SharedString,
    checked: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .px_2()
        .rounded(px(8.0))
        .text_size(Text::Md.px())
        .line_height(gpui::relative(1.0))
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(fg.fill(Fill::Hover)))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .when(checked, |el| {
            el.child(Icon::new(IconName::Check).size(IconSize::Sm).color(fg))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_counts_split_what_the_other_filters_leave() {
        let item = |kind| WorkItem {
            kind,
            ..Default::default()
        };
        let items = vec![item(Kind::Issue), item(Kind::Pr), item(Kind::Issue)];
        assert_eq!(kind_counts(&items, &[0, 1, 2]), [3, 2, 1]);
        assert_eq!(kind_counts(&items, &[1]), [1, 0, 1]);
        assert_eq!(kind_counts(&items, &[]), [0, 0, 0]);
    }
}
