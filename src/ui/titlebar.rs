use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};

impl BenCodeApp {
    pub fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let active_tab_id = self.active_tab_id.clone();
        let current_mode = self.active_view_mode;

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(40.0))
            .w_full()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // Left offset for macOS window controls
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(76.0))
                    .h_full(),
            )
            // Middle Tab Bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .h_full()
                    .overflow_x_hidden()
                    .children(self.open_tabs.iter().map(|tab_id| {
                        let is_active = active_tab_id.as_deref() == Some(tab_id.as_str());
                        let session = self.sessions.iter().find(|s| &s.id == tab_id);
                        let title = session
                            .map(|s| {
                                if s.title.trim().is_empty() {
                                    "Untitled"
                                } else {
                                    s.title.as_str()
                                }
                            })
                            .unwrap_or("New Session");
                        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
                        let id = tab_id.clone();
                        let close_id = tab_id.clone();

                        div()
                            .id(SharedString::from(format!("tab-{}", tab_id)))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(32.0))
                            .max_w(px(180.0))
                            .px_3()
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .when(is_active, |el| {
                                el.bg(colors.bg)
                                    .border_1()
                                    .border_color(colors.border)
                            })
                            .when(!is_active, |el| {
                                el.hover(|s| s.bg(colors.hover))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.switch_tab(id.clone(), cx);
                            }))
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.accent)
                                    .child(if harness == "claude" { "C" } else { "A" }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                    .text_color(if is_active { colors.fg } else { colors.fg_muted })
                                    .overflow_hidden()
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("close-tab-{}", tab_id)))
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(colors.fg_subtle)
                                    .hover(|s| s.text_color(colors.fg))
                                    .child("×")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_tab(&close_id, cx);
                                    })),
                            )
                    }))
                    // Add Tab Button
                    .child(
                        div()
                            .id("titlebar-add-tab-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.hover))
                            .text_size(theme.text_size(TextSize::Sm))
                            .text_color(colors.fg_muted)
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // Right View Mode Switcher: Chat | Changes | Terminal
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .child(
                        div()
                            .id("view-mode-chat")
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(px(26.0))
                            .px_2p5()
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .when(current_mode == ViewMode::Chat, |el| {
                                el.bg(colors.hover).text_color(colors.fg)
                            })
                            .when(current_mode != ViewMode::Chat, |el| {
                                el.text_color(colors.fg_muted).hover(|s| s.bg(colors.hover))
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::MEDIUM)
                            .child("💬 Chat")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_view_mode = ViewMode::Chat;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("view-mode-changes")
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(px(26.0))
                            .px_2p5()
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .when(current_mode == ViewMode::Changes, |el| {
                                el.bg(colors.hover).text_color(colors.fg)
                            })
                            .when(current_mode != ViewMode::Changes, |el| {
                                el.text_color(colors.fg_muted).hover(|s| s.bg(colors.hover))
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Δ Changes")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_view_mode = ViewMode::Changes;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("view-mode-terminal")
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(px(26.0))
                            .px_2p5()
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .when(current_mode == ViewMode::Terminal, |el| {
                                el.bg(colors.hover).text_color(colors.fg)
                            })
                            .when(current_mode != ViewMode::Terminal, |el| {
                                el.text_color(colors.fg_muted).hover(|s| s.bg(colors.hover))
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::MEDIUM)
                            .child("⌨ Terminal")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_view_mode = ViewMode::Terminal;
                                cx.notify();
                            })),
                    ),
            )
    }
}
