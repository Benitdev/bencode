//! MonoCode `ProjectSearch`: Search in files, shown in the Explorer tab in
//! place of the tree. A back row, the query with Match case / Whole word /
//! Regex, include and exclude globs, a status line, then the matches
//! grouped by file; a click opens the match in the editor.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FontWeight, HighlightStyle, Hsla, IntoElement, SharedString,
    StyledText, div, relative,
};

use super::{ENTRY_ICON, GLYPH, resolve_entry_icon};
use crate::app::BenCodeApp;
use crate::app::project_search::SearchToggle;
use crate::project_search::SearchMatch;
use crate::ui::scale::px;
use crate::ui::virtual_rows;

/// A file's two lines: its name, then its folder path.
const FILE_ROW: f32 = 44.0;
const MATCH_ROW: f32 = 22.0;

enum Row<'a> {
    File {
        name: &'a str,
        relative: &'a str,
        count: usize,
    },
    Match {
        index: usize,
        hit: &'a SearchMatch,
    },
}

impl Row<'_> {
    fn height(&self) -> Option<f32> {
        Some(match self {
            Row::File { .. } => FILE_ROW,
            Row::Match { .. } => MATCH_ROW,
        })
    }
}

/// MonoCode `groupMatches`: a file row, then its matches, in result order.
fn rows(matches: &[SearchMatch]) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let mut start = 0;
    while start < matches.len() {
        let relative = matches[start].relative.as_str();
        let end = start
            + matches[start..]
                .iter()
                .take_while(|m| m.relative == relative)
                .count();
        // git grep lists a file's lines together; a stray later line of the
        // same file still lands under its own header.
        rows.push(Row::File {
            name: relative.rsplit('/').next().unwrap_or(relative),
            relative,
            count: end - start,
        });
        rows.extend((start..end).map(|index| Row::Match {
            index,
            hit: &matches[index],
        }));
        start = end;
    }
    rows
}

/// "3 results in 2 files", MonoCode's status copy.
fn summary(matches: &[SearchMatch], truncated: bool) -> String {
    if matches.is_empty() {
        return "No results".into();
    }
    let files = rows(matches)
        .iter()
        .filter(|r| matches!(r, Row::File { .. }))
        .count();
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    format!(
        "{} in {}{}",
        plural(matches.len(), "result"),
        plural(files, "file"),
        if truncated { " (limited)" } else { "" }
    )
}

impl BenCodeApp {
    pub fn render_project_search(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let root = self.workspace_cwd();
        if matches!(root.trim(), "" | "~") {
            return div()
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.5))
                .child("No project folder")
                .into_any_element();
        }
        let stroke = fg.opacity(0.07);
        let search = &self.project_search;
        div()
            .id("project-search")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px(px(6.0))
                    .py_1()
                    .border_b_1()
                    .border_color(stroke)
                    .child(
                        div()
                            .id("project-search-back")
                            .group("project-search-back")
                            .size(px(28.0))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .hover(move |s| s.bg(fg.opacity(0.10)))
                            .tooltip(Tooltip::text("Back to files"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_project_search(cx);
                            }))
                            .child(
                                Icon::new(IconName::ChevronLeft)
                                    .size(IconSize::Md)
                                    .color(fg.opacity(0.5))
                                    .group_hover_color("project-search-back", fg),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.55))
                            .child("Search in files"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(stroke)
                    .child(
                        field(&search.query_input, 12.0, fg)
                            .pr_1()
                            .child(self.render_project_search_toggle(SearchToggle::MatchCase, cx))
                            .child(self.render_project_search_toggle(SearchToggle::WholeWord, cx))
                            .child(self.render_project_search_toggle(SearchToggle::Regex, cx)),
                    )
                    .child(field(&search.include_input, 11.0, fg))
                    .child(field(&search.exclude_input, 11.0, fg)),
            )
            .child(self.render_project_search_status(cx))
            .child(self.render_project_search_results(cx))
            .into_any_element()
    }

    fn render_project_search_toggle(
        &self,
        toggle: SearchToggle,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let on = self.project_search.is_on(toggle);
        let (id, icon, tip) = match toggle {
            SearchToggle::MatchCase => ("search-match-case", IconName::CaseSensitive, "Match case"),
            SearchToggle::WholeWord => {
                ("search-whole-word", IconName::WholeWord, "Match whole word")
            }
            SearchToggle::Regex => ("search-regex", IconName::Regex, "Use regular expression"),
        };
        div()
            .id(id)
            .group(id)
            .size(px(24.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .when(on, |el| el.bg(colors.active))
            .when(!on, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
            .tooltip(Tooltip::text(tip))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_project_search_option(toggle, cx)),
            )
            .child(
                Icon::new(icon)
                    .size(GLYPH)
                    .color(if on { fg } else { fg.opacity(0.4) })
                    .group_hover_color(id, if on { fg } else { fg.opacity(0.7) }),
            )
    }

    fn render_project_search_status(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let search = &self.project_search;
        let line = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .min_h(px(32.0))
            .px_3()
            .py(px(6.0))
            .text_size(px(11.0))
            .text_color(fg.opacity(0.45));
        if search.loading {
            return line
                .child(
                    Icon::new(IconName::LoaderCircle)
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.45)),
                )
                .child("Searching…");
        }
        if let Some(error) = &search.error {
            return line.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.danger)
                    .child(SharedString::from(error.clone())),
            );
        }
        if search.searched.query.is_empty() {
            return line.child("Type to search across the project");
        }
        line.child(summary(&search.result.matches, search.result.truncated))
    }

    fn render_project_search_results(&self, cx: &Context<Self>) -> impl IntoElement {
        let search = &self.project_search;
        let rows = rows(&search.result.matches);
        let heights: Vec<Option<f32>> = rows.iter().map(Row::height).collect();
        let visible = virtual_rows::for_scroll(&heights, &search.scroll, 0.0);
        let built: Vec<AnyElement> = rows[visible.range.clone()]
            .iter()
            .map(|row| match row {
                Row::File {
                    name,
                    relative,
                    count,
                } => render_file_row(name, relative, *count, cx).into_any_element(),
                Row::Match { index, hit } => self.render_match_row(*index, hit, cx),
            })
            .collect();
        crate::ui::scrollbar::framed(
            "project-search-scrollbar",
            &search.scroll,
            div()
                .id("project-search-results")
                .track_scroll(&search.scroll)
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .overflow_x_hidden()
                .child(div().flex_none().h(px(visible.above)))
                .children(built)
                .child(div().flex_none().h(px(visible.below))),
        )
    }

    fn render_match_row(&self, index: usize, hit: &SearchMatch, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let searched = &self.project_search.searched;
        // A regex's match length is unknown here; it is not marked.
        let len = if searched.regex {
            0
        } else {
            searched.query.len()
        };
        let preview = hit.preview.trim_end();
        let start = (hit.column as usize).saturating_sub(1);
        let mark =
            (len > 0 && preview.get(start..start + len).is_some()).then(|| start..start + len);
        let style = HighlightStyle {
            background_color: Some(theme.colors.accent.opacity(0.35)),
            color: Some(fg),
            ..Default::default()
        };
        let line = hit.line.to_string();
        let hit = hit.clone();
        div()
            .id(("search-match", index))
            .flex()
            .items_center()
            .gap_2()
            .h(px(MATCH_ROW))
            .px_2()
            .hover(move |s| s.bg(fg.opacity(0.05)))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_search_match(&hit, window, cx)),
            )
            .font_family(theme.mono_family.clone())
            .text_size(px(11.0))
            .child(
                div()
                    .w(px(28.0))
                    .flex_none()
                    .text_right()
                    .text_color(fg.opacity(0.35))
                    .child(line),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(fg.opacity(0.8))
                    .child(
                        StyledText::new(preview.to_string())
                            .with_highlights(mark.map(|range| (range, style))),
                    ),
            )
            .into_any_element()
    }
}

/// The file's icon, name and match count over its path.
fn render_file_row(name: &str, path: &str, count: usize, cx: &gpui::App) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    div()
        .flex()
        .flex_col()
        .justify_center()
        .h(px(FILE_ROW))
        .px_2()
        .border_t_1()
        .border_color(fg.opacity(0.07))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(resolve_entry_icon(name, false, false).size(ENTRY_ICON))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(fg)
                        .child(SharedString::from(name.to_string())),
                )
                .child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded_full()
                        .bg(colors.accent.opacity(0.2))
                        .text_size(px(10.0))
                        .line_height(relative(1.6))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.accent)
                        .child(count.to_string()),
                ),
        )
        .child(
            div()
                .truncate()
                .text_size(px(10.0))
                .text_color(fg.opacity(0.4))
                .child(SharedString::from(path.to_string())),
        )
}

/// MonoCode's search fields: a tinted box around the input.
fn field(input: &Entity<ely_gpui_component::forms::TextInput>, size: f32, fg: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_h(px(28.0))
        .pl_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(fg.opacity(0.10))
        .bg(fg.opacity(0.05))
        .text_size(px(size))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(relative: &str, line: u32) -> SearchMatch {
        SearchMatch {
            relative: relative.into(),
            line,
            column: 1,
            preview: String::new(),
        }
    }

    #[test]
    fn matches_group_under_their_file() {
        let matches = [hit("src/a.rs", 1), hit("src/a.rs", 4), hit("b.md", 2)];
        let rows = rows(&matches);
        let shape: Vec<String> = rows
            .iter()
            .map(|row| match row {
                Row::File { name, count, .. } => format!("{name}:{count}"),
                Row::Match { hit, .. } => hit.line.to_string(),
            })
            .collect();
        assert_eq!(shape, ["a.rs:2", "1", "4", "b.md:1", "2"]);
        assert_eq!(summary(&matches, false), "3 results in 2 files");
        assert_eq!(summary(&matches[..1], true), "1 result in 1 file (limited)");
        assert_eq!(summary(&[], false), "No results");
    }
}
