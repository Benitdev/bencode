//! Window status bar, after MonoCode's UsageFooter: the active thread's
//! harness and run state on the left, the terminal drawer toggle on the right.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, TextSize};
use gpui::{Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::app::BenCodeApp;
use crate::harness::HarnessKind;
use crate::ui::HarnessIcon;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let glass = self.glass(cx);
        let theme = cx.theme();
        let colors = &theme.colors;
        let harness = self.selected_session().map(|s| s.harness.clone());
        let running = self.is_agent_running();

        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px_3()
            .py_0p5()
            .bg(glass.fill(colors.bg))
            .border_t_1()
            .border_color(colors.border)
            .text_size(theme.text_size(TextSize::Xs))
            .text_color(colors.fg_muted)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when_some(harness, |el, id| {
                        let label = HarnessKind::from_id(&id)
                            .map_or(id.clone(), |kind| kind.label().to_string());
                        el.child(HarnessIcon::new(&id).size(px(13.0))).child(label)
                    })
                    .when(running, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .ml_2()
                                .text_color(colors.accent)
                                .font_weight(FontWeight::MEDIUM)
                                .child(div().size_1().rounded_full().bg(colors.accent))
                                .child("Agent running"),
                        )
                    }),
            )
            .child(
                Button::new("footer-terminal-toggle", "Terminal")
                    .icon(IconName::Terminal)
                    .size(ControlSize::Sm)
                    .variant(if self.is_terminal_open {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .shortcut("cmd-j")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_terminal_open(!this.is_terminal_open, cx)
                    })),
            )
    }
}
