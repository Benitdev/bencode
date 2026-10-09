//! MonoCode Settings › Providers › Accounts (`SettingsView.tsx`
//! `ProviderAccountsSettings` and `ProviderAccountEditor`): each provider's
//! accounts with who they are and their usage, Add account, Rename, and
//! Remove behind one confirmation. State and logic: `app/accounts.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::app::accounts::{ACCOUNT_PROVIDERS, Working};
use crate::harness::accounts::ProviderAccount;
use crate::rate_limits::{ProviderRateLimits, RateLimitProvider, account_status};
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;
use crate::ui::footer::{account_status_label, usage_meter};
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;
use crate::ui::settings_parts::{SettingsGroup, SettingsRow, icon_tile};

/// MonoCode `UsageMeter`'s `w-36`.
const METER_WIDTH: f32 = 144.0;

pub(crate) fn plural_accounts(n: usize) -> String {
    format!("{n} {}", if n == 1 { "account" } else { "accounts" })
}

/// A provider's header on the page: its icon, name and `detail`, then
/// `action` (Add account).
pub(crate) fn provider_header(
    harness: &str,
    title: &'static str,
    detail: String,
    fg: gpui::Hsla,
    action: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .px_4()
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_3()
                .child(icon_tile(HarnessIcon::new(harness).size(px(16.0)), fg))
                .child(
                    div()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        )
                        .child(
                            div()
                                .mt(px(2.0))
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.4))
                                .child(detail),
                        ),
                ),
        )
        .child(action)
}

impl BenCodeApp {
    /// MonoCode `ProviderAccountsSettings`.
    pub(crate) fn render_settings_accounts(&self, cx: &Context<Self>) -> SettingsGroup {
        let refreshing = self.usage.refreshing();
        let group = SettingsGroup::new("Accounts")
            .description(
                "Isolated sign-ins for providers that support account profiles. Switch \
                 accounts from the usage control in the footer.",
            )
            .action(
                IconButton::new("accounts-refresh", IconName::RefreshCw)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Refresh usage limits")
                    .disabled(refreshing)
                    .on_click(cx.listener(|this, _, _, cx| this.load_accounts_page(true, cx))),
            );
        let error = self
            .accounts
            .error
            .clone()
            .or_else(|| self.agy_accounts.error.clone());
        let group = match error {
            Some(error) => group.row(SettingsRow::new("Something went wrong").error(error)),
            None => group,
        };
        group
            .rows(
                ACCOUNT_PROVIDERS
                    .iter()
                    .map(|provider| self.render_provider_accounts(*provider, cx)),
            )
            .row(self.render_agy_accounts(cx))
    }

    /// One provider: its header with Add account, then its accounts.
    fn render_provider_accounts(
        &self,
        provider: RateLimitProvider,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let accounts = self.provider_accounts(provider.id());
        let busy = self.accounts.working.is_some();
        let editor = self
            .accounts
            .editor
            .as_ref()
            .filter(|editor| editor.provider == provider);
        let adding = editor.is_some_and(|editor| editor.account_id.is_none());

        let header = provider_header(
            provider.id(),
            provider.title(),
            plural_accounts(accounts.len()),
            fg,
            Button::new(
                SharedString::from(format!("accounts-add-{}", provider.id())),
                "Add account",
            )
            .icon(IconName::Plus)
            .variant(ButtonVariant::Outline)
            .size(ControlSize::Sm)
            .disabled(busy)
            .on_click(
                cx.listener(move |this, _, _, cx| this.open_account_editor(provider, None, cx)),
            ),
        );

        let rows = accounts.iter().map(|account| {
            let editing = editor
                .is_some_and(|editor| editor.account_id.as_deref() == Some(account.id.as_str()));
            if editing {
                self.render_account_editor(provider.id(), false, cx)
            } else {
                self.render_account_row(provider, account, cx)
            }
        });

        div()
            .child(header)
            .child(
                div()
                    .pl(px(40.0))
                    .border_t_1()
                    .border_color(fg.opacity(0.05))
                    .children(rows)
                    .when(adding, |el| {
                        el.child(self.render_account_editor(provider.id(), true, cx))
                    }),
            )
            .into_any_element()
    }

    /// An account: its name and org, status and who it is, usage meters,
    /// then Rename and Remove.
    fn render_account_row(
        &self,
        provider: RateLimitProvider,
        account: &ProviderAccount,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let now = crate::app::now_ms();
        let busy = self.accounts.working.is_some();
        let removing = self.accounts.working
            == Some(Working::Removing(
                account.provider.clone(),
                account.id.clone(),
            ));
        let identity = self.accounts.identity(account);
        let usage = self.usage.cached(provider, &account.id);
        let id = format!("{}-{}", provider.id(), account.id);

        let meters: Vec<gpui::Div> = usage
            .map(ProviderRateLimits::windows)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, window)| {
                div()
                    .flex_none()
                    .w(px(METER_WIDTH))
                    .child(usage_meter(window, now, colors))
            })
            .collect();
        let subtitle = identity
            .and_then(|identity| identity.subtitle())
            .unwrap_or_else(|| {
                if account.is_default() {
                    "Provider CLI profile".into()
                } else {
                    "Isolated profile".into()
                }
            });

        let rename_target = account.clone();
        let remove_target = account.clone();
        div()
            .id(SharedString::from(format!("account-row-{id}")))
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
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .text_color(fg.opacity(0.85))
                                    .child(account.label.clone()),
                            )
                            .when_some(
                                identity.and_then(|identity| identity.organization_tag()),
                                |el, tag| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .max_w(px(128.0))
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
                            .gap(px(10.0))
                            .min_w_0()
                            .mt(px(2.0))
                            .text_size(px(10.0))
                            .child(
                                account_status_label(&account_status(usage, now), colors)
                                    .flex_none(),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(fg.opacity(0.3))
                                    .child(subtitle),
                            ),
                    ),
            )
            .child(div().flex().flex_none().gap_4().children(meters))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(96.0))
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .when(account.is_default(), |el| {
                        el.child(
                            div()
                                .mr_1()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg.opacity(0.3))
                                .child("DEFAULT"),
                        )
                    })
                    .child(
                        IconButton::new(
                            SharedString::from(format!("account-rename-{id}")),
                            IconName::Pencil,
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .tooltip("Rename account")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_account_editor(provider, Some(&rename_target), cx);
                        })),
                    )
                    .when(!account.is_default(), |el| {
                        if removing {
                            return el.child(
                                div()
                                    .flex()
                                    .size(px(28.0))
                                    .items_center()
                                    .justify_center()
                                    .child(spinning_icon(
                                        SharedString::from(format!("account-removing-{id}")),
                                        IconName::RefreshCw,
                                        IconSize::Sm,
                                        fg.opacity(0.5),
                                    )),
                            );
                        }
                        el.child(
                            IconButton::new(
                                SharedString::from(format!("account-remove-{id}")),
                                IconName::Trash2,
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("Remove account")
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.request_remove_account(remove_target.clone(), cx);
                                },
                            )),
                        )
                    }),
            )
            .into_any_element()
    }

    /// MonoCode `ProviderAccountEditor`: the name field with Cancel and
    /// Save, or Sign in and add for a new account.
    pub(crate) fn render_account_editor(
        &self,
        provider: &str,
        adding: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let working = self.accounts.working.is_some();
        let named = !self.account_editor_input.read(cx).text().trim().is_empty();
        let submit = match (adding, working) {
            (true, true) => "Waiting for browser…",
            (true, false) => "Sign in and add",
            (false, _) => "Save",
        };
        div()
            .id(SharedString::from(format!("account-editor-{provider}")))
            .flex()
            .items_center()
            .h(px(48.0))
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(fg.opacity(0.05))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .h(px(32.0))
                    .pl(px(10.0))
                    .pr_1()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(fg.opacity(0.10))
                    .bg(fg.opacity(0.04))
                    .text_size(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(working, |el| el.opacity(0.5))
                            .child(self.account_editor_input.clone()),
                    )
                    .child(
                        Button::new("account-editor-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .disabled(working)
                            .on_click(cx.listener(|this, _, _, cx| this.close_account_editor(cx))),
                    )
                    .child(
                        Button::new("account-editor-submit", submit)
                            .variant(ButtonVariant::Primary)
                            .size(ControlSize::Sm)
                            .loading(working)
                            .disabled(working || !named)
                            .on_click(cx.listener(|this, _, _, cx| this.submit_account_editor(cx))),
                    ),
            )
            .into_any_element()
    }

    /// MonoCode's Remove warning. A `Dialog` rather than `ConfirmDialog`,
    /// whose message does not wrap.
    pub fn render_account_removal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let account = self.accounts.pending_remove.as_ref()?;
        let provider = RateLimitProvider::from_harness(&account.provider)?;
        let close = app_callback(cx, |this, cx| this.cancel_remove_account(cx));
        let confirm = app_callback(cx, |this, cx| this.confirm_remove_account(cx));
        let message = format!(
            "Remove “{}”? Its stored credentials will be deleted and any running turns for this \
             account will stop. Existing conversations stay in history, but cannot continue until \
             you switch accounts.",
            account.label
        );
        let dialog = Dialog::new(
            "account-remove",
            format!("Remove {} account", provider.title()),
            close,
        )
        .child(
            div()
                .text_size(px(13.0))
                .line_height(relative(1.5))
                .text_color(cx.theme().colors.fg_muted)
                .child(message),
        )
        .action(|close| {
            Button::new("account-remove-cancel", "Cancel")
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, window, cx| close(window, cx))
        })
        .action(move |_| {
            Button::new("account-remove-confirm", "Remove account")
                .variant(ButtonVariant::Danger)
                .on_click(move |_, window, cx| confirm(window, cx))
        });
        Some(dialog.into_any_element())
    }
}
