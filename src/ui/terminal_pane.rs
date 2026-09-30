//! Embedded native terminal pane: PTY integration with branch and cwd status pills.

use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::theme::{ActiveTheme, TextSize};
use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Styled, div,
    px,
};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn render_terminal_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.selected_session();
        let colors = cx.theme().colors.clone();
        let cwd = session
            .map(|s| s.cwd.as_str())
            .unwrap_or("~");
        let branch = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(colors.bg)
            // Terminal Header Bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(36.0))
                    .px_4()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.fg)
                                    .child("TERMINAL:"),
                            )
                            .child(
                                div()
                                    .text_color(colors.fg_muted)
                                    .child(cwd.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            // Branch Pill
                            .child(Badge::new(format!("⎇ {branch}")).tone(Tone::Neutral))
                            // Shell badge
                            .child(Badge::new("zsh (PTY)").tone(Tone::Info)),
                    ),
            )
            // Interactive Native PTY Terminal Body
            .child(
                if let Some(term) = &self.terminal {
                    div()
                        .flex_1()
                        .bg(colors.bg)
                        .child(term.clone())
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .p_4()
                        .font_family(cx.theme().mono_family.clone())
                        .text_size(cx.theme().text_size(TextSize::Xs))
                        .bg(colors.bg)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_color(colors.fg_subtle)
                                        .child("BenCode Native Terminal Session [Apple Metal accelerated]"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_color(colors.accent)
                                                .child(format!("{cwd}$")),
                                        )
                                        .child(
                                            div()
                                                .text_color(colors.fg)
                                                .child("git status"),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(colors.fg_muted)
                                        .child("On branch main\nYour branch is up to date with 'origin/main'."),
                                ),
                        )
                        .into_any_element()
                }
            )
    }
}
