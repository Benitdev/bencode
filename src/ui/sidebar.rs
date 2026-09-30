use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(290.0))
            .h_full()
            .pt(px(48.0))
            .px_3()
            .pb_4()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // App Header Branding
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pb_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Lg))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.fg)
                                            .child("BenCode"),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(colors.accent)
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.on_accent)
                                            .child("GPU"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(colors.fg_subtle)
                                    .child("100% Native Rust • 120 FPS"),
                            ),
                    ),
            )
            // New Session Action Button
            .child(
                div()
                    .px_2()
                    .pb_3()
                    .child(
                        div()
                            .id("btn-new-session")
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .h(px(32.0))
                            .rounded(theme.radius(Radius::Md))
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.bg)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.hover).border_color(colors.border_strong))
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.fg)
                            .child("+ New Session")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // Section Divider
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_2()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.fg_muted)
                            .child("RECENT THREADS"),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child(SharedString::from(format!("{}", self.sessions.len()))),
                    ),
            )
            // Session List
            .child(
                on_axis(div().id("sidebar-sessions-list"))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.sessions.iter().map(|session| {
                        let is_selected = self.selected_session_id.as_deref() == Some(&session.id);
                        let id = session.id.clone();
                        let title = if session.title.trim().is_empty() {
                            "Untitled Thread"
                        } else {
                            &session.title
                        };
                        let harness = session.harness.to_uppercase();
                        let branch = session.branch.clone().unwrap_or_else(|| "main".into());

                        div()
                            .id(SharedString::from(format!("session-item-{}", session.id)))
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2()
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .when(is_selected, |el| el.bg(colors.hover))
                            .hover(|style| style.bg(colors.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_session(id.clone(), cx);
                            }))
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Sm))
                                    .font_weight(if is_selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if is_selected {
                                        colors.fg
                                    } else {
                                        colors.fg_muted
                                    })
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .pt_1()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.accent)
                                            .child(harness),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(colors.fg_subtle)
                                            .child(format!("⎇ {}", branch)),
                                    ),
                            )
                    })),
            )
    }
}
