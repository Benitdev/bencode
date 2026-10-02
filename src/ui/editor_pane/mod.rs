//! Native code editor: one `CodeEditor` per open file, background reads, atomic saves.

mod external;
pub mod files;
pub mod open_files;
mod view;

use std::collections::HashSet;

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
    _changes: Subscription,
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
}

impl EditorState {
    pub fn is_loading(&self) -> bool {
        !self.loading.is_empty()
    }
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
                let lines = count_lines(editor.read(cx).text());
                this.editor.files.mark_changed(&key, lines);
                cx.notify();
            }
        });
        EditorHandle {
            entity,
            _changes: changes,
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
        let text = file.handle.entity.read(cx).text().to_string();
        let Some(version) = self.editor.files.begin_save(&path) else {
            return;
        };
        let abs_path = resolve_path(&self.workspace.cwd, &path);
        let task = cx
            .background_executor()
            .spawn(async move { atomic_write(&abs_path, &text).map_err(|e| e.to_string()) });
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
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let succeeded = result.is_ok();
        let resave = self.editor.files.finish_save(&path, version, succeeded);
        match result {
            Ok(()) => {
                log::info!("editor: saved {path}");
                self.refresh_workspace(cx);
            }
            Err(err) => {
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
