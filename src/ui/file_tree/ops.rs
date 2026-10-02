//! File tree mutations: create, rename and delete, and the dialogs that ask
//! for them. Nothing here overwrites an existing entry, and every failure is
//! shown above the tree (MonoCode `FileTree.tsx` `NameRow` and `opError`).

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use ely_gpui_component::overlays::{ConfirmDialog, PromptDialog};
use gpui::{AnyElement, App, Context, IntoElement, SharedString, Window};

use super::FileDialogAction;
use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

/// Why `name` cannot be used for a new or renamed entry, if it cannot.
/// `taken` holds the names already in the target folder.
pub(super) fn name_problem(name: &str, taken: &[String]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some("A file or folder name must be provided.".to_string());
    }
    if name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Some(format!(
            "The name {name} is not valid as a file or folder name. Please choose a different name."
        ));
    }
    taken.iter().any(|t| t == name).then(|| {
        format!(
            "A file or folder {name} already exists at this location. Please choose a different name."
        )
    })
}

/// Creates an empty file, failing if anything already exists at `path`.
fn create_file_at(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

/// Creates a folder, failing if anything already exists at `path`.
fn create_dir_at(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path)
}

/// Renames without replacing: `fs::rename` silently overwrites on Unix.
/// A case-only rename of the same entry is allowed.
fn rename_at(from: &Path, to: &Path) -> io::Result<()> {
    let same_entry = fs::canonicalize(from).ok() == fs::canonicalize(to).ok();
    if to.symlink_metadata().is_ok() && !same_entry {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a file or folder with that name already exists",
        ));
    }
    fs::rename(from, to)
}

fn delete_at(path: &Path, is_dir: bool) -> io::Result<()> {
    if is_dir {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_of(rel: &str) -> String {
    Path::new(rel)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("")
        .to_string()
}

impl BenCodeApp {
    /// Names in a loaded folder, for the dialogs' conflict check.
    fn names_in(&self, rel_dir: &str, except: Option<&str>) -> Vec<String> {
        self.file_tree
            .dir_cache
            .get(rel_dir)
            .into_iter()
            .flatten()
            .map(|e| e.name.clone())
            .filter(|n| Some(n.as_str()) != except)
            .collect()
    }

    fn full_path(&self, rel: &str) -> PathBuf {
        Path::new(&self.workspace_cwd()).join(rel)
    }

    /// Reloads `parent` after an operation and reports a failure above the
    /// tree. Returns whether the operation succeeded.
    fn finish_tree_op(
        &mut self,
        what: &str,
        parent: &str,
        result: io::Result<()>,
        cx: &mut Context<Self>,
    ) -> bool {
        let ok = match result {
            Ok(()) => {
                if !parent.is_empty() {
                    self.file_tree.expanded_paths.insert(parent.to_string());
                }
                self.refresh_workspace(cx);
                true
            }
            Err(err) => {
                log::error!("could not {what}: {err}");
                self.file_tree.op_error = Some(format!("Could not {what}: {err}"));
                false
            }
        };
        self.load_directory(parent, cx);
        cx.notify();
        ok
    }

    /// Runs `op` off the UI thread, then `finish_tree_op`.
    fn run_tree_op(
        &mut self,
        what: &'static str,
        parent: String,
        op: impl FnOnce() -> io::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.op_error = None;
        let task = cx.background_executor().spawn(async move { op() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |this, cx| {
                this.finish_tree_op(what, &parent, result, cx);
            });
            if let Err(err) = updated {
                log::debug!("file tree op finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub fn create_file(
        &mut self,
        parent_dir: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.op_error = None;
        let rel = join_rel(parent_dir, name.trim());
        let full = self.full_path(&rel);
        let parent = parent_dir.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { create_file_at(&full) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let updated = this.update_in(cx, |this, window, cx| {
                if this.finish_tree_op("create the file", &parent, result, cx) {
                    this.file_tree.selected_path = Some(rel.clone());
                    this.open_file_in_editor(&rel, window, cx);
                }
            });
            if let Err(err) = updated {
                log::debug!("file create finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub fn create_folder(&mut self, parent_dir: &str, name: &str, cx: &mut Context<Self>) {
        let full = self.full_path(&join_rel(parent_dir, name.trim()));
        let op = move || create_dir_at(&full);
        self.run_tree_op("create the folder", parent_dir.to_string(), op, cx);
    }

    pub fn rename_entry(&mut self, target_path: &str, new_name: &str, cx: &mut Context<Self>) {
        let parent = parent_of(target_path);
        let from = self.full_path(target_path);
        let to = self.full_path(&join_rel(&parent, new_name.trim()));
        self.run_tree_op("rename", parent, move || rename_at(&from, &to), cx);
    }

    pub fn delete_entry(&mut self, target_path: &str, is_dir: bool, cx: &mut Context<Self>) {
        let full = self.full_path(target_path);
        let op = move || delete_at(&full, is_dir);
        self.run_tree_op("delete", parent_of(target_path), op, cx);
    }

    pub fn render_file_tree_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let action = self.file_tree.dialog.clone()?;
        let close = app_callback(cx, |this, cx| {
            this.file_tree.dialog = None;
            cx.notify();
        });
        let (id, title, label, submit_label, parent, current) = match &action {
            FileDialogAction::Delete {
                target_path,
                is_dir,
            } => return Some(self.delete_dialog(target_path, *is_dir, close, cx)),
            FileDialogAction::NewFile { parent_dir } => (
                "dialog-new-file",
                "New File",
                "File Name",
                "Create",
                parent_dir.clone(),
                None,
            ),
            FileDialogAction::NewFolder { parent_dir } => (
                "dialog-new-folder",
                "New Folder",
                "Folder Name",
                "Create",
                parent_dir.clone(),
                None,
            ),
            FileDialogAction::Rename {
                target_path,
                is_dir,
            } => (
                "dialog-rename",
                if *is_dir {
                    "Rename Folder"
                } else {
                    "Rename File"
                },
                "New Name",
                "Rename",
                parent_of(target_path),
                Path::new(target_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string),
            ),
        };
        let taken = self.names_in(&parent, current.as_deref());
        let submit = cx.listener(move |this, name: &str, window, cx| {
            this.file_tree.dialog = None;
            match &action {
                FileDialogAction::NewFile { parent_dir } => {
                    this.create_file(parent_dir, name, window, cx)
                }
                FileDialogAction::NewFolder { parent_dir } => {
                    this.create_folder(parent_dir, name, cx)
                }
                FileDialogAction::Rename { target_path, .. } => {
                    this.rename_entry(target_path, name, cx)
                }
                FileDialogAction::Delete { .. } => {}
            }
            cx.notify();
        });
        Some(
            PromptDialog::new(id, title, &self.file_dialog_input, close)
                .label(label)
                .submit(submit_label)
                .check(move |text| name_problem(text, &taken).map_or(Ok(()), |m| Err(m.into())))
                .on_submit(move |raw: &str, window: &mut Window, cx: &mut App| {
                    submit(raw, window, cx)
                })
                .into_any_element(),
        )
    }

    fn delete_dialog(
        &self,
        target_path: &str,
        is_dir: bool,
        close: impl Fn(&mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let name = Path::new(target_path).file_name().map_or_else(
            || target_path.to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let message: SharedString = if is_dir {
            format!("Delete folder \"{name}\" and everything inside it?").into()
        } else {
            format!("Delete \"{name}\"?").into()
        };
        let target = target_path.to_string();
        let on_confirm = app_callback(cx, move |this, cx| {
            this.file_tree.dialog = None;
            this.delete_entry(&target, is_dir, cx);
        });
        ConfirmDialog::new("dialog-delete", "Delete", message, close)
            .confirm("Delete")
            .destructive()
            .on_confirm(on_confirm)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bencode-ops-{tag}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn name_problem_rejects_empty_invalid_and_taken_names() {
        let taken = vec!["main.rs".to_string()];
        assert!(name_problem("  ", &taken).is_some());
        assert!(name_problem("..", &taken).is_some());
        assert!(name_problem("a/b", &taken).is_some());
        assert!(
            name_problem("main.rs", &taken)
                .unwrap()
                .contains("already exists")
        );
        assert_eq!(name_problem("lib.rs", &taken), None);
    }

    #[test]
    fn create_never_truncates_an_existing_file() {
        let dir = temp_dir("create");
        let file = dir.join("keep.txt");
        fs::write(&file, "precious").unwrap();
        assert!(create_file_at(&file).is_err());
        assert!(create_dir_at(&file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "precious");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_never_replaces_another_entry() {
        let dir = temp_dir("rename");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        assert!(rename_at(&a, &b).is_err());
        assert_eq!(fs::read_to_string(&b).unwrap(), "b");
        rename_at(&a, &dir.join("c.txt")).unwrap();
        assert!(dir.join("c.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
