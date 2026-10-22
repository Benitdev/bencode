//! Native code editor: one `CodeEditor` per open file, background reads, atomic saves.

mod blame;
mod disk_sync;
mod external;
pub mod files;
pub mod open_files;
mod preview;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ely_gpui_component::editor::{CodeEditor, EditorEvent, LineNumbers};
use gpui::{Context, Entity, SharedString, Subscription, Window, prelude::*};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::app::project_files::resolve_reference;
pub use files::detect_language;
use files::{
    MAX_EDITOR_FILE_BYTES, ReadError, atomic_write, count_lines, file_name, read_text_file,
    relative_to, resolve_path,
};
use open_files::OpenFiles;
pub use preview::FileViewMode;

/// A live editor and the subscription that tracks its edits.
pub struct EditorHandle {
    pub entity: Entity<CodeEditor>,
    /// Where the file is, fixed when it opened: its tab keeps the path
    /// relative to the project it opened in, which may no longer be the
    /// workspace's.
    pub abs_path: PathBuf,
    /// `content_hash` of the file as last read from or written to disk.
    pub disk_hash: u64,
    _changes: Subscription,
    _selection: Subscription,
}

/// The lines selected in an open file (from one), for "Add to chat".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSelection {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// The primary selection's first and last line (from one); a selection
/// ending at the start of a line does not take that line. `None` for a bare
/// caret.
pub fn selected_lines(text: &str, anchor: usize, head: usize) -> Option<(usize, usize)> {
    if anchor == head {
        return None;
    }
    let (from, to) = (anchor.min(head), anchor.max(head));
    let before = |at: usize| text.get(..at.min(text.len()));
    let line = |at: usize| before(at).map_or(0, |t| t.matches('\n').count()) + 1;
    let ends_on_break = before(to).is_some_and(|t| t.ends_with('\n'));
    let end = if ends_on_break {
        line(to) - 1
    } else {
        line(to)
    };
    Some((line(from), end.max(line(from))))
}

/// An error shown above the editor until dismissed.
#[derive(Clone, Debug)]
pub struct EditorNotice {
    pub title: SharedString,
    pub body: SharedString,
}

/// All editor-pane state owned by `BenCodeApp`.
#[derive(Default)]
pub struct EditorState {
    pub files: OpenFiles<EditorHandle>,
    /// The path the user asked for last; a slower read never steals focus from it.
    pub(crate) requested: Option<String>,
    loading: HashSet<String>,
    pub notice: Option<EditorNotice>,
    /// A dirty file waiting on "Discard changes?".
    pub pending_close: Option<String>,
    /// Dirty files whose disk copy changed underneath, with the disk text.
    pub disk_conflicts: HashMap<String, String>,
    /// Files whose next `Changed` event is a reload, not an edit.
    reloading: HashSet<String>,
    /// The active file's selected lines, if any.
    pub selection: Option<EditorSelection>,
    /// Where to put the cursor once this file is open (search results).
    reveal: Option<Reveal>,
    /// Markdown files' own Preview / Source choice.
    view_modes: HashMap<String, FileViewMode>,
    /// What a markdown file opens in: the last choice made (`fileViewMode`).
    pub(crate) default_view_mode: FileViewMode,
    /// Previews whose frontmatter "Properties" is unfolded.
    properties_open: HashSet<String>,
}

/// A spot in a file: line and byte column from one, and how many bytes
/// to select from there.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reveal {
    path: String,
    line: u32,
    column: u32,
    len: usize,
}

/// The byte range of `reveal` in `text`, kept inside the line and on
/// character boundaries.
fn reveal_range(text: &str, line: u32, column: u32, len: usize) -> std::ops::Range<usize> {
    let start_of_line = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1) as usize)
        .map(str::len)
        .sum::<usize>();
    let line_text = text[start_of_line..].split('\n').next().unwrap_or_default();
    let floor = |at: usize| {
        let mut at = at.min(line_text.len());
        while !line_text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let from = floor(column.saturating_sub(1) as usize);
    let to = floor(from + len);
    start_of_line + from..start_of_line + to
}

impl EditorState {
    pub fn is_loading(&self) -> bool {
        !self.loading.is_empty()
    }
}

/// Why a save did not happen.
enum SaveError {
    /// The file on disk is no longer what the buffer was loaded from.
    ChangedOnDisk(String),
    Io(String),
}

/// Writes `text` unless the disk copy changed since it was last seen
/// (`known` hash); returns the hash of what was written.
fn save_unless_changed(path: &std::path::Path, text: &str, known: u64) -> Result<u64, SaveError> {
    if let Ok(current) = read_text_file(path, MAX_EDITOR_FILE_BYTES)
        && disk_sync::content_hash(&current) != known
    {
        return Err(SaveError::ChangedOnDisk(current));
    }
    atomic_write(path, text).map_err(|e| SaveError::Io(e.to_string()))?;
    Ok(disk_sync::content_hash(text))
}

impl BenCodeApp {
    pub(crate) fn show_editor_notice(
        &mut self,
        title: String,
        body: String,
        cx: &mut Context<Self>,
    ) {
        self.editor.notice = Some(EditorNotice {
            title: title.into(),
            body: body.into(),
        });
        cx.notify();
    }

    /// Opens a file with the cursor at `line`:`column` (from one) and
    /// `len` bytes from there selected.
    pub fn open_file_at(
        &mut self,
        path: &str,
        line: u32,
        column: u32,
        len: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.reveal = Some(Reveal {
            path: path.to_string(),
            line,
            column,
            len,
        });
        // MonoCode: a navigation into a previewable file shows its source.
        if preview::is_markdown_path(path) {
            self.editor
                .view_modes
                .insert(path.to_string(), FileViewMode::Source);
        }
        self.open_file_in_editor(path, window, cx);
        // Already open: the editor is there to move now.
        self.apply_reveal(path, cx);
    }

    /// Opens a file a reply names (`` `usage.rs:42` ``, a link), at `line`
    /// when given. Agents shorten paths (a bare name, a path from a
    /// subfolder): when the path as written is not in the project, its
    /// index (or a listing, before there is one) picks the file meant, off
    /// the UI thread.
    pub fn open_file_reference(
        &mut self,
        path: &str,
        line: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.workspace(cx).cwd.clone();
        let reference = relative_to(&cwd, path);
        let absolute = Path::new(&reference).is_absolute() || crate::remote::is_remote(&reference);
        let index = &self.project_files;
        let indexed = (index.root == cwd && !index.files.is_empty()).then(|| index.files.clone());
        let listed = indexed.is_some() && index.contains(&reference);
        if absolute || listed || cwd.trim().is_empty() {
            self.open_reference_at(&reference, line, window, cx);
            return;
        }
        let open: HashSet<String> = self.editor.files.iter().map(|f| f.path.clone()).collect();
        let task = cx.background_executor().spawn(async move {
            // A file the index has not caught up with yet.
            if resolve_path(&cwd, &reference).exists() {
                return reference;
            }
            let files: Arc<[SharedString]> = indexed.unwrap_or_else(|| {
                crate::workspace::list_project_files(Path::new(&cwd))
                    .into_iter()
                    .map(SharedString::from)
                    .collect()
            });
            resolve_reference(&reference, &files, |f| open.contains(f))
                .map_or(reference.clone(), str::to_string)
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = task.await;
            let opened = this.update_in(cx, |this, window, cx| {
                this.open_reference_at(&path, line, window, cx);
            });
            if let Err(err) = opened {
                log::debug!("file reference resolved after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn open_reference_at(
        &mut self,
        path: &str,
        line: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match line
            .and_then(|line| u32::try_from(line).ok())
            .filter(|&line| line > 0)
        {
            Some(line) => self.open_file_at(path, line, 1, 0, window, cx),
            None => self.open_file_in_editor(path, window, cx),
        }
    }

    /// Moves `path`'s editor to the pending reveal, if it is for this file.
    fn apply_reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.editor.reveal.as_ref().is_none_or(|r| r.path != path) {
            return;
        }
        let Some(entity) = self.editor.files.get(path).map(|f| f.handle.entity.clone()) else {
            return;
        };
        let Some(reveal) = self.editor.reveal.take() else {
            return;
        };
        entity.update(cx, |editor, cx| {
            let range = reveal_range(editor.text(), reveal.line, reveal.column, reveal.len);
            editor.select([range], cx);
        });
    }

    /// Opens a workspace-relative or absolute file, reading it off the UI thread.
    pub fn open_file_in_editor(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.surface_shown_session(None, cx);
        // The file pane is under the Mono's view: a file its chat names
        // opens in the workspace.
        if self.leave_mono_view(cx) {
            self.sync_selection(cx);
        }
        let path = path.to_string();
        self.file_pane
            .open(PaneTab::File { path: path.clone() }, true);
        self.file_pane_focused = true;
        self.editor.requested = Some(path.clone());
        if self.editor.files.activate(&path) || !self.editor.loading.insert(path.clone()) {
            cx.notify();
            return;
        }
        let abs_path = resolve_path(&self.workspace(cx).cwd, &path);
        let read_path = abs_path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { read_text_file(&read_path, MAX_EDITOR_FILE_BYTES) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let opened = this.update_in(cx, |this, window, cx| {
                this.finish_open(path, abs_path, result, window, cx);
            });
            if let Err(err) = opened {
                log::warn!("editor closed before file finished loading: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_open(
        &mut self,
        path: String,
        abs_path: PathBuf,
        result: Result<String, ReadError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.loading.remove(&path);
        let requested = self.editor.requested.as_deref() == Some(path.as_str());
        match result {
            Err(err) => {
                log::warn!("could not open {path} in editor: {err}");
                if requested {
                    self.editor.requested = None;
                }
                self.drop_pane_tab(&PaneTab::File { path: path.clone() }.key(), cx);
                let title = format!("Could not open {}", file_name(&path));
                self.show_editor_notice(title, err.to_string(), cx);
            }
            Ok(text) => {
                let lines = count_lines(&text);
                let handle = self.build_editor(&path, abs_path, &text, window, cx);
                self.editor.files.insert(path.clone(), handle, lines);
                if requested {
                    self.editor.files.activate(&path);
                }
                self.apply_reveal(&path, cx);
                cx.notify();
            }
        }
    }

    fn build_editor(
        &mut self,
        path: &str,
        abs_path: PathBuf,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EditorHandle {
        let entity = cx.new(|cx| {
            CodeEditor::new(text, window, cx)
                .language(detect_language(path))
                .line_numbers(LineNumbers::Absolute)
                // No minimap: Ely colours the whole file and paints a quad
                // per token on every render (about 3ms a frame at 1,600
                // lines), and the editor renders whenever the app does.
                .sticky_scroll()
                .rainbow_brackets()
        });
        let key = path.to_string();
        let changes = cx.subscribe(&entity, move |this, editor, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Changed) {
                if this.editor.reloading.remove(&key) {
                    return;
                }
                let lines = count_lines(editor.read(cx).text());
                this.editor.files.mark_changed(&key, lines);
                // The tab's unsaved mark and the line count are the file
                // pane's alone.
                this.redraw_regions(&[crate::ui::region::Region::FilePane], cx);
            }
        });
        // Only a change in the selected lines redraws the app.
        let selection_key = path.to_string();
        let selection = cx.observe_in(&entity, window, move |this, editor, window, cx| {
            let editor = editor.read(cx);
            let primary = editor.primary();
            let head = primary.head;
            let next = selected_lines(editor.text(), primary.anchor, primary.head).map(
                |(start_line, end_line)| EditorSelection {
                    path: selection_key.clone(),
                    start_line,
                    end_line,
                },
            );
            let mine = this
                .editor
                .selection
                .as_ref()
                .is_none_or(|s| s.path == selection_key);
            if mine && this.editor.selection != next {
                this.editor.selection = next;
                cx.notify();
            }
            this.track_line_blame(&selection_key, cx);
            // A ⌘-click placed the caret on the name to go to.
            if this.definition.jump_on_click.as_deref() == Some(selection_key.as_str()) {
                this.definition.jump_on_click = None;
                this.go_to_definition(&selection_key, head, window, cx);
            }
        });
        EditorHandle {
            entity,
            abs_path,
            disk_hash: disk_sync::content_hash(text),
            _changes: changes,
            _selection: selection,
        }
    }

    /// Closes a tab, asking first when it holds unsaved edits.
    pub fn request_close_editor_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.editor.files.is_dirty(path) {
            self.editor.pending_close = Some(path.to_string());
            cx.notify();
        } else {
            self.close_editor_file(path, cx);
        }
    }

    /// Closes a tab without asking, dropping its unsaved edits.
    pub fn close_editor_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.editor.pending_close.as_deref() == Some(path) {
            self.editor.pending_close = None;
        }
        if self.editor.files.remove(path).is_some() {
            log::info!("editor: closed {path}");
        }
        self.editor.view_modes.remove(path);
        self.editor.properties_open.remove(path);
        self.forget_file_blame(path);
        self.drop_pane_tab(
            &PaneTab::File {
                path: path.to_string(),
            }
            .key(),
            cx,
        );
        cx.notify();
    }

    /// Saves the active file (Cmd-S).
    pub fn save_current_editor_file(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.editor.files.active_path().map(str::to_string) {
            self.save_editor_file(path, cx);
        }
    }

    fn save_editor_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(file) = self.editor.files.get(&path) else {
            return;
        };
        if !file.is_dirty() {
            return;
        }
        if self.editor.disk_conflicts.contains_key(&path) {
            // Reload or Keep mine first; never overwrite a newer disk copy.
            return;
        }
        let text = file.handle.entity.read(cx).text().to_string();
        let known = file.handle.disk_hash;
        let abs_path = file.handle.abs_path.clone();
        let Some(version) = self.editor.files.begin_save(&path) else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { save_unless_changed(&abs_path, &text, known) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let saved = this.update(cx, |this, cx| this.finish_save(path, version, result, cx));
            if let Err(err) = saved {
                log::warn!("editor closed before save finished: {err:#}");
            }
        })
        .detach();
    }

    fn finish_save(
        &mut self,
        path: String,
        version: u64,
        result: Result<u64, SaveError>,
        cx: &mut Context<Self>,
    ) {
        let succeeded = result.is_ok();
        let resave = self.editor.files.finish_save(&path, version, succeeded);
        match result {
            Ok(hash) => {
                log::info!("editor: saved {path}");
                if let Some(handle) = self.editor.files.get_handle_mut(&path) {
                    handle.disk_hash = hash;
                }
                // The saved copy is blamed afresh.
                self.track_line_blame(&path, cx);
                self.refresh_workspace(cx);
            }
            Err(SaveError::ChangedOnDisk(disk_text)) => {
                log::warn!("not saving {path}: it changed on disk");
                self.editor.disk_conflicts.insert(path.clone(), disk_text);
            }
            Err(SaveError::Io(err)) => {
                log::error!("failed to save {path}: {err}");
                let title = format!("Could not save {}", file_name(&path));
                self.show_editor_notice(title, err, cx);
            }
        }
        if resave {
            self.save_editor_file(path, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{reveal_range, selected_lines};

    #[test]
    fn reveal_range_stays_on_its_line() {
        let text = "ab\nconst néedle = 1;\nend";
        assert_eq!(&text[reveal_range(text, 2, 7, 7)], "néedle");
        // Past the line's end, and inside a character.
        assert_eq!(reveal_range(text, 3, 9, 4), 25..25);
        assert_eq!(&text[reveal_range(text, 2, 9, 1)], "");
    }

    #[test]
    fn selections_name_their_lines() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(selected_lines(text, 2, 2), None);
        assert_eq!(selected_lines(text, 0, 2), Some((1, 1)));
        assert_eq!(selected_lines(text, 9, 1), Some((1, 3)));
        // Ending at the start of line 3 takes lines 1-2 only.
        assert_eq!(selected_lines(text, 0, 8), Some((1, 2)));
    }
}
