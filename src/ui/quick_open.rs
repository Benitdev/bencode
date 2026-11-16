//! Go to File, ⌘P (MonoCode `FilePicker` + `fileIndex.rankProjectFiles` +
//! `shared/lib/fuzzy`): a dialog near the top of the window that ranks the
//! project's files by a fuzzy match on their name, then their path, with
//! recently opened files first. Starting the query with `>` makes it the
//! command palette (⇧⌘P opens it so): every command of
//! `app/command_catalog.rs` by name, with its keys, run as if its keys
//! had been pressed where the keyboard was.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use bencode_ui::typography::KbdCombo;
use gpui::{
    AnyElement, Context, Focusable, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, StyledText, Window, deferred, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::Surface;
use crate::app::command_catalog::{PaletteCommand, Scope, palette};
use crate::ui::composer::focus_later;
use crate::ui::file_tree::resolve_entry_icon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

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
/// word starts and camelCase humps. `needle` is the token lowercased.
fn match_token(needle: &[char], text: &str) -> Option<Hit> {
    if needle.is_empty() {
        return Some(Hit {
            score: 0,
            positions: Vec::new(),
        });
    }
    let mut positions = Vec::new();
    let (mut score, mut run, mut qi) = (0_i64, 0_i64, 0);
    let mut prev: Option<char> = None;
    for (i, c) in text.chars().enumerate() {
        if qi == needle.len() {
            break;
        }
        let before = prev.replace(c);
        if c.to_lowercase().next() != Some(needle[qi]) {
            run = 0;
            continue;
        }
        positions.push(i);
        run += 1;
        score += 1 + run * 4;
        match before {
            None => score += 14,
            Some(b) if is_break(b) => score += 14,
            Some(b) if c.is_ascii_uppercase() && !b.is_ascii_uppercase() => score += 10,
            Some(_) => {}
        }
        qi += 1;
    }
    if qi != needle.len() {
        return None;
    }
    score -= text.chars().count() as i64 - needle.len() as i64;
    Some(Hit { score, positions })
}

/// A query split and lowercased once, to match against many texts.
pub struct FuzzyQuery {
    tokens: Vec<Vec<char>>,
}

impl FuzzyQuery {
    pub fn new(query: &str) -> Self {
        Self {
            tokens: query
                .split_whitespace()
                .map(|token| token.to_lowercase().chars().collect())
                .collect(),
        }
    }

    /// MonoCode `fuzzyMatch`: space-separated tokens must all match.
    pub fn matches(&self, text: &str) -> Option<Hit> {
        match self.tokens.as_slice() {
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
    pub fn score_path(&self, relative: &str) -> Option<Hit> {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        if let Some(hit) = self.matches(name) {
            let offset = relative.chars().count() - name.chars().count();
            return Some(Hit {
                score: hit.score + 400,
                positions: hit.positions.into_iter().map(|p| p + offset).collect(),
            });
        }
        self.matches(relative)
    }
}

/// MonoCode `fuzzyMatch`, for one text; many texts share a `FuzzyQuery`.
pub fn fuzzy_match(query: &str, text: &str) -> Option<Hit> {
    FuzzyQuery::new(query).matches(text)
}

/// MonoCode `rankProjectFiles` over `lists` (files, and folders for `@`;
/// each sorted, as the index keeps them): with no query, the recent files; otherwise the matches, recent ones
/// lifted, best first. Runs over the whole index on each keystroke: one
/// pass, and only the shown results become `String`s.
pub fn rank_files(
    lists: &[&[SharedString]],
    query: &str,
    recents: &[String],
) -> Vec<(String, Hit)> {
    let files = || lists.iter().flat_map(|list| list.iter());
    if query.trim().is_empty() {
        let listed = |path: &str| {
            lists
                .iter()
                .any(|list| list.binary_search_by(|f| f.as_ref().cmp(path)).is_ok())
        };
        return recents
            .iter()
            .filter(|path| listed(path))
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
    let recency: HashMap<&str, usize> = recents
        .iter()
        .enumerate()
        .map(|(rank, path)| (path.as_str(), rank))
        .collect();
    let query = FuzzyQuery::new(query.trim());
    let mut scored: Vec<(&str, Hit)> = files()
        .filter_map(|file| {
            let mut hit = query.score_path(file)?;
            if let Some(rank) = recency.get(file.as_ref()) {
                hit.score += (MAX_RECENTS.saturating_sub(*rank) * 8) as i64;
            }
            Some((file.as_ref(), hit))
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
        .into_iter()
        .map(|(path, hit)| (path.to_string(), hit))
        .collect()
}

/// The last ranking and what it was made from. Render and the keys ask for
/// it again and again with nothing changed (a hover redraws the picker),
/// and ranking walks the whole index.
pub(crate) struct RankCache<T> {
    last: RefCell<Option<(RankKey, Rc<T>)>>,
}

impl<T> Default for RankCache<T> {
    fn default() -> Self {
        Self {
            last: RefCell::new(None),
        }
    }
}

/// What a ranking was made from. Holding the lists keeps their addresses
/// from being reused, so pointer equality means the same list.
struct RankKey {
    query: String,
    lists: Vec<Arc<[SharedString]>>,
    recents: Vec<String>,
}

impl RankKey {
    fn made_from(&self, query: &str, lists: &[&Arc<[SharedString]>], recents: &[String]) -> bool {
        self.query == query
            && self.recents == recents
            && self.lists.len() == lists.len()
            && self.lists.iter().zip(lists).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

impl<T> RankCache<T> {
    /// The ranking of `lists` for `query`, made by `rank` unless the last
    /// one was made from the same.
    pub fn get(
        &self,
        query: &str,
        lists: &[&Arc<[SharedString]>],
        recents: &[String],
        rank: impl FnOnce() -> T,
    ) -> Rc<T> {
        if let Some((key, ranked)) = &*self.last.borrow()
            && key.made_from(query, lists, recents)
        {
            return ranked.clone();
        }
        let ranked = Rc::new(rank());
        let key = RankKey {
            query: query.to_string(),
            lists: lists.iter().map(|list| Arc::clone(list)).collect(),
            recents: recents.to_vec(),
        };
        *self.last.borrow_mut() = Some((key, ranked.clone()));
        ranked
    }

    /// Lets go of the last ranking (and the lists it holds).
    pub fn clear(&self) {
        self.last.take();
    }
}

/// How many commands run from the palette are kept to list first.
const MAX_RECENT_COMMANDS: usize = 8;

/// The palette's list for `query` (what follows `>`): indices into
/// `commands`, those out of `scope` left out. With no query, the commands
/// run last and then the catalog in its order; otherwise the matches, best
/// first, a recent one lifted.
fn rank_commands(
    commands: &[PaletteCommand],
    query: &str,
    recents: &[usize],
    in_scope: impl Fn(Scope) -> bool,
) -> Vec<(usize, Hit)> {
    let offered = |ix: &usize| commands[*ix].scope.is_none_or(&in_scope);
    if query.is_empty() {
        let none = || Hit {
            score: 0,
            positions: Vec::new(),
        };
        let rest = (0..commands.len()).filter(|ix| !recents.contains(ix));
        return (recents.iter().copied().chain(rest))
            .filter(offered)
            .map(|ix| (ix, none()))
            .collect();
    }
    let query = FuzzyQuery::new(query);
    let mut hits: Vec<(usize, Hit)> = (0..commands.len())
        .filter(offered)
        .filter_map(|ix| {
            let mut hit = query.matches(&commands[ix].label)?;
            if let Some(rank) = recents.iter().position(|recent| *recent == ix) {
                hit.score += ((MAX_RECENT_COMMANDS - rank) * 4) as i64;
            }
            Some((ix, hit))
        })
        .collect();
    // Stable: equal scores keep the catalog's order.
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.1.score));
    hits
}

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
    /// The list's scroll, so the highlighted row stays in view.
    scroll: gpui::ScrollHandle,
    /// Files picked here, newest first (MonoCode `rememberOpenedFile`).
    recents: Vec<String>,
    /// A file picked from the keyboard, opened on the next frame (opening
    /// needs the window).
    pending: Option<String>,
    ranked: RankCache<Results>,
    /// The palette's commands, listed once: the keymap does not change
    /// while BenCode runs.
    commands: std::cell::OnceCell<Vec<PaletteCommand>>,
    /// Commands run from the palette, newest first (indices into
    /// `commands`).
    recent_commands: Vec<usize>,
    /// A command picked, run on the next frame (dispatching needs the
    /// window).
    pending_command: Option<usize>,
    /// What had the keyboard when the picker opened: a command runs there.
    return_focus: Option<gpui::FocusHandle>,
}

enum Results {
    Files(Vec<(String, Hit)>),
    Commands(Vec<(usize, Hit)>),
}

impl Results {
    fn len(&self) -> usize {
        match self {
            Self::Files(files) => files.len(),
            Self::Commands(commands) => commands.len(),
        }
    }
}

impl BenCodeApp {
    fn quick_open_query(&self, cx: &gpui::App) -> String {
        self.quick_open_input.read(cx).text().to_string()
    }

    /// Recent picks, then the files open in the editor.
    pub(crate) fn quick_open_recents(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let open = self.editor.files.iter().map(|f| f.path.clone());
        for path in self.quick_open.recents.iter().cloned().chain(open) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }

    fn quick_open_results(&self, cx: &gpui::App) -> Rc<Results> {
        let query = self.quick_open_query(cx);
        if let Some(command) = query.trim().strip_prefix('>') {
            // A hundred names: ranked as asked for, not cached.
            let inbox = self.surface_open(Surface::Inbox);
            return Rc::new(Results::Commands(rank_commands(
                self.palette_commands(),
                command.trim(),
                &self.quick_open.recent_commands,
                |scope| match scope {
                    Scope::Inbox => inbox,
                },
            )));
        }
        let recents = self.quick_open_recents();
        let files = &self.project_files.files;
        self.quick_open.ranked.get(&query, &[files], &recents, || {
            Results::Files(rank_files(&[files], &query, &recents))
        })
    }

    fn palette_commands(&self) -> &[PaletteCommand] {
        self.quick_open.commands.get_or_init(palette)
    }

    /// ⌘P and ⇧⌘P: the picker, as the command palette when `commands`.
    /// What has the keyboard now is where a picked command will run.
    pub fn open_quick_open_from(
        &mut self,
        window: &Window,
        commands: bool,
        cx: &mut Context<Self>,
    ) {
        // Opened again over itself: the keyboard is the picker's own.
        if !self.quick_open.open {
            self.quick_open.return_focus = window.focused(cx);
        }
        self.open_quick_open(cx);
        if commands {
            self.quick_open_input
                .update(cx, |input, cx| input.set_text(">", cx));
        }
    }

    /// ⌘P: opens the picker with an empty query and a fresh file index.
    pub fn open_quick_open(&mut self, cx: &mut Context<Self>) {
        self.close_search_modal(cx);
        self.quick_open.open = true;
        self.quick_open.active = 0;
        self.quick_open_input
            .update(cx, |input, cx| input.set_text("", cx));
        focus_later(self.quick_open_input.read(cx).focus_handle(cx), cx);
        self.index_project_files(cx);
        cx.notify();
    }

    pub fn close_quick_open(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.hide_quick_open(cx) {
            return false;
        }
        self.quick_open.return_focus = None;
        self.refocus_prompt(cx);
        true
    }

    /// Takes the picker down, the keyboard left where it is.
    fn hide_quick_open(&mut self, cx: &mut Context<Self>) -> bool {
        if !std::mem::take(&mut self.quick_open.open) {
            return false;
        }
        self.quick_open.ranked.clear();
        cx.notify();
        true
    }

    pub fn on_quick_open_query_changed(&mut self, cx: &mut Context<Self>) {
        self.quick_open.active = 0;
        self.quick_open.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    /// ↑/↓ wrap, Enter picks, Esc closes, Tab is swallowed.
    pub fn quick_open_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let count = self.quick_open_results(cx).len();
        match key {
            "down" | "up" if count > 0 => {
                let step = if key == "down" { 1 } else { count - 1 };
                self.quick_open.active = (self.quick_open.active + step) % count;
                self.quick_open
                    .scroll
                    .scroll_to_item(self.quick_open.active);
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
        match &*self.quick_open_results(cx) {
            Results::Files(files) => {
                let Some((path, _)) = files.get(index).cloned() else {
                    return;
                };
                self.quick_open.recents.retain(|p| *p != path);
                self.quick_open.recents.insert(0, path.clone());
                self.quick_open.recents.truncate(MAX_RECENTS);
                self.quick_open.pending = Some(path);
            }
            Results::Commands(commands) => {
                let Some((command, _)) = commands.get(index) else {
                    return;
                };
                let command = *command;
                let recents = &mut self.quick_open.recent_commands;
                recents.retain(|recent| *recent != command);
                recents.insert(0, command);
                recents.truncate(MAX_RECENT_COMMANDS);
                self.quick_open.pending_command = Some(command);
                // The keyboard goes back where it was, not to the prompt:
                // `open_pending_quick_open` runs the command there.
                self.hide_quick_open(cx);
                return;
            }
        }
        self.close_quick_open(cx);
    }

    /// Opens a file picked from the keyboard, now that a window is at hand.
    pub fn open_pending_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.quick_open.pending.take() {
            self.open_file_in_editor(&path, window, cx);
        }
        if let Some(command) = self.quick_open.pending_command.take() {
            // As if its keys had been pressed where the keyboard was: a
            // command that reads the focus (a terminal's, the file pane's)
            // sees what it would have. What it focuses itself comes after.
            match self.quick_open.return_focus.take() {
                Some(focus) => window.focus(&focus, cx),
                None => self.refocus_prompt(cx),
            }
            if let Some(command) = self.palette_commands().get(command) {
                window.dispatch_action(command.action.boxed_clone(), cx);
            }
        }
    }

    /// MonoCode `emptyLabel`.
    fn quick_open_empty(&self, results: &Results, cx: &gpui::App) -> Option<&'static str> {
        match results {
            Results::Commands(c) => c.is_empty().then_some("No matching commands"),
            Results::Files(files) => {
                if self.project_files.root.trim().is_empty() {
                    Some("Open a project to search files")
                } else if self.project_files.files.is_empty() {
                    Some(if self.project_files.loading {
                        "Indexing files…"
                    } else {
                        "No files found"
                    })
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
            .text_size(Text::Lg.px())
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
        let results = &*results;
        let empty = self.quick_open_empty(results, cx);
        let palette = matches!(results, Results::Commands(_));
        let rows: Vec<AnyElement> = match results {
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
                    let icon = resolve_entry_icon(name, false, false);
                    self.quick_open_row(ix, cx)
                        .child(icon.size(IconSize::Sm))
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
                                    .text_size(Text::Sm.px())
                                    .text_color(colors.fg.ink(Ink::Muted))
                                    .child(match_text(dir, &dir_hits, cx)),
                            )
                        })
                        .into_any_element()
                })
                .collect(),
            Results::Commands(commands) => commands
                .iter()
                .enumerate()
                .map(|(ix, (command, hit))| {
                    let command = &self.palette_commands()[*command];
                    self.quick_open_row(ix, cx)
                        .child(
                            Icon::new(command.icon)
                                .size(IconSize::Sm)
                                .color(colors.fg.ink(Ink::Secondary)),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(match_text(
                            &command.label,
                            &hit.positions,
                            cx,
                        )))
                        .when_some(command.keys.as_deref(), |el, keys| {
                            el.child(div().flex_none().child(KbdCombo::new(keys)))
                        })
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
                    .border_color(colors.fg.fill(Fill::Strong))
                    .shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(crate::ui::frost::dialog_frost(7.0, colors.surface, cx))
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
                                        .color(colors.fg.ink(Ink::Secondary)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(Text::Md.px())
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
                                .text_size(Text::Base.px())
                                .text_color(colors.fg.ink(Ink::Secondary))
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
                                .track_scroll(&self.quick_open.scroll)
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
                    .occlude()
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
        let ranked = rank_files(&[&files], "agent", &[]);
        assert_eq!(ranked[0].0, "src/app/agent.rs");
        assert_eq!(ranked[0].1.positions, (8..13).collect::<Vec<_>>());
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn commands_rank_by_name_recent_first_and_stay_in_scope() {
        let commands = palette();
        let at = |label: &str| commands.iter().position(|c| c.label == label).unwrap();
        let labels = |ranked: &[(usize, Hit)]| -> Vec<&str> {
            ranked
                .iter()
                .map(|(ix, _)| commands[*ix].label.as_str())
                .collect()
        };
        // No query: what was run last, then the catalog as written.
        let recents = [at("Toggle Terminal")];
        let all = rank_commands(&commands, "", &recents, |_| false);
        assert_eq!(labels(&all)[0], "Toggle Terminal");
        assert_eq!(all.iter().filter(|(ix, _)| *ix == recents[0]).count(), 1);
        assert!(
            labels(&all)
                .iter()
                .all(|label| !label.starts_with("Inbox: "))
        );
        // The Inbox's own commands only while it is open.
        let inbox = rank_commands(&commands, "translate", &[], |_| true);
        assert_eq!(labels(&inbox), ["Inbox: Translate"]);
        assert!(rank_commands(&commands, "translate", &[], |_| false).is_empty());
        let split = rank_commands(&commands, "split r", &[], |_| false);
        assert_eq!(labels(&split)[0], "Split Pane Right");
    }

    #[test]
    fn empty_query_lists_recent_files_that_exist() {
        let files: Vec<SharedString> = ["a.rs", "b.rs"].map(SharedString::from).to_vec();
        let recents = vec!["b.rs".to_string(), "gone.rs".to_string()];
        let ranked = rank_files(&[&files], "", &recents);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, "b.rs");
        let lifted = rank_files(&[&files], "rs", &recents);
        assert_eq!(lifted[0].0, "b.rs");
    }
}
