//! A provider sign-in's way to finish elsewhere: the link the CLI printed
//! (Open, Copy), the code its page shows pasted back, and Cancel. Drawn
//! under whichever form started the sign-in. State: `app/accounts.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, ClipboardItem, Context, IntoElement, ParentElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::accounts::LoginOrigin;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    /// The running sign-in's link and code field, when `origin` started it.
    pub(crate) fn render_login_prompt(
        &self,
        origin: LoginOrigin,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let login = self
            .accounts
            .login
            .as_ref()
            .filter(|l| l.origin == origin)?;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let cancel = Button::new("login-cancel", "Cancel sign-in")
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx)));
        let Some(prompt) = &login.prompt else {
            return Some(div().mt_2().child(cancel).into_any_element());
        };
        let hint = if prompt.accepts_code {
            "Browser didn't open, or signing in on another device? Open the link there, then \
             paste the code it shows."
        } else {
            "Browser didn't open? Open the link in a browser on this Mac."
        };
        let (open_url, copy_url) = (prompt.url.clone(), prompt.url.clone());
        let named = !self.login_code_input.read(cx).text().trim().is_empty();
        Some(
            div()
                .mt_3()
                .child(
                    div()
                        .text_size(Text::Xs.px())
                        .line_height(px(16.0))
                        .text_color(fg.ink(Ink::Muted))
                        .child(hint),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .mt_2()
                        .child(
                            Button::new("login-open-link", "Open link")
                                .size(ControlSize::Sm)
                                .icon(IconName::ExternalLink)
                                .on_click(move |_, _, cx| cx.open_url(&open_url)),
                        )
                        .child(
                            Button::new("login-copy-link", "Copy link")
                                .size(ControlSize::Sm)
                                .icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_url.clone(),
                                    ))
                                }),
                        ),
                )
                .when(prompt.accepts_code, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(32.0))
                            .mt_2()
                            .pl(px(10.0))
                            .pr_1()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(fg.fill(Fill::Strong))
                            .bg(fg.fill(Fill::Faint))
                            .text_size(Text::Sm.px())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .when(login.code_sent, |el| el.opacity(0.5))
                                    .child(self.login_code_input.clone()),
                            )
                            .child(
                                Button::new(
                                    "login-submit-code",
                                    if login.code_sent {
                                        "Checking…"
                                    } else {
                                        "Submit"
                                    },
                                )
                                .variant(ButtonVariant::Primary)
                                .size(ControlSize::Sm)
                                .loading(login.code_sent)
                                .disabled(login.code_sent || !named)
                                .on_click(cx.listener(|this, _, _, cx| this.submit_login_code(cx))),
                            ),
                    )
                })
                .when_some(login.code_error.clone(), |el, error| {
                    el.child(
                        div()
                            .mt_1()
                            .text_size(Text::Xs.px())
                            .line_height(px(16.0))
                            .text_color(colors.danger)
                            .child(error),
                    )
                })
                .child(div().mt_2().child(cancel))
                .into_any_element(),
        )
    }
}
