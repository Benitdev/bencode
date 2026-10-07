//! MonoCode `UnifiedDiffView` (with `WorkingTreeDiff` and `CommitDiff` as
//! its loaders): every file of a review stacked under a header that sticks
//! to the top, unchanged runs folded behind "N unmodified lines" bars.
//! Headers and rows share one `gpui::list`; git runs only in the loader.

use std::collections::{HashMap, HashSet};

use ely_gpui_component::files::FileIcon;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ListAlignment, ListOffset,
    ListState, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, list,
    prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::ui::scrollbar::{self, ScrollBar};
use crate::app::file_pane::PaneTab;
use crate::git::checkpoint::CheckpointStore;
use crate::git::{self, DiffSource};
use crate::ui::diff_counts::diff_counts;
use crate::ui::diff_model::{self, Block, BodyRow, Expand, Line, LineKind, Reveal};
use crate::ui::git_changes_panel::{Busy, Side};
use crate::ui::icons::ExtraIcon;

/// MonoCode `UNIFIED_LINE_PX` / `UNIFIED_FOLD_PX`.
const LINE_HEIGHT: f32 = 20.0;
const FOLD_HEIGHT: f32 = 32.0;
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
    Fold { file: usize, id: usize, hidden: usize },
    Message(usize, SharedString),
}

impl DocRow {
    fn file(&self) -> usize {
        match self {
            Self::Header(f) | Self::Line(f, _) | Self::Message(f, _) => *f,
            Self::Fold { file, .. } => *file,
        }
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
    focus: Option<DocFocus>,
    generation: u64,
}

impl DiffDoc {
    fn new(focus: Option<DocFocus>) -> Self {
        Self {
            files: Vec::new(),
            status: DocStatus::Loading,
            open: HashSet::new(),
            reveals: HashMap::new(),
            rows: Vec::new(),
            starts: vec![0],
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
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
                rows.extend(diff_model::body_rows(blocks, reveal_for).into_iter().map(
                    |row| match row {
                        BodyRow::Line(line) => DocRow::Line(ix, line),
                        BodyRow::Fold { id, hidden } => DocRow::Fold {
                            file: ix,
                            id,
                            hidden,
                        },
                    },
                ));
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
                let which = if side == Side::Staged { "Staged" } else { "Unstaged" };
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
        PaneTab::File { .. } => Ok((String::new(), Vec::new())),
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
        PaneTab::SessionChanges { cwd, session_id, .. } => {
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
    pub(crate) fn ensure_diff_doc(&mut self, key: &str, focus: Option<DocFocus>, cx: &mut Context<Self>) {
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
                self.diff_docs.insert(key.to_string(), DiffDoc::new(focus));
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
                        let same = doc.files.len() == files.len()
                            && doc.files.iter().zip(&files).all(|(a, b)| a.id == b.id);
                        if !same {
                            // MonoCode `initialExpansion = "all"`.
                            doc.open = files.iter().map(|f| f.id.clone()).collect();
                            doc.reveals.clear();
                        }
                        doc.files = files;
                        doc.status = DocStatus::Ready;
                        doc.rebuild();
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

    fn reveal_diff_fold(&mut self, key: &str, ix: usize, fold: usize, how: Expand, cx: &mut Context<Self>) {
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

    /// The review of the active tab, or its loading / error state.
    pub(crate) fn render_diff_doc(&self, key: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let centered = || div().flex().flex_1().size_full().items_center().justify_center();
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
                    .child(Icon::new(IconName::CircleAlert).size(IconSize::Md).color(theme.colors.danger))
                    .child(div().mt_2().text_size(px(13.0)).text_color(fg).child(format!("Couldn’t load {what}")))
                    .child(div().text_size(px(12.0)).text_color(fg.opacity(0.5)).child(err.clone()))
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
        let gutter = scrollbar::gutter();
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
        let button = |id: &str, icon: ExtraIcon, tip: &'static str, enabled: bool| {
            div()
                .id(SharedString::from(format!("{id}-{key}")))
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .when(enabled, |el| el.cursor_pointer().hover(|s| s.bg(fg.opacity(0.10))))
                .when(!enabled, |el| el.opacity(0.4))
                .tooltip(Tooltip::text(tip))
                .child(icon.icon().size(IconSize::Sm).color(fg.opacity(0.45)))
        };
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
            .child(
                div()
                    .text_color(fg.opacity(0.7))
                    .child(if count == 1 { "1 file".to_string() } else { format!("{count} files") }),
            )
            .child(diff_counts(additions, deletions, colors).text_size(px(11.0)))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        button("diff-expand-all", ExtraIcon::UnfoldVertical, "Expand all files", true)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_all_diff_files(&expand_key, true, cx)
                            })),
                    )
                    .child({
                        let enabled = !doc.open.is_empty();
                        button("diff-collapse-all", ExtraIcon::FoldVertical, "Collapse all files", enabled)
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
            DocRow::Header(f) => self.render_doc_header(key, doc, *f, false, cx).into_any_element(),
            DocRow::Line(_, line) => render_line(line, cx),
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

    /// MonoCode `FileSection` header: chevron, icon, path, counts, and on
    /// the unstaged side Discard and Stage.
    fn render_doc_header(&self, key: &str, doc: &DiffDoc, ix: usize, pinned: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let file = &doc.files[ix];
        let expanded = doc.open.contains(&file.id);
        let busy = self.changes_ui.busy.is_some();
        let toggle_key = key.to_string();
        let name = file.path.rsplit('/').next().unwrap_or(&file.path).to_string();
        // Stage and Discard act on the open workspace's tree only.
        let live = self.file_pane.get(key).and_then(PaneTab::cwd) == Some(self.workspace.cwd.as_str());
        let unstaged = live && file.side == Some(Side::Unstaged);
        let busy_here = matches!(&self.changes_ui.busy, Some(Busy::File(p)) if *p == file.path);
        let prefix = if pinned { "diff-pinned" } else { "diff" };
        div()
            .bg(colors.bg)
            .child(
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
                            .id(SharedString::from(format!("{prefix}-file-{key}-{}", file.id)))
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
                                Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.45)),
                            )
                            .child(div().flex_none().child(FileIcon::file(&name).size(IconSize::Sm)))
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
                                diff_counts(file.additions, file.deletions, colors)
                                    .flex_none()
                                    .text_size(px(11.0)),
                            ),
                    )
                    .when(unstaged, |el| {
                        let discard_path = file.path.clone();
                        let stage_path = file.path.clone();
                        el.child(
                            div()
                                .id(SharedString::from(format!("{prefix}-discard-{key}-{}", file.id)))
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
                                            this.file_action(discard_path.clone(), Side::Unstaged, true, cx)
                                        }))
                                })
                                .child(Icon::new(IconName::Undo2).size(IconSize::Xs).color(fg.opacity(0.45))),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("{prefix}-stage-{key}-{}", file.id)))
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
                                            this.file_action(stage_path.clone(), Side::Unstaged, false, cx)
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
    fn render_fold(&self, key: &str, file: usize, fold: usize, hidden: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let arrow = |dir: &str, icon: IconName, tip: &'static str, how: Expand| {
            let key = key.to_string();
            div()
                .id(SharedString::from(format!("fold-{dir}-{key}-{file}-{fold}")))
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
            .child(arrow("up", IconName::ChevronUp, "Expand upward", Expand::Up))
            .child(arrow("down", IconName::ChevronDown, "Expand downward", Expand::Down))
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

/// MonoCode `DiffLineRow`: a tinted gutter number, then the text.
fn render_line(line: &Line, cx: &gpui::App) -> AnyElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    let fg = colors.fg;
    let (tint, number_color): (Option<Hsla>, Hsla) = match line.kind {
        LineKind::Add => (Some(colors.success), colors.success),
        LineKind::Del => (Some(colors.danger), colors.danger),
        LineKind::Context => (None, fg.opacity(0.35)),
    };
    div()
        .flex()
        .items_center()
        .h(px(LINE_HEIGHT))
        .w_full()
        .overflow_hidden()
        .when_some(tint, |el, tint| el.bg(tint.opacity(0.15)))
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
                .when_some(tint, |el, tint| el.bg(tint.opacity(0.10)))
                .text_size(px(11.0))
                .text_color(number_color)
                .children(line.number().map(|n| n.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(12.0))
                .text_color(fg.opacity(if line.kind == LineKind::Context { 0.56 } else { 0.8 }))
                .when(line.text.is_empty(), |el| el.child(" "))
                .when(!line.text.is_empty(), |el| {
                    el.child(SharedString::from(line.text.clone()))
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_name_their_side() {
        assert_eq!(side_name(Side::Staged), "staged");
        assert_eq!(side_source(Side::Unstaged), DiffSource::Unstaged);
    }
}
