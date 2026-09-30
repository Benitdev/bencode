use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::app::{BenCodeApp, SidebarMode};
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_project_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .w(px(52.0))
            .h_full()
            .pt(px(48.0))
            .pb_4()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Top Rail Icons (Workspace / Projects)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    // Project Avatar Icon (Active)
                    .child(
                        div()
                            .id("rail-project-active")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .bg(MonoTheme::accent())
                            .text_color(MonoTheme::on_accent())
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme.text_size(TextSize::Sm))
                            .cursor_pointer()
                            .child("BC"),
                    )
                    // Global Search (Cmd+K / Cmd+Shift+F)
                    .child(
                        div()
                            .id("rail-search-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.is_search_open {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.is_search_open {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::Search).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_search_modal(cx);
                            })),
                    )
                    // Notes & Scratchpad
                    .child(
                        div()
                            .id("rail-notes-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.is_notes_open {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.is_notes_open {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::FileText).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_notes(cx);
                            })),
                    )
                    // Automations / Zap
                    .child(
                        div()
                            .id("rail-automations-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.is_automations_open {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.is_automations_open {
                                MonoTheme::skill_gold()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::Zap).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_automations(cx);
                            })),
                    )
                    // Inbox / PR Reviews
                    .child(
                        div()
                            .id("rail-inbox-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.is_inbox_open {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.is_inbox_open {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::Inbox).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_inbox_modal(cx);
                            })),
                    )
                    // Folder / Files Tree
                    .child(
                        div()
                            .id("rail-folder-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.sidebar_mode == SidebarMode::Files {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.sidebar_mode == SidebarMode::Files {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::Folder).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_mode = if this.sidebar_mode == SidebarMode::Files {
                                    SidebarMode::Sessions
                                } else {
                                    SidebarMode::Files
                                };
                                cx.notify();
                            })),
                    )
                    // Git / Source Control
                    .child(
                        div()
                            .id("rail-git-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .bg(if self.sidebar_mode == SidebarMode::Changes {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.sidebar_mode == SidebarMode::Changes {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .child(Icon::new(IconName::GitBranch).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_mode = if this.sidebar_mode == SidebarMode::Changes {
                                    SidebarMode::Sessions
                                } else {
                                    SidebarMode::Changes
                                };
                                this.refresh_git_status(cx);
                            })),
                    ),
            )
            // Bottom Rail Icons (Settings / Help)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("rail-settings-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(if self.is_settings_open {
                                MonoTheme::accent()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .bg(if self.is_settings_open {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .child(Icon::new(IconName::Settings).size(IconSize::Sm))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_settings_open = true;
                                cx.notify();
                            })),
                    ),
            )
    }
}
