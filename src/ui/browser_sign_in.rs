//! The dialog a browser tab's site opens when it asks for a user name and
//! password (BenCode's own; MonoCode has no browser). State and answer:
//! `app/browser_sign_in.rs`.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::forms::{Input, PasswordInput};
use bencode_ui::overlays::Dialog;
use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::tokens::{Ink, Status, Text, Tone as _};

impl BenCodeApp {
    pub fn render_browser_sign_in(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.browser.sign_in.as_ref()?;
        // A tab closed under it: WebKit already had its answer.
        if !self.browser.tabs.contains_key(&open.tab) {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let asked = &open.asked;
        let note = |text: &'static str, color| {
            div()
                .mt_2()
                .text_size(Text::Sm.px())
                .text_color(color)
                .child(text)
        };
        let body = div()
            .child(Input::new(&open.user))
            .child(div().mt_2().child(PasswordInput::new(&open.password)))
            .children(asked.retry.then(|| {
                note(
                    "That user name and password were not accepted.",
                    Status::Failed.color(fg),
                )
            }))
            .children(asked.insecure.then(|| {
                note(
                    "This site does not use a secure connection: the password is sent unencrypted.",
                    Status::Attention.color(fg),
                )
            }))
            .child(note(
                "Kept until BenCode quits, never saved.",
                fg.ink(Ink::Muted),
            ));
        let cancel = app_callback(cx, |this, cx| this.answer_browser_sign_in(false, cx));
        let submit = app_callback(cx, |this, cx| this.answer_browser_sign_in(true, cx));
        let dialog = Dialog::new("browser-sign-in", "Sign in", move |window, cx| {
            cancel(window, cx)
        })
        .detail(format!("{} asks for a user name and password.", asked.host))
        .child(body)
        .action(|close| {
            Button::new("browser-sign-in-cancel", "Cancel")
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, window, cx| close(window, cx))
        })
        .action(move |_| {
            Button::new("browser-sign-in-submit", "Sign in")
                .primary()
                .on_click(move |_, window, cx| submit(window, cx))
        });
        Some(dialog.into_any_element())
    }
}
