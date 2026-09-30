//! Diff viewer view mode: side-by-side or unified diffs with line additions/deletions,
//! syntax highlighting, copy diff, and file change inspection.

use ely_gpui_component::buttons::CopyButton;
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::files::FileIcon;
use ely_gpui_component::git::{DiffStat, GitStatusBadge};
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::lists::GitStatus;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px, uniform_list,
};

use crate::app::BenCodeApp;
use crate::git::{DiffLineKind, GitFileStatus};

fn to_ely_status(status: &GitFileStatus) -> GitStatus {
    match status {
        GitFileStatus::Modified => GitStatus::Modified,
        GitFileStatus::Added => GitStatus::Added,
        GitFileStatus::Deleted => GitStatus::Deleted,
        GitFileStatus::Untracked => GitStatus::Untracked,
        GitFileStatus::Renamed => GitStatus::Renamed,
    }
}

impl BenCodeApp {
    pub fn render_diff_viewer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let files = &self.workspace.changes;
        let selected_path = self.workspace.diff_path.clone().unwrap_or_default();
        let active_file = files.iter().find(|f| f.path == selected_path).cloned();
        let diff_lines_count = self.workspace.diff.len();

        let raw_diff_text: String = if !selected_path.is_empty() {
            self.workspace
                .diff
                .iter()
                .map(|l| match l {
                    DiffLineKind::Header(h) => format!("{h}\n"),
                    DiffLineKind::Addition(a) => format!("+{a}\n"),
                    DiffLineKind::Deletion(d) => format!("-{d}\n"),
                    DiffLineKind::Context(c) => format!(" {c}\n"),
                })
                .collect()
        } else {
            String::new()
        };

        div()
            .flex()
            .flex_1()
            .h_full()
            .bg(colors.bg)
            // Left List: Changed Files
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(280.0))
                    .h_full()
                    .border_r_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .p_3()
                            .border_b_1()
                            .border_color(colors.border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.fg)
                                            .child("CHANGED FILES"),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(cx.theme().radius(Radius::Sm))
                                            .bg(colors.hover)
                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.accent)
                                            .child(SharedString::from(format!("{}", files.len()))),
                                    ),
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
                                        .child(
                                            EmptyState::new(
                                                "no-diffs-empty",
                                                IconName::Sparkles,
                                                "Working tree is clean",
                                            )
                                            .body("No unstaged or staged file changes found"),
                                        )
                                        .into_any_element()
                                ]
                            } else {
                                files.iter().map(|f| {
                                    let is_active = f.path == selected_path;
                                    let path_clone = f.path.clone();
                                    let status = to_ely_status(&f.status);

                                    div()
                                        .id(SharedString::from(format!("diff-file-row-{}", f.path)))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_3()
                                        .py_2()
                                        .cursor_pointer()
                                        .when(is_active, |el| el.bg(colors.active))
                                        .when(!is_active, |el| el.hover(|s| s.bg(colors.hover)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_diff_path(path_clone.clone(), cx);
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .min_w_0()
                                                .child(GitStatusBadge::new(
                                                    format!("badge-diff-{}", f.path),
                                                    status,
                                                ))
                                                .child(FileIcon::file(&f.path).size(IconSize::Xs))
                                                .child(
                                                    div()
                                                        .text_size(cx.theme().text_size(TextSize::Sm))
                                                        .text_color(if is_active { colors.fg } else { colors.fg_muted })
                                                        .truncate()
                                                        .child(f.path.clone()),
                                                ),
                                        )
                                        .child(
                                            DiffStat::new(
                                                f.additions,
                                                f.deletions,
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
                    .bg(colors.bg)
                    // File Header Bar
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_2p5()
                            .border_b_1()
                            .border_color(colors.border)
                            .bg(colors.surface)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(cx.theme().text_size(TextSize::Sm))
                                            .font_family(cx.theme().mono_family.clone())
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.fg)
                                            .child(if selected_path.is_empty() {
                                                "No file selected".to_string()
                                            } else {
                                                selected_path.clone()
                                            }),
                                    )
                                    .when_some(active_file.as_ref(), |parent, f| {
                                        parent.child(
                                            DiffStat::new(f.additions, f.deletions),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when(!selected_path.is_empty(), |el| {
                                        el.child(CopyButton::new("copy-diff-btn", raw_diff_text.clone()))
                                    }),
                            ),
                    )
                    // Diff Lines List
                    .child(
                        if diff_lines_count == 0 {
                            div()
                                .flex_1()
                                .p_8()
                                .text_color(colors.fg_subtle)
                                .child("No diff lines for this file")
                                .into_any_element()
                        } else {
                            uniform_list(
                                "diff-lines-virtual-list",
                                diff_lines_count,
                                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                                    let colors = cx.theme().colors.clone();
                                    let mono_family = cx.theme().mono_family.clone();
                                    let text_xs = cx.theme().text_size(TextSize::Xs);
                                    let radius_sm = cx.theme().radius(Radius::Sm);
                                    let lines = &this.workspace.diff;

                                    range
                                        .filter_map(|idx| {
                                            let line = lines.get(idx)?;
                                            let line_num = idx + 1;
                                            let element = match line {
                                                DiffLineKind::Header(hdr) => div()
                                                    .h(px(22.0))
                                                    .flex()
                                                    .items_center()
                                                    .px_2()
                                                    .rounded(radius_sm)
                                                    .bg(colors.hover)
                                                    .text_color(colors.accent)
                                                    .font_family(mono_family.clone())
                                                    .text_size(text_xs)
                                                    .font_weight(FontWeight::BOLD)
                                                    .child(hdr.clone()),
                                                DiffLineKind::Addition(txt) => div()
                                                    .h(px(22.0))
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px_2()
                                                    .bg(colors.success.opacity(0.1))
                                                    .text_color(colors.success)
                                                    .font_family(mono_family.clone())
                                                    .text_size(text_xs)
                                                    .child(
                                                        div()
                                                            .w(px(28.0))
                                                            .text_color(colors.fg_subtle)
                                                            .child(format!("{line_num}")),
                                                    )
                                                    .child(div().w(px(12.0)).child("+"))
                                                    .child(div().flex_1().child(txt.clone())),
                                                DiffLineKind::Deletion(txt) => div()
                                                    .h(px(22.0))
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px_2()
                                                    .bg(colors.danger.opacity(0.1))
                                                    .text_color(colors.danger)
                                                    .font_family(mono_family.clone())
                                                    .text_size(text_xs)
                                                    .child(
                                                        div()
                                                            .w(px(28.0))
                                                            .text_color(colors.fg_subtle)
                                                            .child(format!("{line_num}")),
                                                    )
                                                    .child(div().w(px(12.0)).child("-"))
                                                    .child(div().flex_1().child(txt.clone())),
                                                DiffLineKind::Context(txt) => div()
                                                    .h(px(22.0))
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .px_2()
                                                    .text_color(colors.fg)
                                                    .font_family(mono_family.clone())
                                                    .text_size(text_xs)
                                                    .child(
                                                        div()
                                                            .w(px(28.0))
                                                            .text_color(colors.fg_subtle)
                                                            .child(format!("{line_num}")),
                                                    )
                                                    .child(div().w(px(12.0)).child(" "))
                                                    .child(div().flex_1().child(txt.clone())),
                                            };
                                            Some(element.into_any_element())
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .flex_1()
                            .h_full()
                            .p_4()
                            .into_any_element()
                        },
                    ),
            )
    }
}
