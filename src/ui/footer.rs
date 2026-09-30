use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::harness::HarnessResolver;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let harnesses = HarnessResolver::discover();

        let claude_avail = harnesses.iter().any(|h| h.id == "claude" && h.available);
        let agy_avail = harnesses.iter().any(|h| h.id == "antigravity" && h.available);
        let codex_avail = harnesses.iter().any(|h| h.id == "codex" && h.available);

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .w_full()
            .px_3()
            .border_t_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Left Status: Engine & GPU Speed
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
                            .child(
                                Icon::new(IconName::Zap)
                                    .size(IconSize::Xs)
                                    .color(MonoTheme::accent()),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_muted())
                                    .child("BenCode Native Engine"),
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
                            .text_color(MonoTheme::fg_subtle())
                            .child("Apple Metal GPU (120 FPS)"),
                    ),
            )
            // Right Provider Quotas matching MonoCode
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    // Claude Usage Chip
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(
                                div()
                                    .size(px(6.0))
                                    .rounded_full()
                                    .bg(if claude_avail { MonoTheme::claude_orange() } else { MonoTheme::fg_subtle() }),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::claude_orange())
                                    .child("Claude:"),
                            )
                            .child(
                                div()
                                    .text_color(MonoTheme::fg_primary())
                                    .child(if claude_avail { "84% limit" } else { "Offline" }),
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
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(
                                div()
                                    .size(px(6.0))
                                    .rounded_full()
                                    .bg(if agy_avail { MonoTheme::antigravity_blue() } else { MonoTheme::fg_subtle() }),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::antigravity_blue())
                                    .child("Antigravity:"),
                            )
                            .child(
                                div()
                                    .text_color(MonoTheme::fg_primary())
                                    .child(if agy_avail { "Connected (ACP)" } else { "Offline" }),
                            ),
                    )
                    // Codex Chip
                    .when(codex_avail, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .px_2()
                                .py_0p5()
                                .rounded(theme.radius(Radius::Sm))
                                .bg(MonoTheme::bg_base())
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .text_size(theme.text_size(TextSize::Xs))
                                .child(
                                    div()
                                        .size(px(6.0))
                                        .rounded_full()
                                        .bg(MonoTheme::codex_green()),
                                )
                                .child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(MonoTheme::codex_green())
                                        .child("Codex:"),
                                )
                                .child(
                                    div()
                                        .text_color(MonoTheme::fg_primary())
                                        .child("Ready"),
                                ),
                        )
                    })
                    // Refresh Button
                    .child(
                        div()
                            .id("footer-refresh-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.0))
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .child(
                                Icon::new(IconName::RotateCw)
                                    .size(IconSize::Xs)
                                    .color(MonoTheme::fg_muted()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(sessions) = this.db.list_recent_sessions(50) {
                                    this.sessions = sessions;
                                    cx.notify();
                                }
                            })),
                    ),
            )
    }
}
