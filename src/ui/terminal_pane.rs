//! Embedded PTY terminal with a header naming its working directory.

use ely_gpui_component::data_display::Badge;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, TextSize};
use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn render_terminal_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let branch = self.git_status.branch.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme.colors.bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(theme.colors.fg_muted)
                            .child(Icon::new(IconName::Terminal).size(IconSize::Xs))
                            .child(div().truncate().child(self.current_cwd.clone())),
                    )
                    .when(!branch.is_empty(), |el| el.child(Badge::new(branch))),
            )
            .child(div().flex_1().min_h_0().child(self.terminal.clone()))
    }
}
