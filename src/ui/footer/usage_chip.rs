//! MonoCode `app/shell/UsageProviderChip.tsx`: the footer chip with the
//! account's name, the tightest window's bar and each window's
//! "11% 6d 14h", and the details popover it opens (one card per window;
//! the account pages are in `account_views`).

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Palette};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, Styled, anchored, deferred,
    div, point, prelude::*, relative,
};

use crate::app::accounts::SignIn;
use crate::app::usage::{UsageTarget, UsageView};
use crate::app::{BenCodeApp, now_ms};
use crate::harness::accounts::{ProviderAccount, supports_accounts};
use crate::rate_limits::{
    antigravity, AccountTone, ProviderRateLimits, RateLimitProvider, RateLimitStatus, RateLimitWindow,
    WindowKind, account_status, best_alternative, chip_label, clamp_used_percent,
    format_reset_countdown, format_usage_percent, format_window_label, needs_provider_login,
    updated_label, window_tooltip,
};
use crate::ui::HarnessIcon;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_frame;

/// MonoCode's notice on the chip of a thread whose account is gone.
const REMOVED_ACCOUNT: &str = "This conversation uses a removed account";

/// MonoCode `Popover width={300} gap={7}`; the account list is wider.
const POPOVER_WIDTH: f32 = 300.0;
const ACCOUNTS_WIDTH: f32 = 340.0;
const POPOVER_GAP: f32 = 7.0;

/// MonoCode `barClass`: the bar's colour by percent used.
pub(super) fn bar_color(used_percent: f64, colors: &Palette) -> Hsla {
    if used_percent >= 90.0 {
        colors.danger
    } else if used_percent >= 80.0 {
        colors.warning
    } else {
        colors.fg.opacity(0.45)
    }
}

/// A rounded track filled to `used_percent`.
pub(super) fn usage_bar(used_percent: f64, height: f32, colors: &Palette) -> gpui::Div {
    let used = clamp_used_percent(used_percent);
    div()
        .h(px(height))
        .rounded_full()
        .overflow_hidden()
        .bg(colors.fg.opacity(0.10))
        .child(
            div()
                .h_full()
                .w(relative(used as f32 / 100.0))
                .rounded_full()
                .bg(bar_color(used, colors)),
        )
}

/// MonoCode `emptyUsageLabel`: the chip of a provider that reported no
/// windows.
fn empty_usage_label(limits: &ProviderRateLimits) -> &'static str {
    let text = limits.error.as_deref().unwrap_or_default().to_lowercase();
    let expired = text.contains("expired") || text.contains("sign-in");
    if limits.status == RateLimitStatus::Error && expired {
        "expired"
    } else {
        "—"
    }
}

/// The chip's tooltip: every window, else why there is none.
fn chip_tooltip(limits: &ProviderRateLimits, now: i64) -> String {
    let windows = limits.windows();
    if !windows.is_empty() {
        return windows
            .iter()
            .map(|(_, window)| window_tooltip(window, now))
            .collect::<Vec<_>>()
            .join(" · ");
    }
    match (&limits.error, limits.status) {
        (Some(error), _) => error.clone(),
        (None, RateLimitStatus::Unavailable) => "Not connected".into(),
        (None, _) if limits.is_loading() => "Loading usage…".into(),
        (None, _) => "Usage details".into(),
    }
}

/// What the popover needs to know about the footer's account.
pub(super) struct ChipAccounts<'a> {
    pub provider: RateLimitProvider,
    /// Empty for a provider without account profiles.
    pub accounts: &'a [ProviderAccount],
    pub active_id: &'a str,
    pub limits: &'a ProviderRateLimits,
    pub now: i64,
}

impl ChipAccounts<'_> {
    pub fn active(&self) -> Option<&ProviderAccount> {
        self.accounts.iter().find(|account| account.id == self.active_id)
    }

    pub fn active_label(&self) -> &str {
        self.active().map_or("Removed account", |account| account.label.as_str())
    }

    pub fn can_manage(&self) -> bool {
        supports_accounts(self.provider.id())
    }
}

impl BenCodeApp {
    pub(super) fn render_usage_chip(&self, target: &UsageTarget, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let now = now_ms();
        let provider = target.provider;
        let removed;
        let limits = if target.available {
            self.usage.limits(provider, &target.account_id)
        } else {
            removed = ProviderRateLimits::unavailable(REMOVED_ACCOUNT, now);
            &removed
        };
        let accounts = if supports_accounts(provider.id()) {
            self.provider_accounts(provider.id())
        } else {
            Vec::new()
        };
        let chip_accounts = ChipAccounts {
            provider,
            accounts: &accounts,
            active_id: &target.account_id,
            limits,
            now,
        };
        let windows = limits.windows();
        let faint = |text: &'static str| div().text_color(fg.opacity(0.35)).child(text);

        let chip = div()
            .id("usage-chip")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(20.0))
            .px_1()
            .rounded(px(4.0))
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(move |s| s.bg(fg.opacity(0.10)).text_color(fg))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.usage.chip_hovered = *hovered))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_usage_popover(provider, cx)))
            .tooltip(Tooltip::text(chip_tooltip(limits, now)))
            .child(HarnessIcon::new(provider.id()).size(px(12.0)))
            .map(|el| {
                if limits.is_loading() {
                    el.child(faint("···"))
                } else if limits.status == RateLimitStatus::Unavailable {
                    el.child(faint("not connected"))
                } else if windows.is_empty() {
                    el.child(faint(empty_usage_label(limits)))
                } else {
                    let tightest = windows
                        .iter()
                        .map(|(_, window)| window.used_percent)
                        .fold(0.0, f64::max);
                    // The account is only named once there is more than one;
                    // Antigravity's is the one `agy` is signed in as.
                    let account = if provider == RateLimitProvider::Antigravity {
                        self.agy_live_label().map(|label| (label, 160.0))
                    } else {
                        chip_accounts
                            .active()
                            .filter(|_| accounts.len() > 1)
                            .map(|active| (active.label.clone(), 96.0))
                    };
                    el.when_some(account, |el, (label, width)| {
                        el.child(div().max_w(px(width)).truncate().text_color(fg.opacity(0.45)).child(label))
                    })
                    .child(usage_bar(tightest, 4.0, colors).w(px(32.0)).flex_none())
                    .child(div().flex().items_center().gap_1().children(
                        windows.iter().enumerate().map(|(ix, (_, window))| {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .when(ix > 0, |el| {
                                    el.child(div().text_color(fg.opacity(0.25)).child("·"))
                                })
                                .child(format!(
                                    "{} {}",
                                    format_usage_percent(window.used_percent),
                                    chip_label(window, now)
                                ))
                        }),
                    ))
                }
            });

        // The chip's padding hangs into the footer's, so its content lines up
        // with a plain harness label (MonoCode `-mx-1`).
        div()
            .relative()
            .flex_none()
            .mx(px(-4.0))
            .child(chip)
            .when(self.usage.popover == Some(provider), |el| {
                el.child(self.render_usage_popover(&chip_accounts, target.available, cx))
            })
    }

    /// MonoCode `Popover side="top" align="start"` over the chip.
    fn render_usage_popover(&self, chip: &ChipAccounts, available: bool, cx: &Context<Self>) -> AnyElement {
        let view = if chip.can_manage() { self.usage.view } else { UsageView::Usage };
        // MonoCode `loginView`: nothing to show until the account signs in.
        let login_view = chip.can_manage()
            && available
            && !chip.limits.has_windows()
            && (needs_provider_login(chip.limits) || self.usage.sign_in != SignIn::Idle);
        let padded = view != UsageView::Usage || !login_view;

        let popover = popover_frame(cx)
            .id("usage-popover")
            .occlude()
            .w(px(if view == UsageView::Accounts { ACCOUNTS_WIDTH } else { POPOVER_WIDTH }))
            .max_h(px(460.0))
            .overflow_y_scroll()
            .when(padded, |el| el.p(px(10.0)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                // A press on the chip is its own toggle.
                if !this.usage.chip_hovered {
                    this.close_usage_popover(cx);
                }
            }))
            .map(|el| match view {
                UsageView::Accounts => el.child(self.render_account_picker(chip, cx)),
                UsageView::Add => el.child(self.render_add_account(chip.provider, cx)),
                UsageView::Usage if login_view => el
                    .child(self.render_account_switch_row(chip.active_label(), cx))
                    .child(self.render_sign_in_panel(chip.provider, cx)),
                UsageView::Usage => el.children(self.render_usage_details(chip, cx)),
            });

        // Pinned to the chip's top-left corner, which the popover's bottom-left
        // then hangs from.
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(
                deferred(
                    anchored()
                        .anchor(gpui::Anchor::BottomLeft)
                        .offset(point(px(0.0), px(-POPOVER_GAP)))
                        .snap_to_window()
                        .child(popover),
                )
                .with_priority(3),
            )
            .into_any_element()
    }

    /// The usage page: header, each window's card, and the account worth
    /// switching to when this one is low.
    fn render_usage_details(&self, chip: &ChipAccounts, cx: &Context<Self>) -> Vec<AnyElement> {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (provider, limits, now) = (chip.provider, chip.limits, chip.now);
        let windows = limits.windows();
        let card = |el: gpui::Div| {
            el.rounded(px(8.0))
                .bg(fg.opacity(0.045))
                .border_1()
                .border_color(fg.opacity(0.06))
        };

        let header = div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .px_1()
            .pt(px(2.0))
            .pb(px(10.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded(px(8.0))
                    .bg(fg.opacity(0.06))
                    .border_1()
                    .border_color(fg.opacity(0.07))
                    .child(HarnessIcon::new(provider.id()).size(px(16.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(px(16.0))
                            .child(format!("{} usage", provider.title())),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(10.0))
                            .line_height(px(16.0))
                            .text_color(fg.opacity(0.4))
                            .child(updated_label(limits, now)),
                    )
                    .when(chip.can_manage(), |el| {
                        // The account, and the way into the account list.
                        let subtitle = chip
                            .active()
                            .and_then(|account| self.accounts.identity(account))
                            .and_then(|identity| identity.subtitle());
                        el.child(
                            div().flex().child(
                                div()
                                    .id("usage-switch-account")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .min_w_0()
                                    .mt_1()
                                    .ml(px(-4.0))
                                    .px_1()
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .text_size(px(10.0))
                                    .text_color(fg.opacity(0.55))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(fg.opacity(0.10)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_usage_view(UsageView::Accounts, cx)
                                    }))
                                    .child(div().flex_none().max_w(px(140.0)).truncate().child(chip.active_label().to_string()))
                                    .when_some(subtitle, |el, subtitle| {
                                        el.child(div().min_w_0().truncate().text_color(fg.opacity(0.35)).child(subtitle))
                                    })
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .size(IconSize::Xs)
                                            .color(fg.opacity(0.55)),
                                    ),
                            ),
                        )
                    }),
            )
            .when(limits.status == RateLimitStatus::Fetching, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .mt(px(2.0))
                        .text_size(px(10.0))
                        .text_color(fg.opacity(0.4))
                        .child(spinning_icon(
                            "usage-popover-spin".into(),
                            IconName::RefreshCw,
                            IconSize::Xs,
                            fg.opacity(0.4),
                        ))
                        .child("Updating"),
                )
            });

        let body = if windows.is_empty() {
            // MonoCode `EmptyUsageState`.
            card(div())
                .px_3()
                .py_4()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg.opacity(0.65))
                        .child(if limits.is_loading() {
                            "Loading usage…"
                        } else if limits.status == RateLimitStatus::Unavailable {
                            "Not connected"
                        } else {
                            "Usage unavailable"
                        }),
                )
                .when_some(limits.error.clone(), |el, error| {
                    el.child(
                        div()
                            .mt_1()
                            .max_w(px(240.0))
                            .text_size(px(10.0))
                            .line_height(px(16.0))
                            .text_center()
                            .text_color(fg.opacity(0.4))
                            .child(error),
                    )
                })
        } else if provider == RateLimitProvider::Antigravity {
            // Each model group has its own limits; the thread's comes first.
            let mut groups = antigravity::GROUPS;
            groups.sort_by_key(|group| *group != chip.active_id);
            div().flex().flex_col().gap(px(6.0)).children(groups.into_iter().flat_map(|group| {
                let windows = self.usage.limits(provider, group).windows();
                let title = div()
                    .px_1()
                    .pt_1()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.opacity(0.4))
                    .child(antigravity::group_title(group));
                (!windows.is_empty())
                    .then_some(title)
                    .into_iter()
                    .chain(windows.into_iter().map(|(kind, window)| card(usage_window_card(kind, window, now, colors))))
                    .collect::<Vec<_>>()
            }))
        } else {
            div().flex().flex_col().gap(px(6.0)).children(
                windows
                    .iter()
                    .map(|(kind, window)| card(usage_window_card(*kind, window, now, colors))),
            )
        };

        let mut parts = vec![header.into_any_element()];
        if limits.status == RateLimitStatus::Error && !windows.is_empty() {
            parts.push(
                div()
                    .mb_2()
                    .px(px(10.0))
                    .py_2()
                    .rounded(px(8.0))
                    .bg(colors.warning.opacity(0.10))
                    .text_size(px(10.0))
                    .line_height(px(16.0))
                    .text_color(colors.warning)
                    .child("Couldn’t refresh. Showing the last available snapshot.")
                    .into_any_element(),
            );
        }
        parts.push(body.into_any_element());

        // MonoCode `SwitchSuggestion`: another account with room to spare.
        let tone = account_status(Some(limits), now).tone;
        if matches!(tone, AccountTone::Exhausted | AccountTone::Low) {
            let others = chip
                .accounts
                .iter()
                .filter(|account| account.id != chip.active_id)
                .map(|account| (account, self.usage.cached(provider, &account.id)));
            if let Some(suggestion) = best_alternative(others, now) {
                parts.push(self.render_switch_suggestion(
                    suggestion,
                    self.usage.cached(provider, &suggestion.id),
                    tone == AccountTone::Exhausted,
                    now,
                    cx,
                ));
            }
        }
        parts
    }
}

/// MonoCode `UsageWindowCard`: used on top, the bar, then what is left and
/// when the window resets.
fn usage_window_card(kind: WindowKind, window: &RateLimitWindow, now: i64, colors: &Palette) -> gpui::Div {
    let fg = colors.fg;
    let used = clamp_used_percent(window.used_percent);
    let row = || div().flex().justify_between().gap_3();
    div()
        .px_3()
        .py(px(10.0))
        .child(
            row()
                .items_baseline()
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .child(div().text_color(fg.opacity(0.65)).child(kind.title()))
                .child(div().flex_none().child(format!("{} used", format_usage_percent(used)))),
        )
        .child(usage_bar(used, 6.0, colors).mt_2())
        .child(
            row()
                .items_center()
                .mt(px(6.0))
                .text_size(px(10.0))
                .line_height(px(16.0))
                .text_color(fg.opacity(0.4))
                .child(format!("{} remaining", format_usage_percent(100.0 - used)))
                .child(div().truncate().child(match window.resets_at {
                    Some(resets_at) => format_reset_countdown(resets_at - now),
                    None => format!("{} window", format_window_label(window.window_minutes)),
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn chip_copy_follows_the_snapshot() {
        let mut limits = ProviderRateLimits::default();
        assert_eq!(chip_tooltip(&limits, NOW), "Loading usage…");
        assert_eq!(empty_usage_label(&limits), "—");

        limits = ProviderRateLimits::error("Claude sign-in expired", None, NOW);
        assert_eq!(empty_usage_label(&limits), "expired");
        assert_eq!(chip_tooltip(&limits, NOW), "Claude sign-in expired");

        limits = ProviderRateLimits::error("Claude usage request failed (500)", None, NOW);
        assert_eq!(empty_usage_label(&limits), "—");

        limits = ProviderRateLimits {
            status: RateLimitStatus::Ok,
            session: Some(RateLimitWindow {
                used_percent: 100.0,
                window_minutes: 300,
                resets_at: Some(NOW + 15 * 60_000),
            }),
            weekly: Some(RateLimitWindow {
                used_percent: 11.0,
                window_minutes: 10_080,
                resets_at: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            chip_tooltip(&limits, NOW),
            "100% used · Resets in 15m · 11% used · wk window"
        );
    }
}
