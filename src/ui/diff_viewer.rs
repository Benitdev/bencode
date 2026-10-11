//! MonoCode `UnifiedDiffView` (with `WorkingTreeDiff` and `CommitDiff` as
//! its loaders): every file of a review stacked under a header that sticks
//! to the top, unchanged runs folded behind "N unmodified lines" bars.
//! Headers and rows share one `gpui::list`; git runs only in the loader.
//! BenCode's own: the same review with the old file beside the new one
//! (`DocRow::Pair`), where the pane is wide enough for two lanes. MonoCode's
//! side by side is its editor diff (`@codemirror/merge`), which is not ported.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use ely_gpui_component::files::FileIcon;
use ely_gpui_component::forms::{code_highlights, json_highlights};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, HighlightStyle, Hsla, InteractiveElement, IntoElement, ListAlignment,
    ListOffset, ListState, ParentElement, Pixels, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window, canvas, div, list, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::git::checkpoint::CheckpointStore;
use crate::git::{self, DiffSource};
use crate::ui::diff_counts::diff_counts;
use crate::ui::diff_model::{
    self, Block, BodyRow, Expand, Line, LineKind, Reveal, SplitRow, Syntax,
};
use crate::ui::git_changes_panel::{Busy, Side};
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::{self, px};
use crate::ui::scrollbar::{self, ScrollBar};

/// MonoCode `UNIFIED_LINE_PX` / `UNIFIED_FOLD_PX`.
const LINE_HEIGHT: f32 = 20.0;
const FOLD_HEIGHT: f32 = 32.0;
/// A line's `text-[12px]`.
const LINE_TEXT: f32 = 12.0;
/// The narrowest review that still shows two lanes; under it a side by
/// side review is drawn unified.
const SPLIT_MIN_WIDTH: f32 = 520.0;
/// A longer line (minified code) keeps one colour.
const HIGHLIGHT_MAX_LEN: usize = 2000;
/// MonoCode `DIFF_LOAD_CONCURRENCY`.
const LOAD_THREADS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileBody {
    Binary,
    TooLarge,
    Failed(String),
    /// Nothing textual changed; the message says so.
    Empty(&'static str),
    Diff(Vec<Block>),
}

/// One file of a review (MonoCode `UnifiedDiffFileModel`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocFile {
    pub id: String,
    pub path: String,
    pub label: String,
    /// The working-tree side, which can be staged or discarded from here.
    pub side: Option<Side>,
    pub additions: usize,
    pub deletions: usize,
    pub body: FileBody,
}

/// The files of `new` that differ from `old`, when both list the same
/// files in the same order; `None` when the list itself changed.
fn changed_files(old: &[DocFile], new: &[DocFile]) -> Option<Vec<usize>> {
    if old.len() != new.len() || old.iter().zip(new).any(|(a, b)| a.id != b.id) {
        return None;
    }
    Some(
        old.iter()
            .zip(new)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(ix, _)| ix)
            .collect(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocStatus {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug)]
enum DocRow {
    Header(usize),
    Line(usize, Line),
    /// A row of the side by side layout: the old line and the new one.
    Pair {
        file: usize,
        old: Option<Line>,
        new: Option<Line>,
    },
    Fold {
        file: usize,
        id: usize,
        hidden: usize,
    },
    Message(usize, SharedString),
}

impl DocRow {
    fn file(&self) -> usize {
        match self {
            Self::Header(f) | Self::Line(f, _) | Self::Message(f, _) => *f,
            Self::Fold { file, .. } | Self::Pair { file, .. } => *file,
        }
    }

    /// The lines the row shows.
    fn lines(&self) -> impl Iterator<Item = &Line> {
        let (a, b) = match self {
            Self::Line(_, line) => (Some(line), None),
            Self::Pair { old, new, .. } => (old.as_ref(), new.as_ref()),
            _ => (None, None),
        };
        a.into_iter().chain(b)
    }

    /// The old and new line numbers the row shows.
    fn numbers(&self) -> (Option<u32>, Option<u32>) {
        self.lines().fold((None, None), |(old, new), line| {
            (old.or(line.old), new.or(line.new))
        })
    }
}

/// The file a review scrolls to once loaded: a path, on one side or either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocFocus {
    pub path: String,
    pub side: Option<Side>,
}

/// One review tab's files, which of them are open, and the list showing them.
pub struct DiffDoc {
    pub files: Vec<DocFile>,
    pub status: DocStatus,
    open: HashSet<String>,
    reveals: HashMap<(String, usize), Reveal>,
    rows: Vec<DocRow>,
    /// Each file's first row, then the row count.
    starts: Vec<usize>,
    list: ListState,
    /// How far each file's lines are scrolled sideways, by file id
    /// (MonoCode's `overflow-x-auto` code lane).
    side_scroll: HashMap<String, Pixels>,
    /// Side by side is chosen, and the pane has the room for it.
    split: bool,
    wide: bool,
    focus: Option<DocFocus>,
    generation: u64,
}

impl DiffDoc {
    /// Row heights changed without the rows changing (interface scale).
    pub(crate) fn remeasure(&self) {
        self.list.remeasure();
    }

    fn side_by_side(&self) -> bool {
        self.split && self.wide
    }

    pub(crate) fn set_split(&mut self, split: bool) {
        let was = self.side_by_side();
        self.split = split;
        if self.side_by_side() != was {
            self.relayout();
        }
    }

    fn set_wide(&mut self, wide: bool) {
        let was = self.side_by_side();
        self.wide = wide;
        if self.side_by_side() != was {
            self.relayout();
        }
    }

    /// Rebuilds the rows in the other layout, with the line that was at
    /// the top still there.
    fn relayout(&mut self) {
        let top = self.list.logical_scroll_top();
        let anchor = self
            .rows
            .get(top.item_ix)
            .map(|row| (row.file(), row.numbers()));
        self.rebuild();
        // The lanes changed width: what was in reach may not be now.
        self.side_scroll.clear();
        let Some((file, (old, new))) = anchor else {
            return;
        };
        let Some((from, to)) = self.starts.get(file).zip(self.starts.get(file + 1)) else {
            return;
        };
        let at = self.rows[*from..*to]
            .iter()
            .position(|row| match (row.numbers(), new) {
                ((_, Some(n)), Some(new)) => n >= new,
                ((Some(o), _), None) => old.is_some_and(|old| o >= old),
                _ => false,
            })
            .unwrap_or(0);
        self.list.scroll_to(ListOffset {
            item_ix: from + at,
            offset_in_item: px(0.0),
        });
    }

    fn new(focus: Option<DocFocus>, split: bool) -> Self {
        Self {
            files: Vec::new(),
            status: DocStatus::Loading,
            open: HashSet::new(),
            reveals: HashMap::new(),
            rows: Vec::new(),
            starts: vec![0],
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
            side_scroll: HashMap::new(),
            split,
            // Until the pane is measured (`render_diff_doc`).
            wide: true,
            focus,
            generation: 0,
        }
    }

    fn file_rows(&self, ix: usize) -> Vec<DocRow> {
        let file = &self.files[ix];
        let mut rows = vec![DocRow::Header(ix)];
        if !self.open.contains(&file.id) {
            return rows;
        }
        let message = |text: String| DocRow::Message(ix, text.into());
        match &file.body {
            FileBody::Binary => rows.push(message("Binary file changed".into())),
            FileBody::TooLarge => rows.push(message("Diff is too large to display".into())),
            FileBody::Failed(err) => rows.push(message(format!("Couldn’t load diff: {err}"))),
            FileBody::Empty(text) => rows.push(message((*text).into())),
            FileBody::Diff(blocks) => {
                let reveal_for = |fold: usize| {
                    self.reveals
                        .get(&(file.id.clone(), fold))
                        .copied()
                        .unwrap_or_default()
                };
                let body = diff_model::body_rows(blocks, reveal_for);
                if self.side_by_side() {
                    rows.extend(
                        diff_model::split_rows(body)
                            .into_iter()
                            .map(|row| match row {
                                SplitRow::Pair { old, new } => DocRow::Pair { file: ix, old, new },
                                SplitRow::Fold { id, hidden } => DocRow::Fold {
                                    file: ix,
                                    id,
                                    hidden,
                                },
                            }),
                    );
                } else {
                    rows.extend(body.into_iter().map(|row| match row {
                        BodyRow::Line(line) => DocRow::Line(ix, line),
                        BodyRow::Fold { id, hidden } => DocRow::Fold {
                            file: ix,
                            id,
                            hidden,
                        },
                    }));
                }
            }
        }
        rows
    }

    /// Rebuilds every row, keeping the scroll position.
    fn rebuild(&mut self) {
        let top = self.list.logical_scroll_top();
        self.rows.clear();
        self.starts = vec![0];
        for ix in 0..self.files.len() {
            let rows = self.file_rows(ix);
            self.rows.extend(rows);
            self.starts.push(self.rows.len());
        }
        self.list.reset(self.rows.len());
        self.list.scroll_to(top);
        let files = &self.files;
        self.side_scroll
            .retain(|id, _| files.iter().any(|file| &file.id == id));
    }

    /// The longest line file `ix` shows, in characters.
    fn widest_line(&self, ix: usize) -> usize {
        let Some((from, to)) = self.starts.get(ix).zip(self.starts.get(ix + 1)) else {
            return 0;
        };
        self.rows[*from..*to]
            .iter()
            .flat_map(DocRow::lines)
            .map(|line| line.text.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// Rebuilds one file's rows in place, so the list keeps its scroll.
    fn rebuild_file(&mut self, ix: usize) {
        let Some(range) = self.starts.get(ix).zip(self.starts.get(ix + 1)) else {
            return;
        };
        let range = *range.0..*range.1;
        let rows = self.file_rows(ix);
        let delta = rows.len() as isize - range.len() as isize;
        self.list.splice(range.clone(), rows.len());
        self.rows.splice(range, rows);
        for start in &mut self.starts[ix + 1..] {
            *start = (*start as isize + delta) as usize;
        }
    }

    fn scroll_to_focus(&mut self) {
        let Some(focus) = self.focus.take() else {
            return;
        };
        let found = self
            .files
            .iter()
            .position(|f| f.path == focus.path && (focus.side.is_none() || f.side == focus.side))
            .or_else(|| self.files.iter().position(|f| f.path == focus.path));
        if let Some(ix) = found {
            self.list.scroll_to(ListOffset {
                item_ix: self.starts[ix],
                offset_in_item: px(0.0),
            });
        }
    }

    /// One lane of file `ix`: `line` as `number`, or the blank side of a
    /// line only the other side has.
    fn lane(
        &self,
        ix: usize,
        line: Option<&Line>,
        number: Option<u32>,
        cx: &gpui::App,
    ) -> gpui::Div {
        let Some(line) = line else {
            return div()
                .h(px(LINE_HEIGHT))
                .bg(cx.theme().colors.fg.opacity(0.03));
        };
        let file = &self.files[ix];
        let scrolled = self.side_scroll.get(&file.id).copied().unwrap_or_default();
        let syntax = diff_model::syntax_for(&file.path);
        render_line(line, number, syntax, scrolled, cx)
    }

    fn totals(&self) -> (usize, usize) {
        self.files
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
    }
}

/// A file a review lists, before its diff is read.
struct Entry {
    id: String,
    path: String,
    label: String,
    side: Option<Side>,
    source: EntrySource,
    /// Counts from the status or commit, for diffs that cannot be shown.
    counts: (usize, usize),
}

/// Where a file's diff is read from.
enum EntrySource {
    Git(DiffSource),
    /// The before and after a thread's checkpoints hold.
    Session(String),
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Staged => "staged",
        Side::Unstaged => "unstaged",
    }
}

fn side_source(side: Side) -> DiffSource {
    match side {
        Side::Staged => DiffSource::Staged,
        Side::Unstaged => DiffSource::Unstaged,
    }
}

/// MonoCode `workingTreeDiffEntries`: staged first, like the sidebar;
/// `scope` keeps one side. A file on both sides says which one it is.
fn working_tree_entries(cwd: &str, scope: Option<Side>) -> Vec<Entry> {
    let status = git::read_local_state(cwd).status;
    let on = |list: &[git::GitFileChange], path: &str| list.iter().any(|f| f.path == path);
    let mut entries = Vec::new();
    for side in [Side::Staged, Side::Unstaged] {
        if scope.is_some_and(|s| s != side) {
            continue;
        }
        let (list, other) = match side {
            Side::Staged => (&status.staged, &status.unstaged),
            Side::Unstaged => (&status.unstaged, &status.staged),
        };
        for file in list {
            let label = if on(other, &file.path) {
                let which = if side == Side::Staged {
                    "Staged"
                } else {
                    "Unstaged"
                };
                format!("{} ({which})", file.path)
            } else {
                file.path.clone()
            };
            entries.push(Entry {
                id: format!("{}:{}", side_name(side), file.path),
                path: file.path.clone(),
                label,
                side: Some(side),
                source: EntrySource::Git(side_source(side)),
                counts: (file.additions, file.deletions),
            });
        }
    }
    entries
}

fn entries_for(tab: &PaneTab, store: &CheckpointStore) -> Result<(String, Vec<Entry>), String> {
    match tab {
        PaneTab::File { .. } | PaneTab::Browser { .. } => Ok((String::new(), Vec::new())),
        PaneTab::Review { cwd, path, side } => Ok((
            cwd.clone(),
            vec![Entry {
                id: format!("{}:{path}", side_name(*side)),
                path: path.clone(),
                label: path.clone(),
                side: Some(*side),
                source: EntrySource::Git(side_source(*side)),
                counts: (0, 0),
            }],
        )),
        PaneTab::SessionChanges {
            cwd, session_id, ..
        } => {
            let entries = store
                .status(session_id, cwd)?
                .files
                .into_iter()
                .map(|f| Entry {
                    id: f.relative.clone(),
                    label: f.relative.clone(),
                    side: None,
                    source: EntrySource::Session(session_id.clone()),
                    counts: (f.additions, f.deletions),
                    path: f.relative,
                })
                .collect();
            Ok((cwd.clone(), entries))
        }
        PaneTab::Changes { cwd, side, .. } => Ok((cwd.clone(), working_tree_entries(cwd, *side))),
        PaneTab::Commit { cwd, sha, .. } => {
            let files = git::commit_files(cwd, sha).map_err(|e| format!("{e:#}"))?;
            let entries = files
                .into_iter()
                .map(|f| Entry {
                    id: f.path.clone(),
                    label: f.path.clone(),
                    side: None,
                    source: EntrySource::Git(DiffSource::Commit(sha.clone())),
                    counts: (f.additions, f.deletions),
                    path: f.path,
                })
                .collect();
            Ok((cwd.clone(), entries))
        }
    }
}

fn load_file(cwd: &str, entry: Entry, store: &CheckpointStore) -> DocFile {
    let (mut additions, mut deletions) = entry.counts;
    let read = match &entry.source {
        EntrySource::Git(source) => {
            git::file_diff(cwd, &entry.path, source).map_err(|err| format!("{err:#}"))
        }
        EntrySource::Session(session_id) => store.file_diff(session_id, cwd, &entry.path),
    };
    let body = match read {
        Err(err) => {
            log::warn!("could not diff {}: {err}", entry.path);
            FileBody::Failed(err)
        }
        Ok(diff) if diff.binary => FileBody::Binary,
        Ok(diff) if diff.too_large => FileBody::TooLarge,
        Ok(diff) => {
            let file = diff_model::build(diff.lines);
            (additions, deletions) = (file.additions, file.deletions);
            if additions == 0 && deletions == 0 {
                FileBody::Empty(match entry.side {
                    Some(Side::Staged) => "No staged changes",
                    Some(Side::Unstaged) => "No unstaged changes",
                    None => "No textual diff",
                })
            } else {
                FileBody::Diff(file.blocks)
            }
        }
    };
    DocFile {
        id: entry.id,
        path: entry.path,
        label: entry.label,
        side: entry.side,
        additions,
        deletions,
        body,
    }
}

/// Lists the tab's files and reads their diffs, a few at a time.
fn load_doc(tab: &PaneTab, store: &CheckpointStore) -> Result<Vec<DocFile>, String> {
    let (cwd, entries) = entries_for(tab, store)?;
    let mut buckets: Vec<Vec<(usize, Entry)>> = (0..LOAD_THREADS).map(|_| Vec::new()).collect();
    for (ix, entry) in entries.into_iter().enumerate() {
        buckets[ix % LOAD_THREADS].push((ix, entry));
    }
    let mut loaded: Vec<(usize, DocFile)> = std::thread::scope(|scope| {
        let cwd = cwd.as_str();
        let workers: Vec<_> = buckets
            .into_iter()
            .map(|bucket| {
                scope.spawn(move || {
                    bucket
                        .into_iter()
                        .map(|(ix, entry)| (ix, load_file(cwd, entry, store)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| {
                w.join().unwrap_or_else(|_| {
                    log::error!("loading a diff panicked");
                    Vec::new()
                })
            })
            .collect()
    });
    loaded.sort_by_key(|(ix, _)| *ix);
    Ok(loaded.into_iter().map(|(_, file)| file).collect())
}

impl BenCodeApp {
    /// Opens `key`'s review, loading it the first time.
    pub(crate) fn ensure_diff_doc(
        &mut self,
        key: &str,
        focus: Option<DocFocus>,
        cx: &mut Context<Self>,
    ) {
        match self.diff_docs.get_mut(key) {
            Some(doc) => {
                if focus.is_some() {
                    doc.focus = focus;
                    if doc.status == DocStatus::Ready {
                        doc.scroll_to_focus();
                    }
                }
            }
            None => {
                self.diff_docs
                    .insert(key.to_string(), DiffDoc::new(focus, self.diff_split));
                self.load_diff_doc(key, cx);
            }
        }
    }

    /// Re-reads a review; a newer read wins over a slower older one.
    pub(crate) fn load_diff_doc(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.file_pane.get(key).cloned() else {
            return;
        };
        let Some(doc) = self.diff_docs.get_mut(key) else {
            return;
        };
        doc.generation += 1;
        let generation = doc.generation;
        let store = self.checkpoints.store.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load_doc(&tab, &store) });
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                let Some(doc) = app.diff_docs.get_mut(&key) else {
                    return;
                };
                if doc.generation != generation {
                    return;
                }
                match result {
                    Ok(files) => {
                        match changed_files(&doc.files, &files) {
                            None => {
                                // MonoCode `initialExpansion = "all"`.
                                doc.open = files.iter().map(|f| f.id.clone()).collect();
                                doc.reveals.clear();
                                doc.files = files;
                                doc.rebuild();
                            }
                            // The same files: only the changed ones get new
                            // rows, so the others keep their measured heights.
                            Some(changed) => {
                                doc.files = files;
                                for ix in changed {
                                    doc.rebuild_file(ix);
                                }
                            }
                        }
                        doc.status = DocStatus::Ready;
                        doc.scroll_to_focus();
                    }
                    Err(err) => {
                        log::error!("could not load {key}: {err}");
                        doc.status = DocStatus::Failed(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("diff loaded after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The working tree moved: reviews of it read the new state.
    pub(crate) fn reload_working_tree_docs(&mut self, of_cwd: &str, cx: &mut Context<Self>) {
        let keys: Vec<String> = self
            .file_pane
            .entries()
            .iter()
            .filter(|e| match &e.tab {
                PaneTab::Review { cwd, .. } | PaneTab::Changes { cwd, .. } => cwd == of_cwd,
                _ => false,
            })
            .map(|e| e.tab.key())
            .filter(|key| self.diff_docs.contains_key(key))
            .collect();
        for key in keys {
            self.load_diff_doc(&key, cx);
        }
    }

    fn with_doc(&mut self, key: &str, cx: &mut Context<Self>, f: impl FnOnce(&mut DiffDoc)) {
        if let Some(doc) = self.diff_docs.get_mut(key) {
            f(doc);
            cx.notify();
        }
    }

    fn toggle_diff_file(&mut self, key: &str, ix: usize, cx: &mut Context<Self>) {
        self.with_doc(key, cx, |doc| {
            let Some(id) = doc.files.get(ix).map(|f| f.id.clone()) else {
                return;
            };
            if !doc.open.remove(&id) {
                doc.open.insert(id);
            }
            doc.rebuild_file(ix);
        });
    }

    fn set_all_diff_files(&mut self, key: &str, open: bool, cx: &mut Context<Self>) {
        self.with_doc(key, cx, |doc| {
            doc.open = if open {
                doc.files.iter().map(|f| f.id.clone()).collect()
            } else {
                HashSet::new()
            };
            doc.rebuild();
        });
    }

    fn reveal_diff_fold(
        &mut self,
        key: &str,
        ix: usize,
        fold: usize,
        how: Expand,
        cx: &mut Context<Self>,
    ) {
        self.with_doc(key, cx, |doc| {
            let Some(file) = doc.files.get(ix) else {
                return;
            };
            let FileBody::Diff(blocks) = &file.body else {
                return;
            };
            let total = blocks
                .iter()
                .find_map(|b| match b {
                    Block::Fold { id, lines } if *id == fold => Some(lines.len()),
                    _ => None,
                })
                .unwrap_or(0);
            let slot = doc.reveals.entry((file.id.clone(), fold)).or_default();
            *slot = diff_model::expand(*slot, total, how);
            doc.rebuild_file(ix);
        });
    }

    /// Scrolls the lines of file `ix` sideways by `by`, as far as its
    /// longest line reaches.
    fn scroll_diff_file_sideways(
        &mut self,
        key: &str,
        ix: usize,
        by: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.diff_docs.get_mut(key) else {
            return;
        };
        let Some(id) = doc.files.get(ix).map(|file| file.id.clone()) else {
            return;
        };
        let text_system = window.text_system();
        let font = text_system.resolve_font(&gpui::font(cx.theme().mono_family.clone()));
        let advance = match text_system.em_advance(font, px(LINE_TEXT)) {
            Ok(advance) => advance,
            Err(err) => {
                log::debug!("diff: no advance for the code font: {err:#}");
                return;
            }
        };
        // A lane (half the row, side by side) less its `w-12` gutter and
        // the text's `px-3` on each side.
        let rem = window.rem_size();
        let row = doc.list.viewport_bounds().size.width - scrollbar::gutter(&doc.list);
        let lane = if doc.side_by_side() { row / 2.0 } else { row };
        let room = lane - rem * 4.5;
        let reach = (advance * doc.widest_line(ix) as f32 - room).max(Pixels::ZERO);
        let before = doc.side_scroll.get(&id).copied().unwrap_or_default();
        let after = (before + by).clamp(Pixels::ZERO, reach);
        if after != before {
            doc.side_scroll.insert(id, after);
            cx.notify();
        }
    }

    /// The review of the active tab, or its loading / error state.
    pub(crate) fn render_diff_doc(&self, key: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let centered = || {
            div()
                .flex()
                .flex_1()
                .size_full()
                .items_center()
                .justify_center()
        };
        let Some(doc) = self.diff_docs.get(key) else {
            return centered().into_any_element();
        };
        match &doc.status {
            DocStatus::Loading => {
                return centered()
                    .child(crate::ui::git_changes_panel::spinning_icon(
                        SharedString::from(format!("diff-loading-{key}")),
                        IconName::LoaderCircle,
                        IconSize::Sm,
                        fg.opacity(0.4),
                    ))
                    .into_any_element();
            }
            DocStatus::Failed(err) => {
                let what = if key.starts_with("commit:") {
                    "commit"
                } else if key.starts_with("session-changes:") {
                    "session changes"
                } else {
                    "changes"
                };
                return centered()
                    .flex_col()
                    .p_6()
                    .gap_1()
                    .child(
                        Icon::new(IconName::CircleAlert)
                            .size(IconSize::Md)
                            .color(theme.colors.danger),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(px(13.0))
                            .text_color(fg)
                            .child(format!("Couldn’t load {what}")),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.5))
                            .child(err.clone()),
                    )
                    .into_any_element();
            }
            DocStatus::Ready => {}
        }
        if doc.files.is_empty() {
            return div()
                .px_4()
                .py_6()
                .text_size(px(13.0))
                .text_color(fg.opacity(0.45))
                .child("No file changes")
                .into_any_element();
        }
        // The file whose rows are at the top keeps its header pinned there.
        let top = doc.list.logical_scroll_top();
        let pinned = doc
            .rows
            .get(top.item_ix)
            .filter(|row| !matches!(row, DocRow::Header(_)) || top.offset_in_item > px(0.0))
            .map(DocRow::file)
            .filter(|f| doc.open.contains(&doc.files[*f].id));
        let rows_key = key.to_string();
        let gutter = scrollbar::gutter(&doc.list);
        // Whether two lanes fit is known once the rows' box is laid out.
        let probe = {
            let app = cx.entity().downgrade();
            let (key, was) = (key.to_string(), doc.wide);
            canvas(
                move |bounds, _, cx| {
                    let wide = scale::logical(bounds.size.width) >= SPLIT_MIN_WIDTH;
                    if wide == was {
                        return;
                    }
                    cx.defer(move |cx| {
                        let set = app.update(cx, |this, cx| {
                            this.with_doc(&key, cx, |doc| doc.set_wide(wide))
                        });
                        if let Err(err) = set {
                            log::debug!("diff measured after app drop: {err:#}");
                        }
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(self.render_doc_bar(key, doc, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(probe)
                    .child(
                        list(
                            doc.list.clone(),
                            cx.processor(move |this, ix: usize, _, cx| {
                                this.render_doc_row(&rows_key, ix, cx)
                            }),
                        )
                        .size_full()
                        .pr(gutter),
                    )
                    .children(pinned.map(|f| {
                        div()
                            .id(SharedString::from(format!("diff-pinned-box-{key}-{f}")))
                            .absolute()
                            .top_0()
                            .left_0()
                            .right(gutter)
                            .child(self.render_doc_header(key, doc, f, true, cx))
                    }))
                    .child(ScrollBar::new(
                        SharedString::from(format!("diff-scrollbar-{key}")),
                        &doc.list,
                    )),
            )
            .into_any_element()
    }

    /// MonoCode's `h-8` bar: the file count, totals, expand and collapse all.
    fn render_doc_bar(&self, key: &str, doc: &DiffDoc, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let count = doc.files.len();
        let (additions, deletions) = doc.totals();
        let button = |id: &str, icon: Icon, tip: &'static str, enabled: bool| {
            div()
                .id(SharedString::from(format!("{id}-{key}")))
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .when(enabled, |el| {
                    el.cursor_pointer().hover(|s| s.bg(fg.opacity(0.10)))
                })
                .when(!enabled, |el| el.opacity(0.4))
                .tooltip(Tooltip::text(tip))
                .child(icon.size(IconSize::Sm).color(fg.opacity(0.45)))
        };
        let split = doc.split;
        let expand_key = key.to_string();
        let collapse_key = key.to_string();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h_8()
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            .text_size(px(12.0))
            .child(div().text_color(fg.opacity(0.7)).child(if count == 1 {
                "1 file".to_string()
            } else {
                format!("{count} files")
            }))
            .child(
                diff_counts(additions, deletions, crate::ui::appearance::diff_colors(cx))
                    .text_size(px(11.0)),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        button(
                            "diff-layout",
                            Icon::new(if split {
                                IconName::Rows2
                            } else {
                                IconName::Columns2
                            }),
                            match (split, doc.wide) {
                                (false, _) => "Show side by side",
                                (true, true) => "Show unified",
                                (true, false) => "Show unified (side by side needs a wider pane)",
                            },
                            true,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_diff_split(!split, cx)),
                        ),
                    )
                    .child(
                        button(
                            "diff-expand-all",
                            ExtraIcon::UnfoldVertical.icon(),
                            "Expand all files",
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_all_diff_files(&expand_key, true, cx)
                        })),
                    )
                    .child({
                        let enabled = !doc.open.is_empty();
                        button(
                            "diff-collapse-all",
                            ExtraIcon::FoldVertical.icon(),
                            "Collapse all files",
                            enabled,
                        )
                        .when(enabled, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| {
                                this.set_all_diff_files(&collapse_key, false, cx)
                            }))
                        })
                    }),
            )
    }

    fn render_doc_row(&self, key: &str, ix: usize, cx: &Context<Self>) -> AnyElement {
        let Some(doc) = self.diff_docs.get(key) else {
            return div().into_any_element();
        };
        let Some(row) = doc.rows.get(ix) else {
            return div().into_any_element();
        };
        match row {
            DocRow::Header(f) => self
                .render_doc_header(key, doc, *f, false, cx)
                .into_any_element(),
            DocRow::Line(f, line) => doc
                .lane(*f, Some(line), line.number(), cx)
                .w_full()
                .on_scroll_wheel(self.side_scroll_listener(key, *f, cx))
                .into_any_element(),
            // Both lanes scroll sideways together, so a pair stays aligned.
            DocRow::Pair { file, old, new } => {
                let lane = |line: &Option<Line>, number: Option<u32>| {
                    doc.lane(*file, line.as_ref(), number, cx)
                        .flex_1()
                        .min_w_0()
                };
                div()
                    .flex()
                    .w_full()
                    .child(
                        lane(old, old.as_ref().and_then(|line| line.old))
                            .border_r_1()
                            .border_color(cx.theme().colors.border),
                    )
                    .child(lane(new, new.as_ref().and_then(|line| line.new)))
                    .on_scroll_wheel(self.side_scroll_listener(key, *file, cx))
                    .into_any_element()
            }
            DocRow::Fold { file, id, hidden } => self.render_fold(key, *file, *id, *hidden, cx),
            DocRow::Message(_, text) => div()
                .px_3()
                .py_3()
                .text_size(px(12.0))
                .text_color(cx.theme().colors.fg.opacity(0.45))
                .child(text.clone())
                .into_any_element(),
        }
    }

    /// A sideways wheel over file `ix`'s lines scrolls them.
    fn side_scroll_listener(
        &self,
        key: &str,
        ix: usize,
        cx: &Context<Self>,
    ) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut gpui::App) + 'static {
        let key = key.to_string();
        cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
            let delta = event.delta.pixel_delta(window.line_height());
            // An up-and-down gesture is the list's.
            if delta.x.abs() > delta.y.abs() {
                cx.stop_propagation();
                this.scroll_diff_file_sideways(&key, ix, -delta.x, window, cx);
            }
        })
    }

    /// MonoCode `FileSection` header: chevron, icon, path, counts, and on
    /// the unstaged side Discard and Stage.
    fn render_doc_header(
        &self,
        key: &str,
        doc: &DiffDoc,
        ix: usize,
        pinned: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let file = &doc.files[ix];
        let expanded = doc.open.contains(&file.id);
        let busy = self.changes_ui.busy.is_some();
        let toggle_key = key.to_string();
        let name = file
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&file.path)
            .to_string();
        // Stage and Discard act on the open workspace's tree only.
        let live =
            self.file_pane.get(key).and_then(PaneTab::cwd) == Some(self.workspace.cwd.as_str());
        let unstaged = live && file.side == Some(Side::Unstaged);
        let busy_here = matches!(&self.changes_ui.busy, Some(Busy::File(p)) if *p == file.path);
        let prefix = if pinned { "diff-pinned" } else { "diff" };
        div().bg(colors.bg).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .bg(fg.opacity(0.02))
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "{prefix}-file-{key}-{}",
                            file.id
                        )))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .tooltip(Tooltip::text(file.label.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_diff_file(&toggle_key, ix, cx)
                        }))
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.45)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .child(FileIcon::file(&name).size(IconSize::Sm)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_family.clone())
                                .text_size(px(12.0))
                                .text_color(fg.opacity(0.85))
                                .child(file.label.clone()),
                        )
                        .child(
                            diff_counts(
                                file.additions,
                                file.deletions,
                                crate::ui::appearance::diff_colors(cx),
                            )
                            .flex_none()
                            .text_size(px(11.0)),
                        ),
                )
                .when(unstaged, |el| {
                    let discard_path = file.path.clone();
                    let stage_path = file.path.clone();
                    el.child(
                        div()
                            .id(SharedString::from(format!(
                                "{prefix}-discard-{key}-{}",
                                file.id
                            )))
                            .size(px(24.0))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .tooltip(Tooltip::text("Discard file"))
                            .when(busy, |el| el.opacity(0.4))
                            .when(!busy, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.bg(fg.opacity(0.10)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.file_action(
                                            discard_path.clone(),
                                            Side::Unstaged,
                                            true,
                                            cx,
                                        )
                                    }))
                            })
                            .child(
                                Icon::new(IconName::Undo2)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.45)),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "{prefix}-stage-{key}-{}",
                                file.id
                            )))
                            .size(px(16.0))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .bg(fg)
                            .tooltip(Tooltip::text("Stage file"))
                            .when(busy || busy_here, |el| el.opacity(0.4))
                            .when(!busy, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.opacity(0.8))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.file_action(
                                            stage_path.clone(),
                                            Side::Unstaged,
                                            false,
                                            cx,
                                        )
                                    }))
                            })
                            .child(
                                gpui::svg()
                                    .path(IconName::Check.path())
                                    .size(px(10.0))
                                    .text_color(colors.bg),
                            ),
                    )
                }),
        )
    }

    /// MonoCode `FoldBar`: reveal twenty lines up or down, or all of them.
    fn render_fold(
        &self,
        key: &str,
        file: usize,
        fold: usize,
        hidden: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let arrow = |dir: &str, icon: IconName, tip: &'static str, how: Expand| {
            let key = key.to_string();
            div()
                .id(SharedString::from(format!(
                    "fold-{dir}-{key}-{file}-{fold}"
                )))
                .size(px(20.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(fg.opacity(0.10)))
                .tooltip(Tooltip::text(tip))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reveal_diff_fold(&key, file, fold, how, cx)
                }))
                .child(Icon::new(icon).size(IconSize::Xs).color(fg.opacity(0.4)))
        };
        let all_key = key.to_string();
        div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(FOLD_HEIGHT))
            .px_2()
            .bg(fg.opacity(0.08))
            .child(arrow(
                "up",
                IconName::ChevronUp,
                "Expand upward",
                Expand::Up,
            ))
            .child(arrow(
                "down",
                IconName::ChevronDown,
                "Expand downward",
                Expand::Down,
            ))
            .child(
                div()
                    .id(SharedString::from(format!("fold-all-{key}-{file}-{fold}")))
                    .flex_1()
                    .min_w_0()
                    .py_1()
                    .cursor_pointer()
                    .font_family(theme.mono_family.clone())
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .hover(|s| s.text_color(fg.opacity(0.7)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reveal_diff_fold(&all_key, file, fold, Expand::All, cx)
                    }))
                    .child(format!(
                        "{hidden} unmodified {}",
                        if hidden == 1 { "line" } else { "lines" }
                    )),
            )
            .into_any_element()
    }
}

/// MonoCode `opacity-70` on an unchanged line, its colours included.
fn strength(kind: LineKind) -> f32 {
    if kind == LineKind::Context { 0.7 } else { 1.0 }
}

/// A line's syntax colours (MonoCode `renderLineText`), each at `strength`
/// of its own.
fn line_highlights(
    text: &str,
    syntax: Syntax,
    strength: f32,
    cx: &gpui::App,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if text.len() > HIGHLIGHT_MAX_LEN {
        return Vec::new();
    }
    let mut spans: Vec<(Range<usize>, Hsla)> = match syntax {
        Syntax::Plain => return Vec::new(),
        Syntax::Json => json_highlights(text, cx),
        Syntax::Code | Syntax::HashComments => code_highlights(text, cx),
    }
    .into_iter()
    .map(|(range, highlight)| (range, highlight.color))
    .collect();
    if syntax == Syntax::HashComments
        && let Some(at) =
            diff_model::hash_comment_start(text, spans.iter().map(|(range, _)| range.clone()))
    {
        spans.retain(|(range, _)| range.end <= at);
        spans.push((at..text.len(), cx.theme().colors.syntax.comment));
    }
    spans
        .into_iter()
        .map(|(range, color)| {
            (
                range,
                HighlightStyle {
                    color: Some(color.opacity(strength)),
                    ..HighlightStyle::default()
                },
            )
        })
        .collect()
}

/// MonoCode `DiffLineRow`: a tinted gutter number, then the text, in the
/// chosen diff palette (`bg-diff-*-bg`, `bg-diff-*-gutter`, `text-diff-*-fg`).
/// The text starts `scrolled` to the left of its lane; the gutter stays.
/// The caller gives the row its width.
fn render_line(
    line: &Line,
    number: Option<u32>,
    syntax: Syntax,
    scrolled: Pixels,
    cx: &gpui::App,
) -> gpui::Div {
    let theme = cx.theme();
    let colors = &theme.colors;
    let fg = colors.fg;
    let diff = crate::ui::appearance::diff_colors(cx);
    // (row, gutter, number)
    let (tint, number_color): (Option<(Hsla, Hsla)>, Hsla) = match line.kind {
        LineKind::Add => (Some((diff.add_bg, diff.add_gutter)), diff.add_fg),
        LineKind::Del => (Some((diff.del_bg, diff.del_gutter)), diff.del_fg),
        LineKind::Context => (None, fg.opacity(0.35)),
    };
    let text = (!line.text.is_empty()).then(|| {
        StyledText::new(SharedString::from(line.text.clone())).with_highlights(line_highlights(
            &line.text,
            syntax,
            strength(line.kind),
            cx,
        ))
    });
    div()
        .flex()
        .items_center()
        .h(px(LINE_HEIGHT))
        .overflow_hidden()
        .when_some(tint, |el, (row, _)| el.bg(row))
        .font_family(theme.mono_family.clone())
        .child(
            div()
                .w_12()
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .pr_2()
                // The gutter lies on the row's tint; together they make
                // the gutter's own strength.
                .when_some(tint, |el, (row, gutter)| {
                    el.bg(gutter.opacity(1.0 - (1.0 - gutter.a) / (1.0 - row.a)))
                })
                .text_size(px(11.0))
                .text_color(number_color)
                .children(number.map(|n| n.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(LINE_TEXT))
                .text_color(fg.opacity(0.8 * strength(line.kind)))
                .map(|el| match text {
                    None => el.child(" "),
                    // A box of its own only for text moved out of its lane.
                    Some(text) if scrolled > Pixels::ZERO => {
                        el.child(div().ml(-scrolled).child(text))
                    }
                    Some(text) => el.child(text),
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, body: FileBody) -> DocFile {
        DocFile {
            id: id.into(),
            path: id.into(),
            label: id.into(),
            side: None,
            additions: 0,
            deletions: 0,
            body,
        }
    }

    #[test]
    fn a_reload_names_only_the_files_that_changed() {
        let old = [file("a", FileBody::Binary), file("b", FileBody::Empty("x"))];
        let same = old.clone();
        assert_eq!(changed_files(&old, &same), Some(Vec::new()));
        let edited = [file("a", FileBody::Binary), file("b", FileBody::TooLarge)];
        assert_eq!(changed_files(&old, &edited), Some(vec![1]));
    }

    #[test]
    fn another_file_list_is_not_patched() {
        let old = [file("a", FileBody::Binary), file("b", FileBody::Binary)];
        assert_eq!(changed_files(&old, &old[..1]), None);
        let swapped = [old[1].clone(), old[0].clone()];
        assert_eq!(changed_files(&old, &swapped), None);
        assert_eq!(changed_files(&[], &old), None);
    }

    #[test]
    fn entries_name_their_side() {
        assert_eq!(side_name(Side::Staged), "staged");
        assert_eq!(side_source(Side::Unstaged), DiffSource::Unstaged);
    }
}
