//! The account pages of the usage popover (MonoCode `UsageProviderChip.tsx`
//! `ProviderAccountPicker`, `AddProviderAccount`, `AccountSwitchRow`,
//! `SwitchSuggestion`; `ProviderSignInPanel.tsx`; `ProviderAccountUsage.tsx`).

use ely_gpui_component::buttons::Button;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Palette};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*,
};

use crate::ui::scale::px;

use super::usage_chip::{ChipAccounts, usage_bar};
use crate::app::BenCodeApp;
use crate::app::accounts::{LoginOrigin, SignIn};
use crate::app::usage::UsageView;
use crate::harness::accounts::ProviderAccount;
use crate::rate_limits::{
    AccountStatus, AccountTone, ProviderRateLimits, RateLimitProvider, RateLimitWindow,
    account_status, clamp_used_percent, format_reset_duration, format_usage_percent,
    format_window_label,
};
use crate::ui::HarnessIcon;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `AccountStatusLabel`: dot + word, e.g. "● Ready" or
/// "● Exhausted back in 31m".
pub(crate) fn account_status_label(status: &AccountStatus, colors: &Palette) -> gpui::Div {
    let fg = colors.fg;
    let (dot, text) = match status.tone {
        AccountTone::Ready => (colors.success, fg.ink(Ink::Secondary)),
        AccountTone::Low => (colors.warning, colors.warning),
        AccountTone::Exhausted => (colors.danger, colors.danger),
        AccountTone::Checking | AccountTone::Unknown => (fg.ink(Ink::Faint), fg.ink(Ink::Faint)),
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
                    .text_color(fg.ink(Ink::Muted))
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
                .text_size(Text::Xs.px())
                .line_height(px(12.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(fg.ink(Ink::Muted))
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
                        .text_color(fg.ink(Ink::Secondary))
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
                .hover(move |s| s.bg(fg.fill(Fill::Strong)))
                .child(
                    Icon::new(IconName::ArrowLeft)
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Muted)),
                ),
        )
        .child(
            div()
                .text_size(Text::Md.px())
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}

fn page_hint(text: &'static str, fg: Hsla) -> gpui::Div {
    div()
        .mt_1()
        .px_1()
        .text_size(Text::Xs.px())
        .line_height(px(16.0))
        .text_color(fg.ink(Ink::Muted))
        .child(text)
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
                self.usage_data.read(cx).cached(provider, &account.id)
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
                .text_size(Text::Sm.px())
                .when(!chip.locked, |el| el.cursor_pointer())
                .map(|el| {
                    if selected {
                        el.bg(colors.accent.opacity(0.10))
                            .border_color(colors.accent.opacity(0.20))
                    } else {
                        el.bg(fg.opacity(0.035))
                            .border_color(fg.fill(Fill::Hover))
                            .text_color(fg.ink(Ink::Body))
                            .hover(move |s| s.bg(fg.opacity(0.075)))
                    }
                })
                .when(!chip.locked, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.close_usage_popover(cx);
                        this.select_provider_account(provider.id(), &account_id, cx);
                    }))
                })
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
                                                .text_size(Text::Xs.px())
                                                .text_color(fg.ink(Ink::Faint))
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
                                                .bg(fg.fill(Fill::Line))
                                                .text_size(Text::Xs.px())
                                                .line_height(px(16.0))
                                                .text_color(fg.ink(Ink::Secondary))
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
                                .text_size(Text::Xs.px())
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
                if chip.locked {
                    "Switch accounts once the Mono's turn finishes."
                } else if chip.mono {
                    "The next message starts a new session on the account you pick, briefed on the recent conversation."
                } else {
                    "Each conversation stays pinned to the account that started it."
                },
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
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.fill(Fill::Line)).text_color(fg))
                    .on_click(cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Add, cx)))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(IconSize::Sm)
                            .color(fg.ink(Ink::Secondary)),
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
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.fill(Fill::Line)).text_color(fg))
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
                    .text_size(Text::Xs.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.ink(Ink::Secondary))
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
                    .border_color(fg.fill(Fill::Strong))
                    .bg(fg.fill(Fill::Faint))
                    .text_size(Text::Sm.px())
                    .when(running, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.account_name_input.clone()),
                    ),
            )
            .child(
                div().mt_3().child(
                    Button::new(
                        "usage-add-submit",
                        if running {
                            "Waiting for browser…"
                        } else {
                            "Sign in and add account"
                        },
                    )
                    .primary()
                    .full_width()
                    .disabled(running || !named)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.add_provider_account(provider, cx)),
                    ),
                ),
            )
            .children(self.render_login_prompt(LoginOrigin::AddAccount, cx))
            .when_some(self.usage.add_error.clone(), |el, error| {
                el.child(
                    div()
                        .mt_2()
                        .text_size(Text::Xs.px())
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
                    .border_color(fg.fill(Fill::Hover))
                    .text_size(Text::Sm.px())
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_usage_view(UsageView::Accounts, cx)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
                    .child(
                        div()
                            .text_size(Text::Xs.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child("Switch"),
                    )
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size(IconSize::Xs)
                            .color(fg.ink(Ink::Faint)),
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
                    .bg(fg.fill(Fill::Hover))
                    .border_1()
                    .border_color(fg.fill(Fill::Selected))
                    .child(HarnessIcon::new(provider.id()).size(px(36.0))),
            )
            .child(
                div()
                    .mt(px(14.0))
                    .text_size(Text::Xl.px())
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
                    .text_size(Text::Sm.px())
                    .line_height(px(16.0))
                    .text_center()
                    .text_color(fg.ink(Ink::Muted))
                    .child(if complete {
                        "You can retry your last message now.".to_string()
                    } else {
                        format!("Sign in to continue using {title}.")
                    }),
            )
            .child(
                div().mt_4().child(match state {
                    SignIn::Running => Button::new("usage-sign-in", "Waiting for browser…")
                        .primary()
                        .disabled(true),
                    SignIn::Complete => Button::new("usage-sign-in", "Signed in")
                        .primary()
                        .icon(IconName::Check)
                        .disabled(true),
                    SignIn::Idle | SignIn::Failed(_) => {
                        Button::new("usage-sign-in", format!("Sign in to {title}"))
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.reconnect_provider(cx)))
                    }
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
                            .text_size(Text::Xs.px())
                            .line_height(px(16.0))
                            .text_center()
                            .text_color(colors.danger)
                            .child(error),
                    )
                },
            )
            .when_some(
                self.render_login_prompt(LoginOrigin::Reconnect, cx),
                |el, prompt| el.child(div().w_full().max_w(px(260.0)).child(prompt)),
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
            .border_color(fg.fill(Fill::Hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(Text::Xs.px())
                            .line_height(px(16.0))
                            .text_color(fg.ink(Ink::Muted))
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
                            .text_size(Text::Sm.px())
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(account.label.clone()),
                            )
                            .child(
                                account_status_label(&account_status(limits, now), colors)
                                    .text_size(Text::Xs.px()),
                            ),
                    ),
            )
            .child(
                Button::new("usage-switch-suggested", "Switch")
                    .primary()
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_usage_popover(cx);
                        this.select_provider_account(&provider, &account_id, cx);
                    })),
            )
            .into_any_element()
    }
}
