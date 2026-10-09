//! The launch's "Resume interrupted chats?" dialog (shaped after Orca's
//! `NativeChatResumeOnRestartModal`; MonoCode resumes without asking): the
//! turns a quit, an update's restart or a crash cut off, each picked by
//! default, and whether to stop asking. State: `app/in_flight.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::Checkbox;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, relative};

use crate::app::BenCodeApp;
use crate::app::in_flight::CONTINUE_PROMPT;
use crate::harness::HarnessKind;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::scale::px;

impl BenCodeApp {
    pub fn render_resume_interrupted(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.resume.is_open() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let count = self.resume.offers.len();
        let picked = self.resume.picked.len();
        let rows = self.resume.offers.iter().enumerate().map(|(ix, offer)| {
            let harness = HarnessKind::from_id(&offer.harness).map_or(offer.harness.as_str(), |k| k.label());
            let id = offer.session_id.clone();
            Checkbox::new(
                SharedString::from(format!("resume-interrupted-{ix}")),
                self.resume.picked.contains(&offer.session_id),
            )
            .label(format!("{} · {harness}", offer.title))
            .on_change(app_callback_with(cx, move |this, on: bool, cx| {
                this.toggle_resume_pick(&id, on, cx)
            }))
        });
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(13.0))
            .line_height(relative(1.5))
            .child(div().text_color(fg.opacity(0.6)).child(format!(
                "BenCode stopped {} mid-turn. Resuming sends “{CONTINUE_PROMPT}” and asks the agent \
                 to check its last step before carrying on. Your own prompt is not sent again.",
                if count == 1 { "this chat" } else { "these chats" },
            )))
            .child(div().flex().flex_col().gap_2().children(rows))
            .child(
                div().pt_1().child(
                    Checkbox::new("resume-interrupted-auto", self.resume_interrupted_auto)
                        .label("Resume automatically next time")
                        .on_change(app_callback_with(cx, |this, on: bool, cx| {
                            this.set_resume_interrupted_auto(on, cx)
                        })),
                ),
            );
        let close = app_callback(cx, |this, cx| this.dismiss_resume(cx));
        let resume = app_callback(cx, |this, cx| this.resume_picked(cx));
        let title = if count == 1 {
            "Resume an interrupted chat?".to_string()
        } else {
            format!("Resume {count} interrupted chats?")
        };
        let dialog = Dialog::new("resume-interrupted-dialog", title, close)
            .child(body)
            .action(|close| {
                Button::new("resume-interrupted-dismiss", "Not now")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new(
                    "resume-interrupted-confirm",
                    if picked == count { "Resume".to_string() } else { format!("Resume {picked}") },
                )
                .primary()
                .disabled(picked == 0)
                .on_click(move |_, window, cx| resume(window, cx))
            });
        Some(dialog.into_any_element())
    }
}
