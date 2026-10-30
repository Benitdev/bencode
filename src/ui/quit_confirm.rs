//! MonoCode's quit-while-busy confirmation (`inFlight.ts`
//! `quitWhileBusyMessage`, `appLifecycle.ts` `askQuitConfirmation`), which
//! BenCode also shows for unsaved automation edits.

use bencode_ui::overlays::ConfirmDialog;
use gpui::{AnyElement, Context, IntoElement};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

/// MonoCode's message, its "resume when you reopen" sentence included:
/// the next launch offers the cut-off turns (`app/in_flight.rs`). `verb`
/// is Quit, or Restart for an update. `unsaved`: automation edits that
/// quitting loses.
fn quit_while_busy_message(count: usize, unsaved: bool, verb: &str) -> String {
    let edits = "Automation changes are not saved and will be lost.";
    match (count, unsaved) {
        (0, _) => format!("{edits} {verb} anyway?"),
        (1, false) => format!(
            "1 chat is still running. {verb} anyway? It can resume when BenCode opens again."
        ),
        (count, false) => format!(
            "{count} chats are still running. {verb} anyway? They can resume when BenCode opens again."
        ),
        (1, true) => format!(
            "1 chat is still running, and it can resume when BenCode opens again. {edits} {verb} anyway?"
        ),
        (count, true) => format!(
            "{count} chats are still running, and they can resume when BenCode opens again. {edits} {verb} anyway?"
        ),
    }
}

impl BenCodeApp {
    pub(crate) fn render_quit_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.quit_confirm_open {
            return None;
        }
        let restart = self.updater.restart_on_quit;
        let verb = if restart { "Restart" } else { "Quit" };
        let unsaved = self.automation_edits_unsaved(cx);
        let message = quit_while_busy_message(self.runs.len(), unsaved, verb);
        let cancel = app_callback(cx, |this, cx| {
            this.quit_confirm_open = false;
            this.updater.restart_on_quit = false;
            cx.notify();
        });
        let quit = app_callback(cx, move |this, cx| {
            this.quit_confirm_open = false;
            if restart {
                this.relaunch_now(cx);
            } else {
                cx.quit();
            }
        });
        Some(
            ConfirmDialog::new("quit-confirm", "BenCode", message, cancel)
                .confirm(verb)
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
        assert_eq!(
            quit_while_busy_message(1, false, "Quit"),
            "1 chat is still running. Quit anyway? It can resume when BenCode opens again."
        );
    }

    #[test]
    fn several_chats_are_plural() {
        assert_eq!(
            quit_while_busy_message(3, false, "Quit"),
            "3 chats are still running. Quit anyway? They can resume when BenCode opens again."
        );
        assert_eq!(
            quit_while_busy_message(2, false, "Restart"),
            "2 chats are still running. Restart anyway? They can resume when BenCode opens again."
        );
    }

    #[test]
    fn unsaved_automation_edits_are_named() {
        assert_eq!(
            quit_while_busy_message(0, true, "Quit"),
            "Automation changes are not saved and will be lost. Quit anyway?"
        );
        assert!(quit_while_busy_message(2, true, "Quit").starts_with("2 chats are still running"));
    }
}
