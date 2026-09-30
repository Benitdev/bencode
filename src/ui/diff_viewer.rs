use ely_gpui_component::{
    layout::on_axis,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::git::{DiffLineKind, GitFileStatus, get_file_diff, get_workspace_changes};
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_diff_viewer(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let cwd = session
            .map(|s| s.cwd.as_str())
            .unwrap_or(".");

        let files = get_workspace_changes(cwd);
        let selected_path = self.selected_diff_path.clone().unwrap_or_else(|| {
            files.first().map(|f| f.path.clone()).unwrap_or_default()
        });

        let active_file = files.iter().find(|f| f.path == selected_path).cloned();
        let diff_lines = if !selected_path.is_empty() {
            get_file_diff(cwd, &selected_path)
        } else {
            Vec::new()
        };

        div()
            .flex()
            .flex_1()
            .h_full()
            .bg(MonoTheme::bg_base())
            // Left List: Changed Files
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(280.0))
                    .h_full()
                    .border_r_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_surface())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .p_3()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::fg_primary())
                                            .child("CHANGED FILES"),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_hover())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::accent())
                                            .child(SharedString::from(format!("{}", files.len()))),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(format!("⎇ {}", session.and_then(|s| s.branch.as_deref()).unwrap_or("main"))),
                            ),
                    )
                    .child(
                        on_axis(div().id("diff-files-scroll"))
                            .flex_1()
                            .overflow_y_scroll()
                            .children(if files.is_empty() {
                                vec![
                                    div()
                                        .p_6()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::Sparkles)
                                                .size(IconSize::Md)
                                                .color(MonoTheme::accent()),
                                        )
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(MonoTheme::fg_muted())
                                                .child("Workspace is clean (no diffs)"),
                                        )
                                        .into_any_element()
                                ]
                            } else {
                                files.iter().map(|f| {
                                    let is_active = f.path == selected_path;
                                    let path_clone = f.path.clone();
                                    let status_badge = f.status.badge_char();
                                    let status_color = match f.status {
                                        GitFileStatus::Modified => MonoTheme::warning(),
                                        GitFileStatus::Added => MonoTheme::success(),
                                        GitFileStatus::Deleted => MonoTheme::danger(),
                                        GitFileStatus::Untracked => MonoTheme::accent(),
                                        GitFileStatus::Renamed => MonoTheme::skill_gold(),
                                    };

                                    div()
                                        .id(SharedString::from(format!("diff-file-row-{}", f.path)))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_3()
                                        .py_2()
                                        .cursor_pointer()
                                        .when(is_active, |el| el.bg(MonoTheme::bg_active()))
                                        .when(!is_active, |el| el.hover(|s| s.bg(MonoTheme::bg_hover())))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.selected_diff_path = Some(path_clone.clone());
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .size(px(16.0))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded(theme.radius(Radius::Sm))
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(status_color)
                                                        .child(status_badge),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Sm))
                                                        .text_color(if is_active {
                                                            MonoTheme::fg_primary()
                                                        } else {
                                                            MonoTheme::fg_muted()
                                                        })
                                                        .overflow_hidden()
                                                        .child(f.path.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .child(
                                                    div()
                                                        .text_color(MonoTheme::success())
                                                        .child(format!("+{}", f.additions)),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(MonoTheme::danger())
                                                        .child(format!("-{}", f.deletions)),
                                                ),
                                        )
                                        .into_any_element()
                                }).collect()
                            }),
                    ),
            )
            // Right Pane: Diff Content
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .bg(MonoTheme::bg_base())
                    // File Header Bar
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_2p5()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_surface())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .font_family(theme.mono_family.clone())
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::fg_primary())
                                            .child(if selected_path.is_empty() {
                                                "No file selected".to_string()
                                            } else {
                                                selected_path.clone()
                                            }),
                                    )
                                    .when(active_file.is_some(), |parent| {
                                        let f = active_file.unwrap();
                                        parent.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .child(
                                                    div()
                                                        .text_color(MonoTheme::success())
                                                        .child(format!("+{}", f.additions)),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(MonoTheme::danger())
                                                        .child(format!("-{}", f.deletions)),
                                                )
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                            .child(Icon::new(IconName::Copy).size(IconSize::Xs))
                                            .child("Copy Diff"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::danger())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::danger_bg()))
                                            .child(
                                                Icon::new(IconName::RotateCcw)
                                                    .size(IconSize::Xs)
                                                    .color(MonoTheme::danger()),
                                            )
                                            .child("Discard"),
                                    ),
                            ),
                    )
                    // Diff Lines List
                    .child(
                        on_axis(div().id("diff-lines-scroll"))
                            .flex_1()
                            .overflow_y_scroll()
                            .p_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .font_family(theme.mono_family.clone())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .children(if diff_lines.is_empty() {
                                        vec![
                                            div()
                                                .p_8()
                                                .text_color(MonoTheme::fg_subtle())
                                                .child("No diff lines for this file")
                                                .into_any_element()
                                        ]
                                    } else {
                                        diff_lines.into_iter().enumerate().map(|(idx, line)| {
                                            match line {
                                                DiffLineKind::Header(hdr) => {
                                                    div()
                                                        .py_1()
                                                        .px_2()
                                                        .my_1()
                                                        .rounded(theme.radius(Radius::Sm))
                                                        .bg(MonoTheme::bg_hover())
                                                        .text_color(MonoTheme::mention_cyan())
                                                        .font_weight(FontWeight::BOLD)
                                                        .child(hdr)
                                                        .into_any_element()
                                                }
                                                DiffLineKind::Addition(txt) => {
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_3()
                                                        .py_0p5()
                                                        .px_2()
                                                        .bg(MonoTheme::success_bg())
                                                        .text_color(MonoTheme::success())
                                                        .child(
                                                            div()
                                                                .w(px(24.0))
                                                                .text_color(MonoTheme::fg_subtle())
                                                                .child(format!("{}", idx + 1)),
                                                        )
                                                        .child(
                                                            div()
                                                                .w(px(12.0))
                                                                .child("+"),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .child(txt),
                                                        )
                                                        .into_any_element()
                                                }
                                                DiffLineKind::Deletion(txt) => {
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_3()
                                                        .py_0p5()
                                                        .px_2()
                                                        .bg(MonoTheme::danger_bg())
                                                        .text_color(MonoTheme::danger())
                                                        .child(
                                                            div()
                                                                .w(px(24.0))
                                                                .text_color(MonoTheme::fg_subtle())
                                                                .child(format!("{}", idx + 1)),
                                                        )
                                                        .child(
                                                            div()
                                                                .w(px(12.0))
                                                                .child("-"),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .child(txt),
                                                        )
                                                        .into_any_element()
                                                }
                                                DiffLineKind::Context(txt) => {
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_3()
                                                        .py_0p5()
                                                        .px_2()
                                                        .text_color(MonoTheme::fg_muted())
                                                        .child(
                                                            div()
                                                                .w(px(24.0))
                                                                .text_color(MonoTheme::fg_subtle())
                                                                .child(format!("{}", idx + 1)),
                                                        )
                                                        .child(
                                                            div()
                                                                .w(px(12.0))
                                                                .child(" "),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .child(txt),
                                                        )
                                                        .into_any_element()
                                                }
                                            }
                                        }).collect()
                                    }),
                            ),
                    ),
            )
    }
}
