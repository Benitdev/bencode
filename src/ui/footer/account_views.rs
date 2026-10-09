//! The account pages of the usage popover (MonoCode `UsageProviderChip.tsx`
//! `ProviderAccountPicker`, `AddProviderAccount`, `AccountSwitchRow`,
//! `SwitchSuggestion`; `ProviderSignInPanel.tsx`; `ProviderAccountUsage.tsx`).

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Palette};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::ui::scale::px;

use super::usage_chip::{ChipAccounts, usage_bar};
use crate::app::BenCodeApp;
use crate::app::accounts::SignIn;
use crate::app::usage::UsageView;
use crate::harness::accounts::ProviderAccount;
use crate::rate_limits::{
    AccountStatus, AccountTone, ProviderRateLimits, RateLimitProvider, RateLimitWindow,
    account_status, clamp_used_percent, format_reset_duration, format_usage_percent,
    format_window_label,
};
use crate::ui::HarnessIcon;
use crate::ui::git_changes_panel::spinning_icon;

/// MonoCode `AccountStatusLabel`: dot + word, e.g. "● Ready" or
/// "● Exhausted back in 31m".
pub(crate) fn account_status_label(status: &AccountStatus, colors: &Palette) -> gpui::Div {
    let fg = colors.fg;
    let (dot, text) = match status.tone {
        AccountTone::Ready => (colors.success, fg.opacity(0.6)),
        AccountTone::Low => (colors.warning, colors.warning),
        AccountTone::Exhausted => (colors.danger, colors.danger),
        AccountTone::Checking | AccountTone::Unknown => (fg.opacity(0.25), fg.opacity(0.35)),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .child(div().flex_none().size(px(6.0)).rounded_full().bg(dot))
        .child(
            div()
                .text_color(text)
                .map(|el| {
                    // Only a long "unknown" reason gives way to its neighbours.
                    if status.tone == AccountTone::Unknown {
                        el.min_w_0().truncate()
                    } else {
                        el.flex_none()
                    }
                })
                .child(status.label.clone()),
        )
        .when_some(status.detail.clone(), |el, detail| {
            el.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(fg.opacity(0.4))
                    .child(detail),
            )
        })
}

/// MonoCode `UsageMeter`: "5h · 2h 24m" over a thin bar, for one window.
pub(crate) fn usage_meter(window: &RateLimitWindow, now: i64, colors: &Palette) -> gpui::Div {
    let fg = colors.fg;
    let used = clamp_used_percent(window.used_percent);
    let full = used >= 100.0 && window.resets_at.is_none_or(|resets_at| resets_at > now);
    let title = format_window_label(window.window_minutes);
    let reset = match window.resets_at {
        None => title.clone(),
        Some(resets_at) if resets_at <= now => "reset due".into(),
        Some(resets_at) => format_reset_duration(resets_at - now),
    };
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap_2()
                .text_size(px(10.0))
                .line_height(px(12.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(fg.opacity(0.4))
                        .child(format!("{title} · {reset}")),
                )
                .child(if full {
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.danger)
                        .child("Full")
                } else {
                    div()
                        .flex_none()
                        .text_color(fg.opacity(0.6))
                        .child(format_usage_percent(used))
                }),
        )
        .child(usage_bar(used, 4.0, colors).mt(px(6.0)))
}

/// A page's title row with its back arrow.
fn page_header(back: gpui::Stateful<gpui::Div>, title: String, fg: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(28.0))
        .child(
            back.flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded(px(6.0))
                .hover(move |s| s.bg(fg.opacity(0.10)))
                .child(
                    Icon::new(IconName::ArrowLeft)
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.45)),
                ),
        )
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}

fn page_hint(text: &'static str, fg: Hsla) -> gpui::Div {
    div()
        .mt_1()
        .px_1()
        .text_size(px(10.0))
        .line_height(px(16.0))
        .text_color(fg.opacity(0.4))
        .child(text)
}

/// MonoCode's solid button: content on the base background.
fn solid_button(id: impl Into<SharedString>, colors: &Palette) -> gpui::Stateful<gpui::Div> {
    let fg = colors.fg;
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(8.0))
        .bg(fg)
        .text_color(colors.bg)
        .font_weight(FontWeight::MEDIUM)
        .hover(move |s| s.bg(fg.opacity(0.85)))
}

impl BenCodeApp {
    /// MonoCode `ProviderAccountPicker`: every account with its status and
    /// meters, then Add account.
    pub(super) fn render_account_picker(
        &self,
        chip: &ChipAccounts,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (provider, now) = (chip.provider, chip.now);
        let back = div()
            .id("usage-accounts-back")
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Usage, cx)));

        let rows = chip.accounts.iter().enumerate().map(|(ix, account)| {
            let selected = account.id == chip.active_id;
            // The footer's own snapshot is the active account's.
            let usage = if selected {
                Some(chip.limits)
            } else {
                self.usage.cached(provider, &account.id)
            };
            let identity = self.accounts.identity(account);
            let meters: Vec<gpui::Div> = usage
                .map(ProviderRateLimits::windows)
                .unwrap_or_default()
                .into_iter()
                .map(|(_, window)| usage_meter(window, now, colors))
                .collect();
            let account_id = account.id.clone();
            div()
                .id(("usage-account", ix))
                .flex()
                .items_center()
                .gap_3()
                .w_full()
                .px(px(10.0))
                .py_2()
                .rounded(px(8.0))
                .border_1()
                .text_size(px(11.0))
                .cursor_pointer()
                .map(|el| {
                    if selected {
                        el.bg(colors.accent.opacity(0.10))
                            .border_color(colors.accent.opacity(0.20))
                    } else {
                        el.bg(fg.opacity(0.035))
                            .border_color(fg.opacity(0.06))
                            .text_color(fg.opacity(0.7))
                            .hover(move |s| s.bg(fg.opacity(0.075)))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_usage_popover(cx);
                    this.select_provider_account(provider.id(), &account_id, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .py(px(2.0))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(6.0))
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_none()
                                        .max_w(px(150.0))
                                        .truncate()
                                        .child(account.label.clone()),
                                )
                                .when_some(
                                    identity.and_then(|identity| identity.subtitle()),
                                    |el, subtitle| {
                                        el.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(10.0))
                                                .text_color(fg.opacity(0.35))
                                                .child(subtitle),
                                        )
                                    },
                                )
                                .when_some(
                                    identity.and_then(|identity| identity.organization_tag()),
                                    |el, tag| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .max_w(px(96.0))
                                                .truncate()
                                                .px_1()
                                                .rounded(px(4.0))
                                                .bg(fg.opacity(0.07))
                                                .text_size(px(9.0))
                                                .line_height(px(16.0))
                                                .text_color(fg.opacity(0.5))
                                                .child(tag.to_string()),
                                        )
                                    },
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .min_w_0()
                                .mt_1()
                                .text_size(px(10.0))
                                .child(
                                    account_status_label(&account_status(usage, now), colors)
                                        // Without meters, a long "unknown" reason truncates.
                                        .when(!meters.is_empty(), |el| el.flex_none()),
                                )
                                .when(!meters.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(10.0))
                                            .children(meters),
                                    )
                                }),
                        ),
                )
                .when(selected, |el| {
                    el.child(
                        Icon::new(IconName::Check)
                            .size(IconSize::Sm)
                            .color(colors.accent),
                    )
                })
        });

        div()
            .child(page_header(
                back,
                format!("{} accounts", provider.title()),
                fg,
            ))
            .child(page_hint(
                "Each conversation stays pinned to the account that started it.",
                fg,
            ))
            .child(div().flex().flex_col().gap_1().mt_2().children(rows))
            .child(
                div()
                    .id("usage-add-account")
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(32.0))
                    .mt_2()
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.55))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.opacity(0.07)).text_color(fg))
                    .on_click(cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Add, cx)))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.55)),
                    )
                    .child("Add account"),
            )
            .child(
                div()
                    .id("usage-manage-accounts")
                    .flex()
                    .items_center()
                    .h(px(32.0))
                    .mt(px(2.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.opacity(0.07)).text_color(fg))
                    .on_click(cx.listener(|this, _, _, cx| this.manage_accounts(cx)))
                    .child("Manage accounts…"),
            )
            .into_any_element()
    }

    /// MonoCode `AddProviderAccount`: a local name, then the provider's
    /// browser sign-in.
    pub(super) fn render_add_account(
        &self,
        provider: RateLimitProvider,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let running = self.usage.adding;
        let named = !self.account_name_input.read(cx).text().trim().is_empty();
        let back = div().id("usage-add-back").when(!running, |el| {
            el.cursor_pointer().on_click(
                cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Accounts, cx)),
            )
        });

        div()
            .child(
                page_header(back, format!("Add {} account", provider.title()), fg)
                    .when(running, |el| el.opacity(0.6)),
            )
            .child(page_hint(
                "Give this account a local name, then finish sign-in in your browser.",
                fg,
            ))
            .child(
                div()
                    .mt_3()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.opacity(0.55))
                    .child("Account name"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(32.0))
                    .mt(px(6.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(fg.opacity(0.10))
                    .bg(fg.opacity(0.04))
                    .text_size(px(11.0))
                    .when(running, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.account_name_input.clone()),
                    ),
            )
            .child(
                solid_button("usage-add-submit", colors)
                    .h(px(32.0))
                    .w_full()
                    .mt_3()
                    .px_3()
                    .text_size(px(11.0))
                    .map(|el| {
                        if running {
                            el.opacity(0.45).child(spinning_icon(
                                "usage-add-spin".into(),
                                IconName::RefreshCw,
                                IconSize::Sm,
                                colors.bg,
                            ))
                        } else if named {
                            el.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.add_provider_account(provider, cx)
                                }))
                        } else {
                            el.opacity(0.45)
                        }
                    })
                    .child(if running {
                        "Waiting for browser…"
                    } else {
                        "Sign in and add account"
                    }),
            )
            .when_some(self.usage.add_error.clone(), |el, error| {
                el.child(
                    div()
                        .mt_2()
                        .text_size(px(10.0))
                        .line_height(px(16.0))
                        .text_color(colors.danger)
                        .child(error),
                )
            })
            .into_any_element()
    }

    /// MonoCode `AccountSwitchRow`: the way to another account from the
    /// sign-in panel.
    pub(super) fn render_account_switch_row(&self, label: &str, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        div()
            .px(px(10.0))
            .pt(px(10.0))
            .child(
                div()
                    .id("usage-switch-row")
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(32.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .bg(fg.opacity(0.045))
                    .border_1()
                    .border_color(fg.opacity(0.06))
                    .text_size(px(11.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.opacity(0.08)))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Accounts, cx)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(fg.opacity(0.4))
                            .child("Switch"),
                    )
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.35)),
                    ),
            )
            .into_any_element()
    }

    /// MonoCode `ProviderSignInPanel`.
    pub(super) fn render_sign_in_panel(
        &self,
        provider: RateLimitProvider,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let title = provider.title();
        let state = &self.usage.sign_in;
        let complete = *state == SignIn::Complete;
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .min_h(px(272.0))
            .px_5()
            .py_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(64.0))
                    .rounded(px(16.0))
                    .bg(fg.opacity(0.06))
                    .border_1()
                    .border_color(fg.opacity(0.08))
                    .child(HarnessIcon::new(provider.id()).size(px(36.0))),
            )
            .child(
                div()
                    .mt(px(14.0))
                    .text_size(px(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .line_height(px(20.0))
                    .child(if complete {
                        format!("Signed in to {title}")
                    } else {
                        "Authentication required".into()
                    }),
            )
            .child(
                div()
                    .mt_1()
                    .max_w(px(224.0))
                    .text_size(px(11.0))
                    .line_height(px(16.0))
                    .text_center()
                    .text_color(fg.opacity(0.45))
                    .child(if complete {
                        "You can retry your last message now.".to_string()
                    } else {
                        format!("Sign in to continue using {title}.")
                    }),
            )
            .child(
                solid_button("usage-sign-in", colors)
                    .h(px(32.0))
                    .mt_4()
                    .px(px(14.0))
                    .text_size(px(12.0))
                    .map(|el| match state {
                        SignIn::Running => el
                            .opacity(0.55)
                            .child(spinning_icon(
                                "usage-sign-in-spin".into(),
                                IconName::RefreshCw,
                                IconSize::Sm,
                                colors.bg,
                            ))
                            .child("Waiting for browser…"),
                        SignIn::Complete => el
                            .opacity(0.55)
                            .child(
                                Icon::new(IconName::Check)
                                    .size(IconSize::Sm)
                                    .color(colors.bg),
                            )
                            .child("Signed in"),
                        SignIn::Idle | SignIn::Failed(_) => el
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.reconnect_provider(cx)))
                            .child(format!("Sign in to {title}")),
                    }),
            )
            .when_some(
                match state {
                    SignIn::Failed(error) => Some(error.clone()),
                    _ => None,
                },
                |el, error| {
                    el.child(
                        div()
                            .mt(px(10.0))
                            .max_w(px(240.0))
                            .text_size(px(10.0))
                            .line_height(px(16.0))
                            .text_center()
                            .text_color(colors.danger)
                            .child(error),
                    )
                },
            )
            .into_any_element()
    }

    /// MonoCode `SwitchSuggestion`: "Out of usage · switch to Work ● Ready".
    pub(super) fn render_switch_suggestion(
        &self,
        account: &ProviderAccount,
        limits: Option<&ProviderRateLimits>,
        exhausted: bool,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (provider, account_id) = (account.provider.clone(), account.id.clone());
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .mt_2()
            .px_3()
            .py(px(10.0))
            .rounded(px(8.0))
            .bg(fg.opacity(0.045))
            .border_1()
            .border_color(fg.opacity(0.06))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(16.0))
                            .text_color(fg.opacity(0.45))
                            .child(if exhausted {
                                "Out of usage · switch to"
                            } else {
                                "Running low · switch to"
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .mt(px(2.0))
                            .text_size(px(11.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(account.label.clone()),
                            )
                            .child(
                                account_status_label(&account_status(limits, now), colors)
                                    .text_size(px(10.0)),
                            ),
                    ),
            )
            .child(
                solid_button("usage-switch-suggested", colors)
                    .h(px(28.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .text_size(px(11.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_usage_popover(cx);
                        this.select_provider_account(&provider, &account_id, cx);
                    }))
                    .child("Switch"),
            )
            .into_any_element()
    }
}
