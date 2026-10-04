//! Native code editor: one `CodeEditor` per open file, background reads, atomic saves.

mod disk_sync;
mod external;
pub mod files;
pub mod open_files;
mod view;

use std::collections::{HashMap, HashSet};

use ely_gpui_component::editor::{CodeEditor, EditorEvent, LineNumbers};
use gpui::{Context, Entity, SharedString, Subscription, Window, prelude::*};

use crate::app::{BenCodeApp, ViewMode};
pub use files::detect_language;
use files::{
    MAX_EDITOR_FILE_BYTES, ReadError, atomic_write, count_lines, file_name, read_text_file,
    resolve_path,
};
use open_files::OpenFiles;

/// A live editor and the subscription that tracks its edits.
pub struct EditorHandle {
    pub entity: Entity<CodeEditor>,
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
    requested: Option<String>,
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
    fn show_editor_notice(&mut self, title: String, body: String, cx: &mut Context<Self>) {
        self.editor.notice = Some(EditorNotice {
            title: title.into(),
            body: body.into(),
        });
        cx.notify();
    }

    /// Opens a workspace-relative or absolute file, reading it off the UI thread.
    pub fn open_file_in_editor(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.active_view_mode = ViewMode::Editor;
        let path = path.to_string();
        self.editor.requested = Some(path.clone());
        if self.editor.files.activate(&path) || !self.editor.loading.insert(path.clone()) {
            cx.notify();
            return;
        }
        let abs_path = resolve_path(&self.workspace.cwd, &path);
        let task = cx
            .background_executor()
            .spawn(async move { read_text_file(&abs_path, MAX_EDITOR_FILE_BYTES) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let opened = this.update_in(cx, |this, window, cx| {
                this.finish_open(path, result, window, cx);
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
                let title = format!("Could not open {}", file_name(&path));
                self.show_editor_notice(title, err.to_string(), cx);
            }
            Ok(text) => {
                let lines = count_lines(&text);
                let handle = self.build_editor(&path, &text, window, cx);
                self.editor.files.insert(path.clone(), handle, lines);
                if requested {
                    self.editor.files.activate(&path);
                }
                cx.notify();
            }
        }
    }

    fn build_editor(
        &mut self,
        path: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EditorHandle {
        let entity = cx.new(|cx| {
            CodeEditor::new(text, window, cx)
                .language(detect_language(path))
                .line_numbers(LineNumbers::Absolute)
                .minimap()
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
                cx.notify();
            }
        });
        // Only a change in the selected lines redraws the app.
        let selection_key = path.to_string();
        let selection = cx.observe(&entity, move |this, editor, cx| {
            let editor = editor.read(cx);
            let primary = editor.primary();
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
        });
        EditorHandle {
            entity,
            disk_hash: disk_sync::content_hash(text),
            _changes: changes,
            _selection: selection,
        }
    }

    /// Switches the active tab to an already open file; its edits stay in memory.
    pub fn switch_editor_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.editor.requested = Some(path.to_string());
        if self.editor.files.activate(path) {
            cx.notify();
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

    /// Closes the active tab (Cmd-W); false when no file is open.
    pub fn request_close_active_editor_file(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.editor.files.active_path().map(str::to_string) else {
            return false;
        };
        self.request_close_editor_file(&path, cx);
        true
    }

    /// Closes a tab without asking, dropping its unsaved edits.
    pub fn close_editor_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.editor.pending_close.as_deref() == Some(path) {
            self.editor.pending_close = None;
        }
        if self.editor.files.remove(path).is_some() {
            log::info!("editor: closed {path}");
        }
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
        let Some(version) = self.editor.files.begin_save(&path) else {
            return;
        };
        let abs_path = resolve_path(&self.workspace.cwd, &path);
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
    use super::selected_lines;

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
