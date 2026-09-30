use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::git::get_workspace_changes;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active_tab_id = self.active_tab_id.clone();
        let current_mode = self.active_view_mode;

        // Calculate count of changed files in active workspace
        let changed_files_count = if let Some(session_id) = &active_tab_id {
            if let Some(session) = self.sessions.iter().find(|s| &s.id == session_id) {
                get_workspace_changes(&session.cwd).len()
            } else {
                0
            }
        } else {
            0
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(40.0))
            .w_full()
            .border_b_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Left offset for macOS window controls
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(78.0))
                    .h_full(),
            )
            // Middle Tab Bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .flex_1()
                    .h_full()
                    .px_2()
                    .overflow_x_hidden()
                    .children(self.open_tabs.iter().map(|tab_id| {
                        let is_active = active_tab_id.as_deref() == Some(tab_id.as_str());
                        let session = self.sessions.iter().find(|s| &s.id == tab_id);
                        let title = session
                            .map(|s| {
                                if s.title.trim().is_empty() {
                                    "Untitled Thread"
                                } else {
                                    s.title.as_str()
                                }
                            })
                            .unwrap_or("New Session");
                        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
                        let id = tab_id.clone();
                        let close_id = tab_id.clone();
                        let harness_dot_color = match harness {
                            "claude" => MonoTheme::claude_orange(),
                            "antigravity" => MonoTheme::antigravity_blue(),
                            "codex" => MonoTheme::codex_green(),
                            _ => MonoTheme::accent(),
                        };

                        div()
                            .id(SharedString::from(format!("tab-bar-item-{}", tab_id)))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(30.0))
                            .max_w(px(200.0))
                            .px_3()
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .when(is_active, |el| {
                                el.bg(MonoTheme::bg_base())
                                    .border_1()
                                    .border_color(MonoTheme::border_stroke())
                            })
                            .when(!is_active, |el| {
                                el.hover(|s| s.bg(MonoTheme::bg_hover()))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.switch_tab(id.clone(), cx);
                            }))
                            // Status / Harness Indicator Dot
                            .child(
                                div()
                                    .size(px(6.0))
                                    .rounded_full()
                                    .bg(harness_dot_color),
                            )
                            // Title
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(theme.text_size(TextSize::Xs))
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
                                    .overflow_hidden()
                                    .child(title.to_string()),
                            )
                            // Close Button
                            .child(
                                div()
                                    .id(SharedString::from(format!("tab-close-btn-{}", tab_id)))
                                    .px_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                    .child("×")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_tab(&close_id, cx);
                                    })),
                            )
                    }))
                    // Add Tab (+)
                    .child(
                        div()
                            .id("titlebar-new-tab-plus-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(theme.radius(Radius::Sm))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_size(theme.text_size(TextSize::Sm))
                            .text_color(MonoTheme::fg_muted())
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // Right View Mode Switcher (Pill Group matching MonoCode)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mr_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .p_1()
                            .rounded(theme.radius(Radius::Md))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            // Chat Mode Tab
                            .child(
                                div()
                                    .id("toggle-mode-chat")
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .h(px(24.0))
                                    .px_2p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .when(current_mode == ViewMode::Chat, |el| {
                                        el.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary())
                                    })
                                    .when(current_mode != ViewMode::Chat, |el| {
                                        el.text_color(MonoTheme::fg_muted()).hover(|s| s.text_color(MonoTheme::fg_primary()))
                                    })
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("💬 Chat")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_view_mode = ViewMode::Chat;
                                        cx.notify();
                                    })),
                            )
                            // Changes Mode Tab (with real changed files count)
                            .child(
                                div()
                                    .id("toggle-mode-changes")
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .h(px(24.0))
                                    .px_2p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .when(current_mode == ViewMode::Changes, |el| {
                                        el.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary())
                                    })
                                    .when(current_mode != ViewMode::Changes, |el| {
                                        el.text_color(MonoTheme::fg_muted()).hover(|s| s.text_color(MonoTheme::fg_primary()))
                                    })
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(if changed_files_count > 0 {
                                        format!("Δ Changes ({})", changed_files_count)
                                    } else {
                                        "Δ Changes".to_string()
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_view_mode = ViewMode::Changes;
                                        cx.notify();
                                    })),
                            )
                            // Terminal Mode Tab
                            .child(
                                div()
                                    .id("toggle-mode-terminal")
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .h(px(24.0))
                                    .px_2p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .when(current_mode == ViewMode::Terminal, |el| {
                                        el.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary())
                                    })
                                    .when(current_mode != ViewMode::Terminal, |el| {
                                        el.text_color(MonoTheme::fg_muted()).hover(|s| s.text_color(MonoTheme::fg_primary()))
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
                    // Split Pane Action Button
                    .child(
                        div()
                            .id("titlebar-split-pane-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded(theme.radius(Radius::Sm))
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_muted())
                            .text_size(theme.text_size(TextSize::Xs))
                            .child("◫"),
                    )
                    // Settings Button
                    .child(
                        div()
                            .id("titlebar-settings-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded(theme.radius(Radius::Sm))
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_muted())
                            .text_size(theme.text_size(TextSize::Xs))
                            .child("⚙"),
                    ),
            )
    }
}
