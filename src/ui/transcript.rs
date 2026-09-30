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
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_transcript_panel(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(MonoTheme::bg_base())
            // 1. Session Sub-Header Bar
            .child(self.render_header(session, cx))
            // 2. Scrollable Messages Timeline
            .child(
                on_axis(div().id("transcript-scroll-area"))
                    .flex_1()
                    .px_6()
                    .py_4()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_5()
                            .max_w(px(840.0))
                            .mx_auto()
                            .children(if let Some(s) = session {
                                if s.blocks.is_empty() {
                                    vec![
                                        // Empty Session Welcome Screen matching MonoCode
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .py_12()
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Lg))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(MonoTheme::fg_primary())
                                                    .child(format!("Thread: {}", s.title)),
                                            )
                                            .child(
                                                div()
                                                    .pt_2()
                                                    .text_size(theme.text_size(TextSize::Sm))
                                                    .text_color(MonoTheme::fg_muted())
                                                    .child(format!("Harness: {} • Path: {}", s.harness, s.cwd)),
                                            )
                                            // Suggestion Chips
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_wrap()
                                                    .gap_2()
                                                    .pt_6()
                                                    .child(
                                                        div()
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("🔍 Review recent git changes"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("⚡ Run tests & fix failures"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("📦 Explain project architecture"),
                                                    ),
                                            )
                                            .into_any_element()
                                    ]
                                } else {
                                    s.blocks.iter().map(|block| {
                                        let is_user = block.role == "user";
                                        let text = block.text.as_deref().unwrap_or("");
                                        let harness_label = s.harness.to_uppercase();

                                        if is_user {
                                            div()
                                                .flex()
                                                .justify_end()
                                                .w_full()
                                                .child(
                                                    div()
                                                        .max_w(px(680.0))
                                                        .p_4()
                                                        .rounded(theme.radius(Radius::Lg))
                                                        .bg(MonoTheme::bg_surface())
                                                        .border_1()
                                                        .border_color(MonoTheme::border_stroke())
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .justify_between()
                                                                .pb_1()
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(MonoTheme::accent())
                                                                        .child("YOU"),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .text_color(MonoTheme::fg_subtle())
                                                                        .child("just now"),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Sm))
                                                                .text_color(MonoTheme::fg_primary())
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
                                                .py_2()
                                                .rounded(theme.radius(Radius::Md))
                                                .bg(MonoTheme::bg_surface())
                                                .border_1()
                                                .border_color(MonoTheme::border_stroke())
                                                .child(
                                                    div()
                                                        .size(px(6.0))
                                                        .rounded_full()
                                                        .bg(MonoTheme::success()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(MonoTheme::skill_gold())
                                                        .child("TOOL:"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .font_family(theme.mono_family.clone())
                                                        .text_color(MonoTheme::fg_muted())
                                                        .child(text.to_string()),
                                                )
                                                .into_any_element()
                                        } else {
                                            // Assistant Turn Card
                                            div()
                                                .p_4()
                                                .rounded(theme.radius(Radius::Lg))
                                                .bg(MonoTheme::bg_surface())
                                                .border_1()
                                                .border_color(MonoTheme::border_stroke())
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .pb_2()
                                                        .child(
                                                            div()
                                                                .size(px(7.0))
                                                                .rounded_full()
                                                                .bg(MonoTheme::accent()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_color(MonoTheme::accent())
                                                                .child(harness_label),
                                                        ),
                                                )
                                                // Assistant Response Text
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Sm))
                                                        .text_color(MonoTheme::fg_primary())
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
                                        .h(px(320.0))
                                        .text_size(theme.text_size(TextSize::Sm))
                                        .text_color(MonoTheme::fg_muted())
                                        .child("Select a thread from the sidebar or click + New Session")
                                        .into_any_element()
                                ]
                            }),
                    ),
            )
            // 3. Composer Dock
            .child(self.render_composer(session, cx))
    }

    fn render_header(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(46.0))
            .px_6()
            .border_b_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Left Title & Cwd
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(MonoTheme::fg_primary())
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
                            .text_color(MonoTheme::fg_subtle())
                            .child("•"),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_muted())
                            .child(
                                session
                                    .map(|s| s.cwd.as_str())
                                    .unwrap_or("No workspace open")
                                    .to_string(),
                            ),
                    ),
            )
            // Right Status Badge
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_muted())
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
