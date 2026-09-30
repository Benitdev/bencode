//! Bottom status bar: GPU engine indicators, provider quotas, and refresh trigger.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, TextSize};
use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let harnesses = &self.harnesses;

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
            .border_color(colors.border)
            .bg(colors.surface)
            // Left Status: Engine & GPU Speed
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Zap)
                            .size(IconSize::Xs)
                            .color(colors.accent),
                    )
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_muted)
                            .child("BenCode Native"),
                    )
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("•"),
                    )
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("Apple Metal GPU (120 FPS)"),
                    ),
            )
            // Right Provider Quotas & Refresh
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Badge::new(if claude_avail { "Claude: Active" } else { "Claude: Offline" })
                            .tone(if claude_avail { Tone::Warning } else { Tone::Neutral })
                            .dot(),
                    )
                    .child(
                        Badge::new(if agy_avail { "Antigravity: ACP" } else { "Antigravity: Offline" })
                            .tone(if agy_avail { Tone::Info } else { Tone::Neutral })
                            .dot(),
                    )
                    .when(codex_avail, |el| {
                        el.child(Badge::new("Codex: Ready").tone(Tone::Success).dot())
                    })
                    .child(
                        IconButton::new("footer-refresh-btn", IconName::RotateCw)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Refresh database")
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
