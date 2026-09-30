use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_id = self.selected_session_id.clone();
        let search_query = self.search_query.to_lowercase();

        // Filter sessions by search query
        let filtered_sessions: Vec<_> = self.sessions
            .iter()
            .filter(|s| {
                if search_query.is_empty() {
                    true
                } else {
                    s.title.to_lowercase().contains(&search_query)
                        || s.cwd.to_lowercase().contains(&search_query)
                        || s.branch.as_deref().unwrap_or("").to_lowercase().contains(&search_query)
                }
            })
            .cloned()
            .collect();

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(270.0))
            .h_full()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // 1. Search Bar
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(32.0))
                            .px_3()
                            .rounded(theme.radius(Radius::Md))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_muted())
                                    .child("🔍"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.search_input.clone()),
                            ),
                    ),
            )
            // 2. Action Bar (New Session & Workspace Info)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_muted())
                                    .child("RECENT THREADS"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(SharedString::from(format!("{}", filtered_sessions.len()))),
                            ),
                    )
                    .child(
                        div()
                            .id("sidebar-new-session-pill")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::accent())
                            .text_color(MonoTheme::on_accent())
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("+ New")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // 3. Thread List Items
            .child(
                on_axis(div().id("sidebar-threads-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(filtered_sessions.into_iter().map(|session| {
                        let is_active = selected_id.as_deref() == Some(&session.id);
                        let id = session.id.clone();
                        let title = if session.title.trim().is_empty() {
                            "Untitled Session"
                        } else {
                            &session.title
                        };
                        let harness = session.harness.to_lowercase();
                        let branch = session.branch.clone().unwrap_or_else(|| "main".into());
                        let (harness_color, harness_label) = match harness.as_str() {
                            "claude" => (MonoTheme::claude_orange(), "Claude"),
                            "antigravity" => (MonoTheme::antigravity_blue(), "Agy"),
                            "codex" => (MonoTheme::codex_green(), "Codex"),
                            _ => (MonoTheme::accent(), "Agent"),
                        };

                        div()
                            .id(SharedString::from(format!("session-row-{}", session.id)))
                            .relative()
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2p5()
                            .mx_1p5()
                            .my_0p5()
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .when(is_active, |el| el.bg(MonoTheme::bg_active()))
                            .when(!is_active, |el| el.hover(|s| s.bg(MonoTheme::bg_hover())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_session(id.clone(), cx);
                            }))
                            // Left vertical bar on active
                            .child(
                                if is_active {
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(6.0))
                                        .bottom(px(6.0))
                                        .w(px(3.0))
                                        .rounded(theme.radius(Radius::Sm))
                                        .bg(MonoTheme::accent())
                                } else {
                                    div()
                                },
                            )
                            // Title Row (with pinned badge if applicable)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .font_weight(if is_active {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .text_color(if is_active {
                                                MonoTheme::fg_primary()
                                            } else {
                                                MonoTheme::fg_muted()
                                            })
                                            .child(title.to_string()),
                                    )
                                    .when(session.pinned, |el| {
                                        el.child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .text_color(MonoTheme::skill_gold())
                                                .child("📌"),
                                        )
                                    }),
                            )
                            // Meta Row: Harness Badge + Branch Pill
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .pt_1p5()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .size(px(6.0))
                                                    .rounded_full()
                                                    .bg(harness_color),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(harness_color)
                                                    .child(harness_label),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child(format!("⎇ {}", branch)),
                                    ),
                            )
                    })),
            )
    }
}
