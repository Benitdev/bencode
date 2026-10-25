//! The delete dialogs for threads (MonoCode `DeleteSessionDialog`): one
//! thread, the cards picked in the sidebar, or a title-bar tab's threads.
//! BenCode's own: "Also delete the CLI's session", remembered for next
//! time (`AppSettings::delete_cli_sessions`), removes what the CLI keeps
//! of each session too (`harness::session_purge`).

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::forms::Checkbox;
use bencode_ui::overlays::Dialog;
use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, relative};

use crate::app::BenCodeApp;
use crate::harness::HarnessKind;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::sidebar::SessionDialog;
use crate::ui::tokens::{Ink, Text, Tone as _};

impl BenCodeApp {
    /// The delete dialog, when one is open.
    pub fn render_session_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.session_dialog.clone()?;
        let (id, title, message, ids) = match dialog {
            SessionDialog::Delete(id) => {
                let title = self
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map_or("this session".into(), |s| {
                        crate::app::session_list::display_title(&s.title, &s.harness)
                    });
                (
                    "delete-thread",
                    "Delete session?".to_string(),
                    format!("“{title}” will be permanently deleted."),
                    vec![id],
                )
            }
            SessionDialog::DeleteSelected(ids) => (
                "delete-selected-threads",
                "Delete sessions?".to_string(),
                format!(
                    "Delete {} selected conversations? This can’t be undone.",
                    ids.len()
                ),
                ids,
            ),
            SessionDialog::DeleteMany(ids) => {
                let count = ids.len();
                (
                    "delete-threads",
                    format!("Delete {count} threads?"),
                    format!(
                        "All {count} conversations in this tab and their transcripts will be removed."
                    ),
                    ids,
                )
            }
        };
        let fg = cx.theme().colors.fg;
        let cli = self.settings.delete_cli_sessions;
        let option = self.cli_session_option(&ids, cx);
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(Text::Md.px())
            .line_height(relative(1.5))
            .child(div().text_color(fg.ink(Ink::Secondary)).child(message))
            .children(option);
        let close = app_callback(cx, |this, cx| {
            this.session_dialog = None;
            cx.notify();
        });
        let delete = app_callback(cx, move |this, cx| {
            this.session_dialog = None;
            this.delete_threads(&ids, cli, cx);
            cx.notify();
        });
        let (cancel_id, confirm_id) = (format!("{id}-cancel"), format!("{id}-confirm"));
        let dialog = Dialog::new(id, title, close)
            .child(body)
            .action(move |close| {
                Button::new(cancel_id, "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new(confirm_id, "Delete")
                    .variant(ButtonVariant::Danger)
                    .on_click(move |_, window, cx| delete(window, cx))
            });
        Some(dialog.into_any_element())
    }

    /// The "Also delete the CLI's session" checkbox, when one of `ids` has
    /// a session BenCode can delete, with what it means; else why not.
    fn cli_session_option(&self, ids: &[String], cx: &Context<Self>) -> Option<AnyElement> {
        let fg = cx.theme().colors.fg;
        let note = |text: String| {
            div()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .child(text)
        };
        let deletable: Vec<&str> = ids
            .iter()
            .filter(|id| self.cli_session(id).is_some())
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .map(|s| s.harness.as_str())
            .collect();
        let Some(first) = deletable.first() else {
            // Only a lone thread says why: a list's reasons would be a list.
            let [id] = ids else { return None };
            let thread = self.sessions.iter().find(|s| &s.id == id)?;
            thread
                .provider_session_id
                .as_ref()
                .filter(|s| !s.is_empty())?;
            let cli = label(&thread.harness);
            let why = if crate::remote::route(&thread.cwd).is_some() {
                format!("Its {cli} session stays on the machine it ran on.")
            } else if crate::harness::session_purge::supported(&thread.harness) {
                format!("Its {cli} session stays: another thread resumes it.")
            } else {
                format!("Its {cli} session stays: {cli} keeps sessions in its own database.")
            };
            return Some(note(why).into_any_element());
        };
        let cli = if deletable.iter().all(|h| h == first) {
            label(first)
        } else {
            "CLI"
        };
        let (what, after) = if deletable.len() == 1 {
            (
                format!("Also delete the {cli} session"),
                format!("Its files are removed from disk, so {cli} can no longer resume it."),
            )
        } else {
            (
                format!("Also delete {} {cli} sessions", deletable.len()),
                format!("Their files are removed from disk, so {cli} can no longer resume them."),
            )
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    Checkbox::new("delete-cli-sessions", self.settings.delete_cli_sessions)
                        .label(what)
                        .on_change(app_callback_with(cx, |this, on: bool, cx| {
                            this.set_delete_cli_sessions(on, cx)
                        })),
                )
                .child(note(after))
                .into_any_element(),
        )
    }
}

fn label(harness: &str) -> &str {
    match HarnessKind::from_id(harness) {
        Some(kind) => kind.label(),
        None => harness,
    }
}
