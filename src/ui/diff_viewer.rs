//! Changes view: changed files on the left, the selected file's diff on the right.
//! Diff rows are numbered and virtualized; nothing here touches git.

use ely_gpui_component::buttons::{ButtonVariant, CopyButton, IconButton};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::files::FileIcon;
use ely_gpui_component::git::GitStatusBadge;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Palette, TextSize};
use gpui::{
    AnyElement, App, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px, uniform_list,
};

use crate::app::BenCodeApp;
use crate::git::{DiffLineKind, DiffRow, GitFileChange};
use crate::ui::diff_counts::diff_counts;
use crate::ui::git_changes_panel::to_ely_status;

const FILE_LIST_WIDTH: gpui::Pixels = px(280.0);
const ROW_HEIGHT: gpui::Pixels = px(22.0);

const LINE_NUMBER_WIDTH: gpui::Pixels = px(40.0);

/// Sign, text colour and background tint for one diff row.
fn row_style(kind: &DiffLineKind, colors: &Palette) -> (&'static str, Hsla, Option<Hsla>) {
    match kind {
        DiffLineKind::Header(_) => ("", colors.accent, Some(colors.hover)),
        DiffLineKind::Addition(_) => ("+", colors.success, Some(colors.success.opacity(0.1))),
        DiffLineKind::Deletion(_) => ("-", colors.danger, Some(colors.danger.opacity(0.1))),
        DiffLineKind::Context(_) => (" ", colors.fg, None),
    }
}

fn row_text(kind: &DiffLineKind) -> &str {
    match kind {
        DiffLineKind::Header(text)
        | DiffLineKind::Addition(text)
        | DiffLineKind::Deletion(text)
        | DiffLineKind::Context(text) => text,
    }
}

fn line_number(number: Option<u32>) -> String {
    number.map(|n| n.to_string()).unwrap_or_default()
}

fn diff_row(row: &DiffRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (sign, fg, bg) = row_style(&row.kind, &theme.colors);
    let subtle = theme.colors.fg_subtle;
    let gutter = |number: Option<u32>| {
        div()
            .w(LINE_NUMBER_WIDTH)
            .flex_none()
            .text_color(subtle)
            .child(line_number(number))
    };
    div()
        .h(ROW_HEIGHT)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .when_some(bg, |el, bg| el.bg(bg))
        .text_color(fg)
        .font_family(theme.mono_family.clone())
        .text_size(theme.text_size(TextSize::Xs))
        .when(matches!(row.kind, DiffLineKind::Header(_)), |el| {
            el.font_weight(FontWeight::SEMIBOLD)
        })
        .child(gutter(row.old))
        .child(gutter(row.new))
        .child(div().w_3().flex_none().child(sign))
        .child(
            div()
                .flex_1()
                .whitespace_nowrap()
                .child(row_text(&row.kind).to_string()),
        )
        .into_any_element()
}

impl BenCodeApp {
    pub fn render_diff_viewer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().colors.bg)
            .child(self.render_changed_files(cx))
            .child(self.render_diff_pane(cx))
    }

    fn render_changed_files(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let files = &self.workspace.changes;
        let selected = self.workspace.diff_path.as_deref();
        div()
            .id("changed-files")
            .flex()
            .flex_col()
            .flex_none()
            .w(FILE_LIST_WIDTH)
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.surface)
            .child(
                div()
                    .p_3()
                    .text_size(theme.text_size(TextSize::Xs))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.fg_muted)
                    .child(format!("CHANGED FILES · {}", files.len())),
            )
            .when(files.is_empty(), |el| {
                el.child(
                    EmptyState::new("clean-tree", IconName::Sparkles, "Working tree is clean")
                        .body("No staged or unstaged changes."),
                )
            })
            .children(files.iter().map(|file| {
                self.render_changed_file(file, selected == Some(file.path.as_str()), cx)
            }))
    }

    fn render_changed_file(
        &self,
        file: &GitFileChange,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let path = file.path.clone();
        div()
            .id(SharedString::from(format!("changed-{}", file.path)))
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_2()
            .px_3()
            .py_1p5()
            .cursor_pointer()
            .when(active, |el| el.bg(colors.active))
            .hover(|s| s.bg(colors.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select_diff_path(path.clone(), cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(div().flex_none().child(GitStatusBadge::new(
                        SharedString::from(format!("status-{}", file.path)),
                        to_ely_status(&file.status),
                    )))
                    .child(
                        div()
                            .flex_none()
                            .child(FileIcon::file(&file.path).size(IconSize::Xs)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(cx.theme().text_size(TextSize::Sm))
                            .text_color(if active { colors.fg } else { colors.fg_muted })
                            .child(file.path.clone()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .child(diff_counts(file.additions, file.deletions, colors)),
            )
    }

    fn render_diff_pane(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(path) = self.workspace.diff_path.clone() else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("no-diff", IconName::GitPullRequest, "No file selected")
                        .body("Pick a changed file to see its diff."),
                )
                .into_any_element();
        };
        let stat = self
            .workspace
            .changes
            .iter()
            .find(|f| f.path == path)
            .map(|f| (f.additions, f.deletions));
        let rows = self.workspace.diff.len();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .min_w_0()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .font_family(theme.mono_family.clone())
                            .text_size(theme.text_size(TextSize::Sm))
                            .child(div().flex_1().min_w_0().truncate().child(path.clone()))
                            .when_some(stat, |el, (added, removed)| {
                                el.child(div().flex_none().child(diff_counts(
                                    added,
                                    removed,
                                    &theme.colors,
                                )))
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child({
                                let file_rel = path.clone();
                                IconButton::new("diff-open-editor", IconName::ExternalLink)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Open file in external editor")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_in_external_editor(Some(&file_rel), Some(1), cx);
                                    }))
                            })
                            .child(CopyButton::new(
                                "copy-diff",
                                self.workspace.diff_text.clone(),
                            )),
                    ),
            )
            .child(if rows == 0 {
                div()
                    .p_6()
                    .text_color(theme.colors.fg_subtle)
                    .child("No textual changes.")
                    .into_any_element()
            } else {
                uniform_list(
                    "diff-rows",
                    rows,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        this.workspace.diff[range]
                            .iter()
                            .map(|row| diff_row(row, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .py_2()
                .into_any_element()
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_and_line_numbers() {
        assert_eq!(row_text(&DiffLineKind::Addition("x".into())), "x");
        assert_eq!(line_number(None), "");
        assert_eq!(line_number(Some(7)), "7");
    }
}
