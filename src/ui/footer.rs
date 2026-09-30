use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Styled, div, px,
};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .w_full()
            .px_3()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // Left Status
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_muted)
                            .child("⚡ BenCode Native Engine"),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("•"),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("Apple Metal GPU (120 FPS)"),
                    ),
            )
            // Right Provider Quotas
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    // Claude Usage Chip
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(colors.hover)
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.accent)
                                    .child("Claude:"),
                            )
                            .child(
                                div()
                                    .text_color(colors.fg)
                                    .child("84% limit"),
                            ),
                    )
                    // Antigravity Chip
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(colors.hover)
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.success)
                                    .child("Antigravity:"),
                            )
                            .child(
                                div()
                                    .text_color(colors.fg)
                                    .child("Connected (ACP)"),
                            ),
                    ),
            )
    }
}
