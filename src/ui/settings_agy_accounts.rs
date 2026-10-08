//! Settings › Providers › Accounts, Antigravity's part (BenCode's own): the
//! saved sign-ins, which one `agy` uses now, Switch, Add account, Rename
//! and Remove. Laid out like the other providers' accounts
//! (`settings_accounts.rs`). State and logic: `app/agy_accounts.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::app::agy_accounts::AgyWork;
use crate::harness::accounts::ProviderAccount;
use crate::harness::agy_accounts::{AgyProfile, PROVIDER};
use crate::ui::app_callback::app_callback;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;
use crate::ui::settings_accounts::{plural_accounts, provider_header};

impl BenCodeApp {
    /// Antigravity's header with Add account, then its accounts.
    pub(crate) fn render_agy_accounts(&self, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let accounts = self.agy_accounts();
        let state = &self.agy_accounts;
        let busy = state.working.is_some();
        let signing_in = state.working == Some(AgyWork::SigningIn);

        let header = provider_header(
            PROVIDER,
            "Antigravity",
            format!("{} · one sign-in for the whole machine", plural_accounts(accounts.len())),
            fg,
            Button::new("accounts-add-antigravity", "Add account")
                .icon(IconName::Plus)
                .variant(ButtonVariant::Outline)
                .size(ControlSize::Sm)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.add_agy_account(cx))),
        );

        let rows = accounts.iter().map(|(account, profile)| {
            if state.renaming.as_deref() == Some(account.id.as_str()) {
                self.render_account_editor(PROVIDER, false, cx)
            } else {
                self.render_agy_account_row(account, profile, cx)
            }
        });
        let signed_out = state.live_email.is_none() && !accounts.is_empty() && !signing_in;

        div()
            .border_b_1()
            .border_color(colors.border)
            .child(header)
            .child(
                div()
                    .pl(px(40.0))
                    .border_t_1()
                    .border_color(fg.opacity(0.05))
                    .children(rows)
                    .when(signing_in, |el| el.child(self.render_agy_sign_in_row(cx)))
                    .when(signed_out, |el| {
                        el.child(
                            div()
                                .px_4()
                                .py_2()
                                .text_size(px(10.0))
                                .text_color(fg.opacity(0.4))
                                .child("Antigravity is signed out. Switch to an account to sign it in."),
                        )
                    }),
            )
            .into_any_element()
    }

    /// An account: its name, whether `agy` uses it and who it is, then
    /// Switch, Rename and Remove.
    fn render_agy_account_row(
        &self,
        account: &ProviderAccount,
        profile: &AgyProfile,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let state = &self.agy_accounts;
        let busy = state.working.is_some();
        let active = state.is_active(profile);
        let switching = state.working == Some(AgyWork::Switching(account.id.clone()));
        let removing = state.working == Some(AgyWork::Removing(account.id.clone()));
        let id = &account.id;
        // The email is the name until the account is given one.
        let subtitle = profile
            .email
            .clone()
            .filter(|email| *email != account.label)
            .unwrap_or_else(|| "Saved sign-in".into());

        let switch_target = account.id.clone();
        let rename_target = account.clone();
        let remove_target = account.id.clone();
        div()
            .id(SharedString::from(format!("agy-account-row-{id}")))
            .flex()
            .items_center()
            .gap_3()
            .h(px(48.0))
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(fg.opacity(0.05))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.85))
                            .child(account.label.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .min_w_0()
                            .mt(px(2.0))
                            .text_size(px(10.0))
                            .when(active, |el| {
                                el.child(div().flex_none().text_color(colors.success).child("In use"))
                            })
                            .child(div().min_w_0().truncate().text_color(fg.opacity(0.3)).child(subtitle)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .when(active, |el| {
                        el.child(
                            div()
                                .mr_1()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg.opacity(0.3))
                                .child("ACTIVE"),
                        )
                    })
                    .when(!active, |el| {
                        el.child(
                            Button::new(SharedString::from(format!("agy-account-switch-{id}")), "Switch")
                                .variant(ButtonVariant::Outline)
                                .size(ControlSize::Sm)
                                .loading(switching)
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.switch_agy_account(&switch_target, cx);
                                })),
                        )
                    })
                    .child(
                        IconButton::new(SharedString::from(format!("agy-account-rename-{id}")), IconName::Pencil)
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("Rename account")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.rename_agy_account(&rename_target, cx);
                            })),
                    )
                    .map(|el| {
                        if removing {
                            return el.child(
                                div().flex().size(px(28.0)).items_center().justify_center().child(spinning_icon(
                                    SharedString::from(format!("agy-account-removing-{id}")),
                                    IconName::RefreshCw,
                                    IconSize::Sm,
                                    fg.opacity(0.5),
                                )),
                            );
                        }
                        // Removing the one in use would only save it again.
                        el.child(
                            IconButton::new(SharedString::from(format!("agy-account-remove-{id}")), IconName::Trash2)
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .tooltip(if active { "Switch to another account to remove this one" } else { "Remove account" })
                                .disabled(busy || active)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_remove_agy_account(&remove_target, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The row a sign-in under way adds: what to do, and Cancel.
    fn render_agy_sign_in_row(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        div()
            .id("agy-account-sign-in")
            .flex()
            .items_center()
            .gap_3()
            .h(px(48.0))
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(fg.opacity(0.05))
            .child(spinning_icon(
                "agy-account-signing-in".into(),
                IconName::RefreshCw,
                IconSize::Sm,
                fg.opacity(0.5),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.6))
                    .child("Waiting for the sign-in in the terminal…"),
            )
            .child(
                Button::new("agy-account-sign-in-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_agy_sign_in(cx))),
            )
            .into_any_element()
    }

    /// The Remove warning, as the other providers' (`render_account_removal`).
    pub fn render_agy_account_removal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.agy_accounts.pending_remove.as_ref()?;
        let (account, _) = self.agy_accounts().into_iter().find(|(account, _)| &account.id == id)?;
        let close = app_callback(cx, |this, cx| this.cancel_remove_agy_account(cx));
        let confirm = app_callback(cx, |this, cx| this.confirm_remove_agy_account(cx));
        let message = format!(
            "Remove “{}”? Its saved sign-in will be deleted. To use it again, add the account and \
             sign in.",
            account.label
        );
        let dialog = Dialog::new("agy-account-remove", "Remove Antigravity account", close)
            .child(
                div()
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .text_color(cx.theme().colors.fg_muted)
                    .child(message),
            )
            .action(|close| {
                Button::new("agy-account-remove-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new("agy-account-remove-confirm", "Remove account")
                    .variant(ButtonVariant::Danger)
                    .on_click(move |_, window, cx| confirm(window, cx))
            });
        Some(dialog.into_any_element())
    }
}
