use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_composer(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_running = self.is_agent_running;
        let model_name = session
            .map(|s| {
                if s.model.is_empty() {
                    "Claude 3.7 Sonnet"
                } else {
                    s.model.as_str()
                }
            })
            .unwrap_or("Claude 3.7 Sonnet");
        let branch_name = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        div()
            .px_6()
            .pb_5()
            .pt_2()
            .bg(MonoTheme::bg_base())
            .child(
                // Floating Composer Card
                div()
                    .max_w(px(840.0))
                    .mx_auto()
                    .rounded(theme.radius(Radius::Lg))
                    .border_1()
                    .border_color(if is_running {
                        MonoTheme::accent()
                    } else {
                        MonoTheme::border_stroke()
                    })
                    .bg(MonoTheme::bg_surface())
                    .child(
                        // 1. Controls Top Bar (Model Picker, Branch, Context Meter)
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    // Model Picker Chip
                                    .child(
                                        div()
                                            .id("composer-model-picker")
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .child(
                                                div()
                                                    .size(px(6.0))
                                                    .rounded_full()
                                                    .bg(MonoTheme::claude_orange()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(MonoTheme::fg_primary())
                                                    .child(model_name.to_string()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child("▾"),
                                            ),
                                    )
                                    // Branch Pill
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .child(format!("⎇ {}", branch_name)),
                                    ),
                            )
                            // Right Side: Context Meter
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child("14.2k / 200k tokens"),
                            ),
                    )
                    .child(
                        // 2. Input Prompt Text Area + Send Action
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .p_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(46.0))
                                    .px_1()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .text_color(if is_running {
                                                MonoTheme::accent()
                                            } else {
                                                MonoTheme::fg_subtle()
                                            })
                                            .child(if is_running {
                                                "⚡ Claude Code is executing task in background..."
                                            } else {
                                                "Ask Claude Code or type / for skills, @ for files..."
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    // Attachment button (+)
                                    .child(
                                        div()
                                            .id("composer-attach-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.0))
                                            .rounded(theme.radius(Radius::Md))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_base())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .text_color(MonoTheme::fg_muted())
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .child("+"),
                                    )
                                    // Send / Stop button
                                    .child(
                                        div()
                                            .id("composer-send-action-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.0))
                                            .rounded(theme.radius(Radius::Md))
                                            .bg(if is_running {
                                                MonoTheme::danger()
                                            } else {
                                                MonoTheme::accent()
                                            })
                                            .text_color(MonoTheme::on_accent())
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.9))
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .font_weight(FontWeight::BOLD)
                                            .child(if is_running { "■" } else { "↑" })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.handle_send_or_stop(cx);
                                            })),
                                    ),
                            ),
                    ),
            )
    }
}
