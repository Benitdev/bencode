//! MonoCode's quit-while-busy confirmation (`inFlight.ts`
//! `quitWhileBusyMessage`, `appLifecycle.ts` `askQuitConfirmation`).

use ely_gpui_component::overlays::ConfirmDialog;
use gpui::{AnyElement, Context, IntoElement};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

/// MonoCode's message without its "resume when you reopen" sentence:
/// BenCode does not resume interrupted turns.
fn quit_while_busy_message(count: usize) -> String {
    if count == 1 {
        "1 chat is still running. Quit anyway?".to_string()
    } else {
        format!("{count} chats are still running. Quit anyway?")
    }
}

impl BenCodeApp {
    pub(crate) fn render_quit_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.quit_confirm_open {
            return None;
        }
        let message = quit_while_busy_message(self.runs.len());
        let cancel = app_callback(cx, |this, cx| {
            this.quit_confirm_open = false;
            cx.notify();
        });
        let quit = app_callback(cx, |this, cx| {
            this.quit_confirm_open = false;
            cx.quit();
        });
        Some(
            ConfirmDialog::new("quit-confirm", "BenCode", message, cancel)
                .confirm("Quit")
                .destructive()
                .on_confirm(quit)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_chat_is_singular() {
        assert_eq!(quit_while_busy_message(1), "1 chat is still running. Quit anyway?");
    }

    #[test]
    fn several_chats_are_plural() {
        assert_eq!(quit_while_busy_message(3), "3 chats are still running. Quit anyway?");
    }
}
