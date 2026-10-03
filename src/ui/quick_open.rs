//! Go to File, ⌘P (MonoCode `FilePicker` + `fileIndex.rankProjectFiles` +
//! `shared/lib/fuzzy`): a dialog near the top of the window that ranks the
//! project's files by a fuzzy match on their name, then their path, with
//! recently opened files first. Starting the query with `>` lists commands.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Focusable, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, StyledText, Window, deferred, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::ui::composer::focus_later;
use crate::ui::file_tree::resolve_entry_icon;

/// MonoCode `MAX_RECENTS`, `MAX_RESULTS`.
const MAX_RECENTS: usize = 30;
const MAX_RESULTS: usize = 80;
const PICKER_WIDTH: f32 = 560.0;
const LIST_MAX_HEIGHT: f32 = 380.0;

/// A fuzzy hit: its score and the matched character indices.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub score: i64,
    pub positions: Vec<usize>,
}

fn is_break(c: char) -> bool {
    matches!(c, '/' | '\\' | '-' | '_' | '.' | ' ')
}

/// MonoCode `matchToken`: every query character in order, rewarding runs,
/// word starts and camelCase humps.
fn match_token(query: &str, text: &str) -> Option<Hit> {
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() {
        return Some(Hit {
            score: 0,
            positions: Vec::new(),
        });
    }
    let chars: Vec<char> = text.chars().collect();
    let mut positions = Vec::new();
    let (mut score, mut run, mut qi) = (0_i64, 0_i64, 0);
    for (i, &c) in chars.iter().enumerate() {
        if qi == needle.len() {
            break;
        }
        if c.to_lowercase().next() != Some(needle[qi]) {
            run = 0;
            continue;
        }
        positions.push(i);
        run += 1;
        score += 1 + run * 4;
        if i == 0 || is_break(chars[i - 1]) {
            score += 14;
        } else if c.is_ascii_uppercase() && !chars[i - 1].is_ascii_uppercase() {
            score += 10;
        }
        qi += 1;
    }
    if qi != needle.len() {
        return None;
    }
    score -= chars.len() as i64 - needle.len() as i64;
    Some(Hit { score, positions })
}

/// MonoCode `fuzzyMatch`: space-separated tokens must all match.
pub fn fuzzy_match(query: &str, text: &str) -> Option<Hit> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    match tokens.as_slice() {
        [] => Some(Hit {
            score: 0,
            positions: Vec::new(),
        }),
        [one] => match_token(one, text),
        many => {
            let mut hit = Hit {
                score: 0,
                positions: Vec::new(),
            };
            for token in many {
                let part = match_token(token, text)?;
                hit.score += part.score;
                hit.positions.extend(part.positions);
            }
            hit.positions.sort_unstable();
            Some(hit)
        }
    }
}

/// MonoCode `scorePath`: a hit in the file name beats one in the folders.
pub fn score_path(query: &str, relative: &str) -> Option<Hit> {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let offset = relative.chars().count() - name.chars().count();
    if let Some(hit) = fuzzy_match(query, name) {
        return Some(Hit {
            score: hit.score + 400,
            positions: hit.positions.into_iter().map(|p| p + offset).collect(),
        });
    }
    fuzzy_match(query, relative)
}

/// MonoCode `rankProjectFiles`: with no query, the recent files; otherwise
/// the matches, recent ones lifted, best first.
pub fn rank_files(files: &[SharedString], query: &str, recents: &[String]) -> Vec<(String, Hit)> {
    if query.trim().is_empty() {
        return recents
            .iter()
            .filter(|path| files.iter().any(|f| f.as_ref() == path.as_str()))
            .take(MAX_RESULTS)
            .map(|path| {
                (
                    path.clone(),
                    Hit {
                        score: 0,
                        positions: Vec::new(),
                    },
                )
            })
            .collect();
    }
    let mut scored: Vec<(String, Hit)> = files
        .iter()
        .filter_map(|file| {
            let mut hit = score_path(query.trim(), file)?;
            if let Some(rank) = recents.iter().position(|r| r.as_str() == file.as_ref()) {
                hit.score += (MAX_RECENTS.saturating_sub(rank) * 8) as i64;
            }
            Some((file.to_string(), hit))
        })
        .collect();
    scored.sort_by(|(a, ha), (b, hb)| {
        hb.score
            .cmp(&ha.score)
            .then(a.len().cmp(&b.len()))
            .then_with(|| a.cmp(b))
    });
    scored.truncate(MAX_RESULTS);
    scored
}

/// A command offered after `>`.
struct Action {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
}

/// MonoCode's palette: its one command, reloading the app.
const ACTIONS: [Action; 1] = [Action {
    id: "reload",
    label: "Reload BenCode",
    hint: "⌘⇧R",
}];

/// Paints the matched characters of `text` (MonoCode `MatchText`).
fn match_text(text: &str, positions: &[usize], cx: &gpui::App) -> StyledText {
    let accent = cx.theme().colors.accent;
    let style = HighlightStyle {
        color: Some(accent),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let highlights = text
        .char_indices()
        .enumerate()
        .filter(|(ix, _)| positions.contains(ix))
        .map(|(_, (at, c))| (at..at + c.len_utf8(), style));
    StyledText::new(text.to_string()).with_highlights(highlights)
}

/// The open picker.
#[derive(Default)]
pub struct QuickOpen {
    pub open: bool,
    pub active: usize,
    /// Files picked here, newest first (MonoCode `rememberOpenedFile`).
    recents: Vec<String>,
    /// A file picked from the keyboard, opened on the next frame (opening
    /// needs the window).
    pending: Option<String>,
}

enum Results {
    Files(Vec<(String, Hit)>),
    Actions(Vec<(usize, Hit)>),
}

impl Results {
    fn len(&self) -> usize {
        match self {
            Self::Files(files) => files.len(),
            Self::Actions(actions) => actions.len(),
        }
    }
}

impl BenCodeApp {
    fn quick_open_query(&self, cx: &gpui::App) -> String {
        self.quick_open_input.read(cx).text().to_string()
    }

    /// Recent picks, then the files open in the editor.
    fn quick_open_recents(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let open = self.editor.files.iter().map(|f| f.path.clone());
        for path in self.quick_open.recents.iter().cloned().chain(open) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }

    fn quick_open_results(&self, cx: &gpui::App) -> Results {
        let query = self.quick_open_query(cx);
        match query.trim().strip_prefix('>') {
            Some(command) => {
                let command = command.trim();
                let mut hits: Vec<(usize, Hit)> = ACTIONS
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, a)| Some((ix, fuzzy_match(command, a.label)?)))
                    .collect();
                hits.sort_by(|a, b| b.1.score.cmp(&a.1.score));
                Results::Actions(hits)
            }
            None => Results::Files(rank_files(
                &self.workspace.files,
                &query,
                &self.quick_open_recents(),
            )),
        }
    }

    /// ⌘P: opens the picker with an empty query and a fresh file index.
    pub fn open_quick_open(&mut self, cx: &mut Context<Self>) {
        self.quick_open.open = true;
        self.quick_open.active = 0;
        self.quick_open_input
            .update(cx, |input, cx| input.set_text("", cx));
        focus_later(self.quick_open_input.read(cx).focus_handle(cx), cx);
        self.refresh_workspace(cx);
        cx.notify();
    }

    pub fn close_quick_open(&mut self, cx: &mut Context<Self>) -> bool {
        if !std::mem::take(&mut self.quick_open.open) {
            return false;
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    pub fn on_quick_open_query_changed(&mut self, cx: &mut Context<Self>) {
        self.quick_open.active = 0;
        cx.notify();
    }

    /// ↑/↓ wrap, Enter picks, Esc closes, Tab is swallowed.
    pub fn quick_open_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let count = self.quick_open_results(cx).len();
        match key {
            "down" if count > 0 => self.quick_open.active = (self.quick_open.active + 1) % count,
            "up" if count > 0 => {
                self.quick_open.active = (self.quick_open.active + count - 1) % count
            }
            "enter" => self.quick_open_pick(self.quick_open.active, cx),
            "escape" => {
                self.close_quick_open(cx);
            }
            "tab" | "up" | "down" => {}
            _ => return false,
        }
        cx.notify();
        true
    }

    fn quick_open_pick(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.quick_open_results(cx) {
            Results::Files(files) => {
                let Some((path, _)) = files.into_iter().nth(index) else {
                    return;
                };
                self.quick_open.recents.retain(|p| *p != path);
                self.quick_open.recents.insert(0, path.clone());
                self.quick_open.recents.truncate(MAX_RECENTS);
                self.quick_open.pending = Some(path);
            }
            Results::Actions(actions) => {
                if let Some((ix, _)) = actions.get(index)
                    && ACTIONS[*ix].id == "reload"
                {
                    cx.restart();
                }
            }
        }
        self.close_quick_open(cx);
    }

    /// Opens a file picked from the keyboard, now that a window is at hand.
    pub fn open_pending_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.quick_open.pending.take() {
            self.open_file_in_editor(&path, window, cx);
        }
    }

    /// MonoCode `emptyLabel`.
    fn quick_open_empty(&self, results: &Results, cx: &gpui::App) -> Option<&'static str> {
        match results {
            Results::Actions(a) => a.is_empty().then_some("No matching commands"),
            Results::Files(files) => {
                if self.workspace.cwd.trim().is_empty() {
                    Some("Open a project to search files")
                } else if self.workspace.files.is_empty() {
                    Some("No files found")
                } else if files.is_empty() {
                    Some(if self.quick_open_query(cx).trim().is_empty() {
                        "Type a file name to search"
                    } else {
                        "No matching files"
                    })
                } else {
                    None
                }
            }
        }
    }

    fn quick_open_row(&self, index: usize, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        let highlighted = index == self.quick_open.active;
        div()
            .id(SharedString::from(format!("quick-open-{index}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px_2()
            .rounded(px(6.0))
            .text_size(px(14.0))
            .text_color(colors.fg)
            .cursor_pointer()
            .when(highlighted, |el| el.bg(colors.active))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.quick_open.active != index {
                    this.quick_open.active = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.quick_open_pick(index, cx);
                this.open_pending_quick_open(window, cx);
            }))
    }

    pub fn render_quick_open(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.quick_open.open {
            return None;
        }
        let colors = &cx.theme().colors;
        let results = self.quick_open_results(cx);
        let empty = self.quick_open_empty(&results, cx);
        let palette = matches!(results, Results::Actions(_));
        let rows: Vec<AnyElement> = match &results {
            Results::Files(files) => files
                .iter()
                .enumerate()
                .map(|(ix, (path, hit))| {
                    let (dir, name) = path
                        .rsplit_once('/')
                        .map_or(("", path.as_str()), |(d, n)| (d, n));
                    let name_offset = if dir.is_empty() {
                        0
                    } else {
                        dir.chars().count() + 1
                    };
                    let name_hits: Vec<usize> = hit
                        .positions
                        .iter()
                        .filter(|p| **p >= name_offset)
                        .map(|p| p - name_offset)
                        .collect();
                    let dir_hits: Vec<usize> = hit
                        .positions
                        .iter()
                        .copied()
                        .filter(|p| *p < dir.chars().count())
                        .collect();
                    let (icon, tint) = resolve_entry_icon(name, false, false);
                    self.quick_open_row(ix, cx)
                        .child(Icon::new(icon).size(IconSize::Sm).color(tint))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(match_text(name, &name_hits, cx)),
                        )
                        .when(!dir.is_empty(), |el| {
                            el.child(
                                div()
                                    .min_w_0()
                                    .max_w(px(PICKER_WIDTH * 0.45))
                                    .truncate()
                                    .font_family(cx.theme().mono_family.clone())
                                    .text_size(px(11.0))
                                    .text_color(colors.fg.opacity(0.4))
                                    .child(match_text(dir, &dir_hits, cx)),
                            )
                        })
                        .into_any_element()
                })
                .collect(),
            Results::Actions(actions) => actions
                .iter()
                .enumerate()
                .map(|(ix, (action, hit))| {
                    let action = &ACTIONS[*action];
                    self.quick_open_row(ix, cx)
                        .child(
                            Icon::new(IconName::RefreshCw)
                                .size(IconSize::Sm)
                                .color(colors.fg.opacity(0.5)),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(match_text(
                            action.label,
                            &hit.positions,
                            cx,
                        )))
                        .child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(colors.fg.opacity(0.1))
                                .bg(colors.fg.opacity(0.05))
                                .font_family(cx.theme().mono_family.clone())
                                .text_size(px(10.0))
                                .text_color(colors.fg.opacity(0.5))
                                .child(action.hint),
                        )
                        .into_any_element()
                })
                .collect(),
        };
        let dialog = div()
            .id("quick-open")
            .absolute()
            .top(gpui::relative(0.12))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("quick-open-box")
                    .w(px(PICKER_WIDTH))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(colors.fg.opacity(0.1))
                    .bg(colors.surface)
                    .shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div().pb_1p5().child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py(px(10.0))
                                .border_b_1()
                                .border_color(colors.border)
                                .child(
                                    Icon::new(IconName::Search)
                                        .size(IconSize::Xs)
                                        .color(colors.fg.opacity(0.5)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(13.0))
                                        .child(self.quick_open_input.clone()),
                                ),
                        ),
                    )
                    .map(|el| match empty {
                        Some(label) => el.child(
                            div()
                                .px_3()
                                .pt_1()
                                .pb_3()
                                .text_size(px(12.0))
                                .text_color(colors.fg.opacity(0.5))
                                .child(label),
                        ),
                        None => el.child(
                            div()
                                .id(if palette {
                                    "quick-open-commands"
                                } else {
                                    "quick-open-files"
                                })
                                .max_h(px(LIST_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .px_1p5()
                                .pb_1p5()
                                .children(rows),
                        ),
                    }),
            );
        Some(
            deferred(
                div()
                    .id("quick-open-scrim")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_quick_open(cx);
                        }),
                    )
                    .child(dialog),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_needs_every_char_in_order_and_rewards_word_starts() {
        assert!(fuzzy_match("xyz", "main.rs").is_none());
        let start = fuzzy_match("mr", "main.rs").unwrap();
        let mid = fuzzy_match("ar", "main.rs").unwrap();
        assert!(start.score > mid.score);
        assert_eq!(start.positions, [0, 5]);
        assert!(fuzzy_match("ma rs", "main.rs").is_some());
    }

    #[test]
    fn name_hits_beat_folder_hits() {
        let files: Vec<SharedString> = ["src/app/agent.rs", "agent/notes.md", "README.md"]
            .map(SharedString::from)
            .to_vec();
        let ranked = rank_files(&files, "agent", &[]);
        assert_eq!(ranked[0].0, "src/app/agent.rs");
        assert_eq!(ranked[0].1.positions, (8..13).collect::<Vec<_>>());
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn empty_query_lists_recent_files_that_exist() {
        let files: Vec<SharedString> = ["a.rs", "b.rs"].map(SharedString::from).to_vec();
        let recents = vec!["b.rs".to_string(), "gone.rs".to_string()];
        let ranked = rank_files(&files, "", &recents);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, "b.rs");
        let lifted = rank_files(&files, "rs", &recents);
        assert_eq!(lifted[0].0, "b.rs");
    }
}
