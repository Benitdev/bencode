//! Keeping open files in step with the disk (MonoCode `FileEditor.tsx`
//! `watchFile`): a clean buffer reloads when the file changes underneath it;
//! a buffer with unsaved edits gets a "changed on disk" choice, and saving
//! over a newer disk version is refused until the user picks one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::feedback::Alert;
use ely_gpui_component::primitives::Severity;
use gpui::{Context, IntoElement, ParentElement, Styled, div};

use super::files::{MAX_EDITOR_FILE_BYTES, count_lines, file_name, read_text_file, resolve_path};
use crate::app::BenCodeApp;

/// Fingerprint of file contents, to tell "changed on disk" apart.
pub fn content_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl BenCodeApp {
    /// Re-reads every open file off the UI thread and reacts to changes.
    pub fn recheck_open_files_on_disk(&mut self, cx: &mut Context<Self>) {
        let files: Vec<(String, std::path::PathBuf, u64)> = self
            .editor
            .files
            .iter()
            .map(|f| {
                let abs = resolve_path(&self.workspace.cwd, &f.path);
                (f.path.clone(), abs, f.handle.disk_hash)
            })
            .collect();
        if files.is_empty() {
            return;
        }
        let task = cx.background_executor().spawn(async move {
            files
                .into_iter()
                .filter_map(|(path, abs, known)| {
                    let text = read_text_file(&abs, MAX_EDITOR_FILE_BYTES).ok()?;
                    (content_hash(&text) != known).then_some((path, text))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let changed = task.await;
            if changed.is_empty() {
                return;
            }
            let applied = this.update(cx, |this, cx| {
                for (path, text) in changed {
                    this.on_disk_changed(path, text, cx);
                }
            });
            if let Err(err) = applied {
                log::debug!("disk check after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn on_disk_changed(&mut self, path: String, text: String, cx: &mut Context<Self>) {
        if self.editor.files.is_dirty(&path) {
            self.editor.disk_conflicts.insert(path, text);
            cx.notify();
        } else {
            self.reload_from_disk(&path, text, cx);
        }
    }

    /// Replaces the buffer with the disk version and marks it clean.
    pub fn reload_from_disk(&mut self, path: &str, text: String, cx: &mut Context<Self>) {
        let Some(file) = self.editor.files.get(path) else {
            return;
        };
        let entity = file.handle.entity.clone();
        self.editor.reloading.insert(path.to_string());
        let lines = count_lines(&text);
        let hash = content_hash(&text);
        entity.update(cx, |editor, cx| editor.set_text(text, cx));
        self.editor.files.mark_reloaded(path, lines);
        if let Some(file) = self.editor.files.get_handle_mut(path) {
            file.disk_hash = hash;
        }
        self.editor.disk_conflicts.remove(path);
        cx.notify();
    }

    /// Keeps the unsaved edits; the next save may overwrite the disk.
    pub fn keep_local_edits(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(text) = self.editor.disk_conflicts.remove(path)
            && let Some(file) = self.editor.files.get_handle_mut(path)
        {
            file.disk_hash = content_hash(&text);
        }
        cx.notify();
    }

    /// "File changed on disk" for the active file, with Reload / Keep mine.
    pub(super) fn render_disk_conflict(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let path = self.editor.files.active_path()?.to_string();
        let disk_text = self.editor.disk_conflicts.get(&path)?.clone();
        let (reload_path, keep_path) = (path.clone(), path.clone());
        let reload = Button::new("disk-reload", "Reload")
            .variant(ButtonVariant::Secondary)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.reload_from_disk(&reload_path, disk_text.clone(), cx)
            }));
        let keep = Button::new("disk-keep", "Keep mine")
            .variant(ButtonVariant::Ghost)
            .on_click(cx.listener(move |this, _, _, cx| this.keep_local_edits(&keep_path, cx)));
        Some(
            div().p_2().child(
                Alert::new(
                    "editor-disk-conflict",
                    Severity::Warning,
                    format!("{} changed on disk", file_name(&path)),
                )
                .body("It was edited outside this editor while you had unsaved changes.")
                .action(div().flex().gap_2().child(reload).child(keep)),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_refuses_to_overwrite_a_newer_disk_copy() {
        use super::super::{SaveError, save_unless_changed};
        let dir = std::env::temp_dir().join(format!("bencode-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "agent edit").unwrap();

        let stale = content_hash("original");
        let refused = save_unless_changed(&file, "mine", stale);
        assert!(matches!(refused, Err(SaveError::ChangedOnDisk(ref t)) if t == "agent edit"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "agent edit");

        let current = content_hash("agent edit");
        assert!(save_unless_changed(&file, "mine", current).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hash_tells_contents_apart() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("a\n"));
    }
}
