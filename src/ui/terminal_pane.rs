use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_terminal_pane(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let cwd = session
            .map(|s| s.cwd.as_str())
            .unwrap_or("~");
        let branch = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(MonoTheme::bg_base())
            // Terminal Header Bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(36.0))
                    .px_4()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_surface())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_primary())
                                    .child("TERMINAL:"),
                            )
                            .child(
                                div()
                                    .text_color(MonoTheme::fg_muted())
                                    .child(cwd.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            // Branch Pill
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_base())
                                    .border_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(format!("⎇ {}", branch)),
                            )
                            // Shell badge
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(MonoTheme::accent())
                                    .child("zsh (PTY)"),
                            ),
                    ),
            )
            // Interactive Native PTY Terminal Body
            .child(
                if let Some(term) = &self.terminal {
                    div()
                        .flex_1()
                        .bg(MonoTheme::bg_base())
                        .child(term.clone())
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .p_4()
                        .font_family(theme.mono_family.clone())
                        .text_size(theme.text_size(TextSize::Xs))
                        .bg(MonoTheme::bg_base())
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_color(MonoTheme::fg_subtle())
                                        .child("BenCode Native Terminal Session [Apple Metal accelerated]"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_color(MonoTheme::accent())
                                                .child(format!("{}$", cwd)),
                                        )
                                        .child(
                                            div()
                                                .text_color(MonoTheme::fg_primary())
                                                .child("git status"),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(MonoTheme::fg_muted())
                                        .child("On branch main\nYour branch is up to date with 'origin/main'."),
                                ),
                        )
                        .into_any_element()
                }
            )
    }
}
