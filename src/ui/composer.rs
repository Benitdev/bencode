use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;

impl BenCodeApp {
    pub fn render_composer(
        &mut self,
        _session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let is_running = self.is_agent_running;

        div()
            .p_4()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .max_w(px(800.0))
                    .mx_auto()
                    // Input Bar container
                    .child(
                        div()
                            .flex_1()
                            .h(px(42.0))
                            .px_4()
                            .rounded(theme.radius(Radius::Md))
                            .bg(colors.bg)
                            .border_1()
                            .border_color(colors.border)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Sm))
                                    .text_color(if is_running {
                                        colors.accent
                                    } else {
                                        colors.fg_subtle
                                    })
                                    .child(if is_running {
                                        "⚡ Agent is executing task in background..."
                                    } else {
                                        "Ask agent or enter instruction... (Click Send to dispatch)"
                                    }),
                            ),
                    )
                    // Action button (Send or Stop)
                    .child(
                        div()
                            .id("btn-composer-action")
                            .px_5()
                            .h(px(42.0))
                            .rounded(theme.radius(Radius::Md))
                            .bg(if is_running {
                                colors.danger
                            } else {
                                colors.accent
                            })
                            .text_color(colors.on_accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::SEMIBOLD)
                            .hover(|s| s.opacity(0.9))
                            .child(if is_running { "Stop" } else { "Send" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.handle_send_or_stop(cx);
                            })),
                    ),
            )
    }
}
