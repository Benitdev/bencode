//! Bottom terminal drawer (MonoCode's terminal dock): the project's PTY
//! under a one-tab header, toggled from the footer or with ⌘J.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{Context, FontWeight, IntoElement, ParentElement, Styled, div, px};

use crate::app::BenCodeApp;

const DRAWER_HEIGHT: gpui::Pixels = px(220.0);

impl BenCodeApp {
    pub fn render_terminal_drawer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let project = std::path::Path::new(&self.current_cwd)
            .file_name()
            .map_or_else(
                || "terminal".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .h(DRAWER_HEIGHT)
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .bg(colors.surface)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Md))
                            .bg(colors.hover)
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                Icon::new(IconName::Terminal)
                                    .size(IconSize::Xs)
                                    .color(colors.fg),
                            )
                            .child(project),
                    )
                    .child(
                        IconButton::new("terminal-drawer-hide", IconName::ChevronDown)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Hide terminal (⌘J)")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_terminal_open(false, cx)),
                            ),
                    ),
            )
            .child(div().flex_1().min_h_0().p_1().child(self.terminal.clone()))
    }
}
