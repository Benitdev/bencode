//! Leftmost project rail: window controls, search, inbox, notes, automations,
//! and project list with git diff stats. 100% faithful to MonoCode's ProjectRail.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    WindowControlArea, div, prelude::*, px, rgb,
};

use crate::app::BenCodeApp;

fn format_diff_number(n: usize) -> String {
    if n >= 1000 {
        let thousands = n / 1000;
        let rem = n % 1000;
        format!("{thousands},{rem:03}")
    } else {
        n.to_string()
    }
}

const PROJECT_COLORS: [u32; 6] = [
    0xec4899, // pink
    0xf97316, // orange
    0xd946ef, // magenta
    0x3b82f6, // blue
    0x10b981, // emerald
    0x8b5cf6, // purple
];

impl BenCodeApp {
    pub fn render_project_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let current_cwd = self.current_cwd.clone();

        // Compute real git diff stats for active repo
        let staged_add: usize = self.git_status.staged.iter().map(|f| f.additions).sum();
        let unstaged_add: usize = self.git_status.unstaged.iter().map(|f| f.additions).sum();
        let staged_del: usize = self.git_status.staged.iter().map(|f| f.deletions).sum();
        let unstaged_del: usize = self.git_status.unstaged.iter().map(|f| f.deletions).sum();
        let total_add = staged_add + unstaged_add;
        let total_del = staged_del + unstaged_del;

        div()
            .flex()
            .flex_col()
            .justify_between()
            .w(px(200.0))
            .h_full()
            .bg(colors.bg)
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    // Top Bar with macOS Window Controls & Nav buttons
                    .child(
                        div()
                            .window_control_area(WindowControlArea::Drag)
                            .flex()
                            .items_center()
                            .h(px(40.0))
                            .px_2()
                            .child(div().w(px(78.0)).h_full())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_0p5()
                                    .child(
                                        IconButton::new("rail-nav-back", IconName::ChevronLeft)
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .tooltip("Back"),
                                    )
                                    .child(
                                        IconButton::new("rail-nav-forward", IconName::ChevronRight)
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .tooltip("Forward"),
                                    )
                                    .child(
                                        IconButton::new("rail-toggle-sidebar", IconName::PanelLeft)
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .tooltip("Toggle Sidebar (⌘B)")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.is_sidebar_open = !this.is_sidebar_open;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    // Quick Tools / Nav items
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .px_2()
                            .pt_1()
                            // Search (⌘K)
                            .child(
                                div()
                                    .id("rail-search-item")
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(32.0))
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_search_modal(cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.fg)
                                            .child(
                                                Icon::new(IconName::Search)
                                                    .size(IconSize::Sm)
                                                    .color(colors.fg_muted),
                                            )
                                            .child("Search"),
                                    )
                                    .child(
                                        div()
                                            .px_1()
                                            .py_0p5()
                                            .rounded_sm()
                                            .bg(colors.surface)
                                            .border_1()
                                            .border_color(colors.border)
                                            .text_size(px(10.0))
                                            .text_color(colors.fg_muted)
                                            .child("⌘K"),
                                    ),
                            )
                            // Inbox
                            .child(
                                div()
                                    .id("rail-inbox-item")
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(32.0))
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_inbox_modal(cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.fg)
                                            .child(
                                                Icon::new(IconName::Inbox)
                                                    .size(IconSize::Sm)
                                                    .color(colors.fg_muted),
                                            )
                                            .child("Inbox"),
                                    )
                                    .child(div().size(px(8.0)).rounded_full().bg(rgb(0x388bfd))),
                            )
                            // Notes
                            .child(
                                div()
                                    .id("rail-notes-item")
                                    .flex()
                                    .items_center()
                                    .h(px(32.0))
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_notes(cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.fg)
                                            .child(
                                                Icon::new(IconName::FileText)
                                                    .size(IconSize::Sm)
                                                    .color(colors.fg_muted),
                                            )
                                            .child("Notes"),
                                    ),
                            )
                            // Automations
                            .child(
                                div()
                                    .id("rail-automations-item")
                                    .flex()
                                    .items_center()
                                    .h(px(32.0))
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_automations(cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.fg)
                                            .child(
                                                Icon::new(IconName::Zap)
                                                    .size(IconSize::Sm)
                                                    .color(colors.fg_muted),
                                            )
                                            .child("Automations"),
                                    ),
                            ),
                    )
                    // Projects Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .mt_4()
                            .px_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(24.0))
                                    .px_2()
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(colors.fg_muted)
                                            .child("Projects"),
                                    )
                                    .child(
                                        IconButton::new("rail-add-project", IconName::Plus)
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .tooltip("Open Project"),
                                    ),
                            )
                            .child(div().flex().flex_col().gap_0p5().children(
                                self.recent_projects.iter().enumerate().map(|(ix, path)| {
                                    let name = std::path::Path::new(path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(path)
                                        .to_string();
                                    let is_selected = path == &current_cwd;
                                    let dot_color = rgb(PROJECT_COLORS[ix % PROJECT_COLORS.len()]);
                                    let path_clone = path.clone();

                                    let has_diff = is_selected && (total_add > 0 || total_del > 0);

                                    div()
                                        .id(SharedString::from(format!("rail-project-{ix}")))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .h(px(32.0))
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .px_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .when(is_selected, |el| {
                                            el.bg(gpui::rgba(0x388bfd38))
                                                .border_1()
                                                .border_color(gpui::rgba(0x388bfd59))
                                        })
                                        .when(!is_selected, |el| el.hover(|s| s.bg(colors.hover)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.switch_project(path_clone.clone(), cx);
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_1()
                                                .items_center()
                                                .gap_2()
                                                .min_w_0()
                                                .child(if is_selected {
                                                    Icon::new(IconName::Globe)
                                                        .size(IconSize::Sm)
                                                        .color(rgb(0x388bfd))
                                                        .into_any_element()
                                                } else {
                                                    div()
                                                        .flex_none()
                                                        .size(px(8.0))
                                                        .rounded_full()
                                                        .bg(dot_color)
                                                        .into_any_element()
                                                })
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(px(13.0))
                                                        .font_weight(if is_selected {
                                                            FontWeight::SEMIBOLD
                                                        } else {
                                                            FontWeight::NORMAL
                                                        })
                                                        .text_color(if is_selected {
                                                            rgb(0xffffff)
                                                        } else {
                                                            gpui::rgba(0xffffffa6)
                                                        })
                                                        .child(name),
                                                ),
                                        )
                                        .when(has_diff, |el| {
                                            el.child(
                                                div()
                                                    .flex_none()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .text_size(px(11.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .when(total_add > 0, |el| {
                                                        el.child(
                                                            div().text_color(rgb(0x3fb950)).child(
                                                                format!(
                                                                    "+{}",
                                                                    format_diff_number(total_add)
                                                                ),
                                                            ),
                                                        )
                                                    })
                                                    .when(total_del > 0, |el| {
                                                        el.child(
                                                            div().text_color(rgb(0xf85149)).child(
                                                                format!(
                                                                    "-{}",
                                                                    format_diff_number(total_del)
                                                                ),
                                                            ),
                                                        )
                                                    }),
                                            )
                                        })
                                }),
                            )),
                    ),
            )
            // Bottom Settings & Collapse
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(40.0))
                    .px_2()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .id("rail-settings-btn")
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(32.0))
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_settings_open = true;
                                cx.notify();
                            }))
                            .child(
                                Icon::new(IconName::Settings)
                                    .size(IconSize::Sm)
                                    .color(colors.fg_muted),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors.fg_muted)
                                    .child("Settings"),
                            ),
                    )
                    .child(
                        IconButton::new("rail-collapse", IconName::PanelLeft)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Collapse Sidebar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_sidebar_open = !this.is_sidebar_open;
                                cx.notify();
                            })),
                    ),
            )
    }
}
