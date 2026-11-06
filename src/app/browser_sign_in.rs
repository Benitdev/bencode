//! A site's HTTP sign-in in a browser tab (`browser/auth.rs`; a dev server
//! behind a password showed a white page): the dialog's state, and its
//! answer handed to the page. The password stays in its field until sent,
//! then the field is emptied; WebKit keeps it for the session, BenCode
//! writes it nowhere. Drawn by `ui/browser_sign_in.rs`.

use bencode_ui::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Subscription, Window};

use super::BenCodeApp;
use crate::browser::page::SignIn;

pub struct BrowserSignIn {
    pub tab: u64,
    pub asked: SignIn,
    pub user: Entity<TextInput>,
    pub password: Entity<TextInput>,
    /// Asked by other tabs while this one was open.
    queued: Vec<(u64, SignIn)>,
    _subscriptions: Vec<Subscription>,
}

impl BenCodeApp {
    /// A tab's site asks for a sign-in: the dialog opens, or the one open
    /// for the same tab asks again (a refused password keeps the name).
    pub(crate) fn ask_browser_sign_in(
        &mut self,
        tab: u64,
        asked: SignIn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = &self.browser.tabs;
        if let Some(open) = self.browser.sign_in.as_mut() {
            if open.tab != tab && tabs.contains_key(&open.tab) {
                open.queued.push((tab, asked));
                return;
            }
            // A closed tab's dialog is this one's now.
            if open.tab != tab {
                open.user.update(cx, |input, cx| input.set_text("", cx));
            }
            (open.tab, open.asked) = (tab, asked);
            open.password.update(cx, |input, cx| input.set_text("", cx));
            let focus = open.password.read(cx).focus_handle(cx);
            crate::ui::composer::focus_later(focus, cx);
            cx.notify();
            return;
        }
        let user = super::text_input(window, cx, "User name");
        let password = cx.new(|cx| TextInput::new(window, cx).masked().placeholder("Password"));
        let to_password = password.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe(&user, move |_: &mut Self, _, event: &InputEvent, cx| {
                if *event == InputEvent::Submit {
                    crate::ui::composer::focus_later(to_password.clone(), cx);
                }
            }),
            cx.subscribe(&password, |this: &mut Self, _, event: &InputEvent, cx| {
                if *event == InputEvent::Submit {
                    this.answer_browser_sign_in(true, cx);
                }
            }),
        ];
        crate::ui::composer::focus_later(user.read(cx).focus_handle(cx), cx);
        self.browser.sign_in = Some(BrowserSignIn {
            tab,
            asked,
            user,
            password,
            queued: Vec::new(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Sends what was typed (`send`), or goes on without signing in.
    pub(crate) fn answer_browser_sign_in(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some(open) = self.browser.sign_in.as_mut() else {
            return;
        };
        let user = open.user.read(cx).text().to_string();
        let password = open.password.read(cx).text().to_string();
        open.password.update(cx, |input, cx| input.set_text("", cx));
        let tab = open.tab;
        // Another tab's, in the same fields, else the dialog closes.
        if open.queued.is_empty() {
            self.browser.sign_in = None;
            self.refocus_prompt(cx);
        } else {
            let (next, asked) = open.queued.remove(0);
            (open.tab, open.asked) = (next, asked);
            open.user.update(cx, |input, cx| input.set_text("", cx));
            crate::ui::composer::focus_later(open.user.read(cx).focus_handle(cx), cx);
        }
        match self.browser_page(tab) {
            Some(page) => page.sign_in(send.then_some((user.as_str(), password.as_str()))),
            None => log::debug!("browser: sign-in answered after its tab closed"),
        }
        cx.notify();
    }
}
