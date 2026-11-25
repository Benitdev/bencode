//! Computer Use settings and the approval for a local agent's turn.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::forms::Switch;
use bencode_ui::overlays::Dialog;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, relative};

use crate::app::BenCodeApp;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::settings_parts::{SettingsGroup, SettingsRow};
use crate::ui::tokens::Text;

impl BenCodeApp {
    pub(crate) fn render_computer_use_settings(&self, cx: &Context<Self>) -> SettingsGroup {
        let toggle = Switch::new("computer-use-enabled", self.computer_use_enabled).on_change(
            app_callback_with(cx, |this, enabled, cx| this.set_computer_use(enabled, cx)),
        );
        let mut group = SettingsGroup::new("Computer Use").row(
            SettingsRow::new("Let local agents request desktop access")
                .description("Approve a turn to let its agent see your displays, open apps, and use the mouse and keyboard. Start a new turn after enabling.")
                .control(toggle),
        );
        if self.computer_use_enabled {
            group = group
                .row(
                    SettingsRow::new("Screen Recording")
                        .description("Allows the agent to see the desktop.")
                        .control(
                            Button::new("computer-use-screen", "Grant access")
                                .size(ControlSize::Sm)
                                .on_click(|_, _, _| {
                                    crate::computer_use::request_screen_permission()
                                }),
                        ),
                )
                .row(
                    SettingsRow::new("Accessibility")
                        .description("Allows the agent to send mouse and keyboard input.")
                        .control(
                            Button::new("computer-use-accessibility", "Grant access")
                                .size(ControlSize::Sm)
                                .on_click(|_, _, _| {
                                    crate::computer_use::request_input_permission()
                                }),
                        ),
                );
        }
        if let Some(label) = self.computer_use.active_label() {
            group = group.row(
                SettingsRow::new("Desktop access active")
                    .description(label.to_owned())
                    .control(
                        Button::new("computer-use-stop", "Stop")
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.stop_computer_use(cx))),
                    ),
            );
        }
        group
    }

    pub(crate) fn render_computer_use_prompt(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (turn, label) = self.computer_use.prompt()?;
        let deny = app_callback(cx, move |this, cx| this.deny_computer_use(turn, cx));
        let allow = app_callback(cx, move |this, cx| this.allow_computer_use(turn, cx));
        Some(
            Dialog::new("computer-use-approval", "Allow desktop access?", deny)
                .child(
                    div()
                        .text_size(Text::Md.px())
                        .line_height(relative(1.5))
                        .text_color(cx.theme().colors.fg_muted)
                        .child(format!("“{label}” wants to see your displays and control apps with the mouse and keyboard. Access ends with this turn.")),
                )
                .action(|close| {
                    Button::new("computer-use-deny", "Deny")
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .action(move |_| {
                    Button::new("computer-use-allow", "Allow this turn")
                        .variant(ButtonVariant::Primary)
                        .on_click(move |_, window, cx| allow(window, cx))
                })
                .into_any_element(),
        )
    }
}
