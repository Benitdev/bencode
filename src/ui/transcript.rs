use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;

impl BenCodeApp {
    pub fn render_transcript_panel(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .pt(px(48.0))
            .bg(colors.bg)
            // Header bar
            .child(self.render_header(session, cx))
            // Scrollable Messages Timeline
            .child(
                on_axis(div().id("transcript-scroll-area"))
                    .flex_1()
                    .p_6()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .max_w(px(800.0))
                            .mx_auto()
                            .children(if let Some(s) = session {
                                if s.blocks.is_empty() {
                                    vec![
                                        div()
                                            .p_6()
                                            .rounded(theme.radius(Radius::Lg))
                                            .bg(colors.surface)
                                            .border_1()
                                            .border_color(colors.border)
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Base))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(colors.fg)
                                                    .child(format!("Thread: {}", s.title)),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Sm))
                                                    .text_color(colors.fg_muted)
                                                    .child(format!("Harness: {} • Working dir: {}", s.harness, s.cwd)),
                                            )
                                            .child(
                                                div()
                                                    .pt_2()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(colors.accent)
                                                    .child("⚡ Ready to take instructions. Type a prompt below to start."),
                                            )
                                            .into_any_element()
                                    ]
                                } else {
                                    s.blocks.iter().map(|block| {
                                        let is_user = block.role == "user";
                                        let text = block.text.as_deref().unwrap_or("");

                                        if is_user {
                                            div()
                                                .flex()
                                                .justify_end()
                                                .w_full()
                                                .child(
                                                    div()
                                                        .max_w(px(640.0))
                                                        .p_4()
                                                        .rounded(theme.radius(Radius::Lg))
                                                        .bg(colors.hover)
                                                        .border_1()
                                                        .border_color(colors.border)
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_color(colors.accent)
                                                                .child("YOU"),
                                                        )
                                                        .child(
                                                            div()
                                                                .pt_1()
                                                                .text_size(theme.text_size(TextSize::Sm))
                                                                .text_color(colors.fg)
                                                                .child(text.to_string()),
                                                        ),
                                                )
                                                .into_any_element()
                                        } else if block.role == "tool" {
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .px_3()
                                                .py_1p5()
                                                .rounded(theme.radius(Radius::Md))
                                                .bg(colors.surface)
                                                .border_1()
                                                .border_color(colors.border)
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(colors.accent)
                                                        .child("TOOL:"),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(colors.fg_muted)
                                                        .child(text.to_string()),
                                                )
                                                .into_any_element()
                                        } else {
                                            div()
                                                .p_4()
                                                .rounded(theme.radius(Radius::Lg))
                                                .bg(colors.surface)
                                                .border_1()
                                                .border_color(colors.border)
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(colors.fg_subtle)
                                                        .child(format!("AGENT ({})", s.harness.to_uppercase())),
                                                )
                                                .child(
                                                    div()
                                                        .pt_2()
                                                        .text_size(theme.text_size(TextSize::Sm))
                                                        .text_color(colors.fg)
                                                        .child(text.to_string()),
                                                )
                                                .into_any_element()
                                        }
                                    }).collect()
                                }
                            } else {
                                vec![
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .h(px(300.0))
                                        .text_size(theme.text_size(TextSize::Base))
                                        .text_color(colors.fg_muted)
                                        .child("Select a thread from the sidebar or click + New Session")
                                        .into_any_element()
                                ]
                            }),
                    ),
            )
            // Composer Dock at bottom
            .child(self.render_composer(session, cx))
    }

    fn render_header(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(54.0))
            .px_6()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Base))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.fg)
                            .child(
                                session
                                    .map(|s| s.title.as_str())
                                    .unwrap_or("BenCode Workspace")
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child(
                                session
                                    .map(|s| s.cwd.as_str())
                                    .unwrap_or("No project open")
                                    .to_string(),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(colors.surface)
                            .border_1()
                            .border_color(colors.border)
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_muted)
                            .child(
                                session
                                    .map(|s| s.model.as_str())
                                    .unwrap_or("Claude 3.7 Sonnet")
                                    .to_string(),
                            ),
                    ),
            )
    }
}
