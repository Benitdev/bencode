//! The Explorer's actions (MonoCode `FileTree.tsx`): listing, the inline
//! create / rename field, delete, cut / copy / paste / duplicate, Finder
//! files, the context menu and the tree's keys. Disk work runs on the
//! background executor; a failure shows above the tree.

use std::path::{Path, PathBuf};

use ely_gpui_component::overlays::ConfirmDialog;
use gpui::{AnyElement, ClipboardEntry, Context, IntoElement};

use super::name::{dirs_touched_by_create, is_within, parent_of, rebase, well_formed};
use super::{Clip, EditState, MenuTarget, TreeEdit, fs};
use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::composer::focus_later;

impl BenCodeApp {
    fn tree_root_path(&self) -> PathBuf {
        PathBuf::from(&self.file_tree.root)
    }

    /// Runs `work` on the tree's root off the UI thread, then `done` with
    /// its result if the root is still shown; a failure is reported.
    fn run_tree_job<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(&Path) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.op_error = None;
        let root = self.file_tree.root.clone();
        let job_root = PathBuf::from(&root);
        let task = cx
            .background_executor()
            .spawn(async move { work(&job_root) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |this, cx| {
                if this.file_tree.root != root {
                    return;
                }
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(err) => {
                        log::warn!("file tree: {err}");
                        this.file_tree.op_error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("file tree job finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Re-lists the root and every open folder; folders that vanished are
    /// forgotten. Cached folders that are collapsed are marked stale and
    /// listed again when they open (see `split_refresh_dirs`).
    pub fn refresh_file_tree(&mut self, cx: &mut Context<Self>) {
        let root = self.file_tree.root.clone();
        if matches!(root.trim(), "" | "~") {
            return;
        }
        let tree = &mut self.file_tree;
        let (dirs, stale) =
            super::split_refresh_dirs(&tree.expanded_paths, tree.dir_cache.keys().cloned());
        tree.stale_dirs.extend(stale);
        tree.loading.extend(dirs.iter().cloned());
        self.list_tree_dirs(dirs, cx);
    }

    /// Lists `dirs` in the background and swaps the results in.
    fn list_tree_dirs(&mut self, dirs: Vec<String>, cx: &mut Context<Self>) {
        let root = self.file_tree.root.clone();
        let list_root = PathBuf::from(&root);
        let task = cx.background_executor().spawn(async move {
            dirs.into_iter()
                .map(|dir| {
                    let listed = fs::list_dir(&list_root, &dir);
                    (dir, listed)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let listed = task.await;
            let updated = this.update(cx, |this, cx| {
                let tree = &mut this.file_tree;
                if tree.root != root {
                    return;
                }
                for (dir, result) in listed {
                    tree.loading.remove(&dir);
                    tree.stale_dirs.remove(&dir);
                    match result {
                        Ok(entries) => {
                            tree.dir_errors.remove(&dir);
                            tree.dir_cache.insert(dir, entries);
                        }
                        // An open folder shows why; a cached one that is
                        // gone is just forgotten.
                        Err(err) if dir.is_empty() || tree.expanded_paths.contains(&dir) => {
                            tree.dir_cache.remove(&dir);
                            tree.dir_errors.insert(dir, err);
                        }
                        Err(_) => {
                            tree.dir_cache.remove(&dir);
                        }
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("file listing finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub fn toggle_folder_expanded(&mut self, rel: &str, cx: &mut Context<Self>) {
        let tree = &mut self.file_tree;
        if !tree.expanded_paths.remove(rel) {
            tree.expanded_paths.insert(rel.to_string());
            // A stale listing keeps showing while the fresh one loads.
            if tree.needs_listing(rel) && tree.loading.insert(rel.to_string()) {
                self.list_tree_dirs(vec![rel.to_string()], cx);
            }
        }
        cx.notify();
    }

    /// Opens `dirs` (and lists any not cached yet).
    fn expand_tree_dirs(&mut self, dirs: &[String], cx: &mut Context<Self>) {
        let mut missing = Vec::new();
        for dir in dirs {
            if dir.is_empty() {
                self.file_tree.root_collapsed = false;
                continue;
            }
            self.file_tree.expanded_paths.insert(dir.clone());
            if self.file_tree.needs_listing(dir) && self.file_tree.loading.insert(dir.clone()) {
                missing.push(dir.clone());
            }
        }
        if !missing.is_empty() {
            self.list_tree_dirs(missing, cx);
        }
    }

    /// Re-lists folders an operation changed, forgetting moved-away ones.
    fn refresh_touched(&mut self, touched: Vec<String>, forget: &[String], cx: &mut Context<Self>) {
        for gone in forget {
            self.file_tree
                .dir_cache
                .retain(|dir, _| !is_within(dir, gone));
        }
        let mut dirs = touched;
        dirs.sort();
        dirs.dedup();
        self.list_tree_dirs(dirs, cx);
        self.refresh_workspace(cx);
    }

    /// MonoCode "Collapse All": only the root stays open; any edit ends.
    pub fn collapse_all_folders(&mut self, cx: &mut Context<Self>) {
        self.file_tree.edit = None;
        self.file_tree.expanded_paths.clear();
        self.file_tree.root_collapsed = false;
        cx.notify();
    }

    fn begin_tree_edit(&mut self, edit: TreeEdit, text: String, cx: &mut Context<Self>) {
        self.file_tree.menu = None;
        self.file_tree.edit = Some(edit);
        self.file_tree.edit_state = EditState::default();
        // MonoCode selects a file's stem, or the whole name.
        let stem = match text.rfind('.') {
            Some(dot) if dot > 0 => dot,
            _ => text.len(),
        };
        self.file_dialog_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.select(0..stem, cx);
        });
        let handle = gpui::Focusable::focus_handle(self.file_dialog_input.read(cx), cx);
        focus_later(handle, cx);
        cx.notify();
    }

    /// MonoCode `startCreate`: a field in the selected folder (or the
    /// selected file's folder), opened.
    pub fn start_tree_create(&mut self, is_dir: bool, at: Option<String>, cx: &mut Context<Self>) {
        let at = at.or_else(|| self.file_tree.selected_path.clone());
        let parent = self.file_tree.create_parent_of(at.as_deref());
        self.expand_tree_dirs(&[String::new(), parent.clone()], cx);
        self.begin_tree_edit(TreeEdit::Create { parent, is_dir }, String::new(), cx);
    }

    /// MonoCode `startRename` (not the root).
    pub fn start_tree_rename(&mut self, path: &str, cx: &mut Context<Self>) {
        if path.is_empty() {
            return;
        }
        let is_dir = self.file_tree.is_dir(path);
        let current = Path::new(path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.file_tree.selected_path = Some(path.to_string());
        self.begin_tree_edit(
            TreeEdit::Rename {
                path: path.to_string(),
                is_dir,
            },
            current,
            cx,
        );
    }

    pub fn tree_edit_active(&self) -> bool {
        self.file_tree.edit.is_some()
    }

    pub fn cancel_tree_edit(&mut self, cx: &mut Context<Self>) {
        if self.file_tree.edit.take().is_some() {
            self.file_tree.edit_state = EditState::default();
            cx.notify();
        }
    }

    /// Enter commits; blur commits unless the name is wrong (MonoCode
    /// `NameRow.finish`). An error keeps the field open.
    pub fn commit_tree_edit(&mut self, from_blur: bool, cx: &mut Context<Self>) {
        let Some(edit) = self.file_tree.edit.clone() else {
            return;
        };
        if self.file_tree.edit_state.busy {
            return;
        }
        let raw = self.file_dialog_input.read(cx).text().to_string();
        if let Some(issue) = self.tree_name_issue(&raw)
            && issue.is_error()
        {
            if from_blur {
                self.cancel_tree_edit(cx);
            } else {
                self.file_tree.edit_state.attempted = true;
                cx.notify();
            }
            return;
        }
        match edit {
            TreeEdit::Create { parent, is_dir } => self.create_tree_entry(parent, is_dir, raw, cx),
            TreeEdit::Rename { path, is_dir } => self.rename_tree_entry(path, is_dir, raw, cx),
        }
    }

    /// Runs an inline edit's disk work; a failure stays on the field.
    fn run_tree_edit<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(&Path) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.edit_state.busy = true;
        self.file_tree.edit_state.submit_error = None;
        let root = self.file_tree.root.clone();
        let job_root = PathBuf::from(&root);
        let task = cx
            .background_executor()
            .spawn(async move { work(&job_root) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let updated = this.update(cx, |this, cx| {
                if this.file_tree.root != root {
                    return;
                }
                this.file_tree.edit_state.busy = false;
                match result {
                    Ok(value) => {
                        this.file_tree.edit = None;
                        this.file_tree.edit_state = EditState::default();
                        done(this, value, cx);
                    }
                    Err(err) => this.file_tree.edit_state.submit_error = Some(err),
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("file tree edit finished after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode `onCreateCommit`: a trailing `/` makes a folder; nested
    /// folders open; a new file opens in the editor.
    fn create_tree_entry(
        &mut self,
        parent: String,
        is_dir: bool,
        raw: String,
        cx: &mut Context<Self>,
    ) {
        let as_folder = is_dir || raw.ends_with(['/', '\\']);
        let file_name = well_formed(&raw);
        let touched = dirs_touched_by_create(&parent, &file_name);
        let work_parent = parent.clone();
        self.run_tree_edit(
            move |root| fs::create(root, &work_parent, &file_name, as_folder),
            move |this, created, cx| {
                this.refresh_touched(touched.clone(), &[], cx);
                this.expand_tree_dirs(&touched, cx);
                this.file_tree.selected_path = Some(created.clone());
                if !as_folder {
                    this.open_tree_file(created, cx);
                }
            },
            cx,
        );
    }

    /// Opens a file the tree just made; the editor needs the window, so
    /// the next frame opens it.
    fn open_tree_file(&mut self, rel: String, cx: &mut Context<Self>) {
        self.file_tree.pending_open = Some(rel);
        cx.notify();
    }

    /// MonoCode `onRenameCommit`: the tree and open editors follow.
    fn rename_tree_entry(
        &mut self,
        path: String,
        is_dir: bool,
        raw: String,
        cx: &mut Context<Self>,
    ) {
        let file_name = well_formed(&raw);
        let unchanged = Path::new(&path).file_name().and_then(|n| n.to_str())
            == Some(file_name.as_str())
            && !raw.contains(['/', '\\']);
        if file_name.is_empty() || unchanged {
            self.cancel_tree_edit(cx);
            return;
        }
        let parent = parent_of(&path);
        let mut touched = dirs_touched_by_create(&parent, &file_name);
        touched.push(parent.clone());
        let from = path.clone();
        self.run_tree_edit(
            move |root| fs::rename(root, &from, &file_name),
            move |this, next, cx| {
                let forget = if is_dir {
                    vec![path.clone()]
                } else {
                    Vec::new()
                };
                this.refresh_touched(touched.clone(), &forget, cx);
                this.expand_tree_dirs(&touched, cx);
                this.remap_tree_paths(&path, &next, cx);
                this.on_tree_entry_moved(&path, &next, cx);
            },
            cx,
        );
    }

    /// MonoCode `remapTreePaths`; open folders that moved are listed at
    /// their new place.
    fn remap_tree_paths(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let tree = &mut self.file_tree;
        tree.expanded_paths = tree
            .expanded_paths
            .iter()
            .map(|p| rebase(p, from, to))
            .collect();
        let moved: Vec<String> = tree
            .expanded_paths
            .iter()
            .filter(|p| is_within(p, to) && !tree.dir_cache.contains_key(*p))
            .cloned()
            .collect();
        tree.loading.extend(moved.iter().cloned());
        if !moved.is_empty() {
            self.list_tree_dirs(moved, cx);
        }
        let tree = &mut self.file_tree;
        if let Some(selected) = tree.selected_path.as_mut() {
            *selected = rebase(selected, from, to);
        }
        if let Some(clip) = tree.clip.as_mut() {
            clip.path = rebase(&clip.path, from, to);
        }
    }

    /// Open editors follow a rename or move: saved tabs reopen at the new
    /// path; a tab with unsaved edits stays where it is.
    fn on_tree_entry_moved(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let active = self.editor.files.active_path().map(str::to_string);
        let moved: Vec<String> = self
            .editor
            .files
            .iter()
            .filter(|f| is_within(&f.path, from) && !f.is_dirty())
            .map(|f| f.path.clone())
            .collect();
        for old in &moved {
            self.close_editor_file(old, cx);
        }
        let reopen_active = active
            .filter(|a| moved.contains(a))
            .map(|a| rebase(&a, from, to));
        if let Some(path) = reopen_active {
            self.open_tree_file(path, cx);
        }
    }

    /// MonoCode `onFileDeleted`: tabs inside the deleted path close.
    fn on_tree_entry_deleted(&mut self, path: &str, cx: &mut Context<Self>) {
        let gone: Vec<String> = self
            .editor
            .files
            .iter()
            .filter(|f| is_within(&f.path, path))
            .map(|f| f.path.clone())
            .collect();
        for old in gone {
            self.close_editor_file(&old, cx);
        }
    }

    /// Asks before deleting (MonoCode `removeEntry`'s confirm).
    pub fn request_tree_delete(&mut self, path: &str, cx: &mut Context<Self>) {
        if path.is_empty() {
            return;
        }
        let is_dir = self.file_tree.is_dir(path);
        self.file_tree.pending_delete = Some((path.to_string(), is_dir));
        cx.notify();
    }

    pub(super) fn delete_tree_entry(&mut self, path: String, is_dir: bool, cx: &mut Context<Self>) {
        let target = path.clone();
        self.run_tree_job(
            move |root| fs::delete(root, &target),
            move |this, (), cx| {
                let parent = parent_of(&path);
                let forget = if is_dir {
                    vec![path.clone()]
                } else {
                    Vec::new()
                };
                this.refresh_touched(vec![parent.clone()], &forget, cx);
                let tree = &mut this.file_tree;
                if tree
                    .selected_path
                    .as_deref()
                    .is_none_or(|s| is_within(s, &path))
                {
                    tree.selected_path = Some(parent);
                }
                if tree
                    .clip
                    .as_ref()
                    .is_some_and(|c| is_within(&c.path, &path))
                {
                    tree.clip = None;
                }
                tree.expanded_paths.retain(|p| !is_within(p, &path));
                this.on_tree_entry_deleted(&path, cx);
            },
            cx,
        );
    }

    /// MonoCode `pasteAt`: the clipped entry, else files copied in Finder.
    pub fn paste_in_tree(&mut self, target: &str, cx: &mut Context<Self>) {
        let dest = self.file_tree.create_parent_of(Some(target));
        let Some(clip) = self.file_tree.clip.clone() else {
            let paths: Vec<PathBuf> = cx
                .read_from_clipboard()
                .map(|item| {
                    item.entries()
                        .iter()
                        .filter_map(|entry| match entry {
                            ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
                            _ => None,
                        })
                        .flatten()
                        .collect()
                })
                .unwrap_or_default();
            self.drop_external_files(paths, &dest, cx);
            return;
        };
        if clip.is_dir && is_within(&dest, &clip.path) {
            self.file_tree.op_error = Some("Cannot paste a folder into itself.".into());
            cx.notify();
            return;
        }
        let from = clip.path.clone();
        let work_dest = dest.clone();
        self.run_tree_job(
            move |root| {
                if clip.cut {
                    fs::move_into(root, &from, &work_dest)
                } else {
                    fs::copy_into(root, &root.join(&from), &work_dest)
                }
            },
            move |this, created, cx| {
                if clip.cut {
                    let mut touched = vec![parent_of(&clip.path), parent_of(&created)];
                    touched.dedup();
                    let forget = if clip.is_dir {
                        vec![clip.path.clone()]
                    } else {
                        Vec::new()
                    };
                    this.refresh_touched(touched, &forget, cx);
                    this.remap_tree_paths(&clip.path, &created, cx);
                    this.on_tree_entry_moved(&clip.path, &created, cx);
                    this.file_tree.clip = None;
                } else {
                    this.refresh_touched(vec![dest.clone()], &[], cx);
                }
                this.expand_tree_dirs(&[dest.clone()], cx);
                this.file_tree.selected_path = Some(created);
            },
            cx,
        );
    }

    /// MonoCode `duplicateAt`.
    pub fn duplicate_in_tree(&mut self, path: &str, cx: &mut Context<Self>) {
        if path.is_empty() {
            return;
        }
        let dest = parent_of(path);
        let from = path.to_string();
        let work_dest = dest.clone();
        self.run_tree_job(
            move |root| fs::copy_into(root, &root.join(&from), &work_dest),
            move |this, created, cx| {
                this.refresh_touched(vec![dest], &[], cx);
                this.file_tree.selected_path = Some(created);
            },
            cx,
        );
    }

    /// MonoCode `copyExternalFiles`: Finder files copied into `dest`, the
    /// last one selected.
    pub fn drop_external_files(&mut self, paths: Vec<PathBuf>, dest: &str, cx: &mut Context<Self>) {
        self.file_tree.drop_target = None;
        if paths.is_empty() {
            cx.notify();
            return;
        }
        let work_dest = dest.to_string();
        let dest = dest.to_string();
        self.run_tree_job(
            move |root| {
                let mut created = None;
                for from in &paths {
                    created = Some(fs::copy_into(root, from, &work_dest)?);
                }
                created.ok_or_else(|| "Nothing to paste".to_string())
            },
            move |this, created, cx| {
                this.refresh_touched(vec![dest.clone()], &[], cx);
                this.expand_tree_dirs(&[dest], cx);
                this.file_tree.selected_path = Some(created);
            },
            cx,
        );
    }

    pub(super) fn tree_abs_path(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.tree_root_path()
        } else {
            self.tree_root_path().join(rel)
        }
    }

    pub(super) fn reveal_tree_path(&mut self, rel: &str, cx: &mut Context<Self>) {
        cx.reveal_path(&self.tree_abs_path(rel));
    }

    fn tree_target_for_keys(&self) -> MenuTarget {
        let path = self.file_tree.selected_path.clone().unwrap_or_default();
        MenuTarget {
            is_dir: self.file_tree.is_dir(&path),
            path,
        }
    }

    pub fn tree_copy_path(&mut self, cx: &mut Context<Self>) {
        let target = self.tree_target_for_keys();
        let text = self
            .tree_abs_path(&target.path)
            .to_string_lossy()
            .into_owned();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    /// ⌘C / ⌘X on the selection (not the root).
    pub fn tree_clip(&mut self, cut: bool, cx: &mut Context<Self>) {
        let target = self.tree_target_for_keys();
        if target.is_root() {
            return;
        }
        self.file_tree.clip = Some(Clip {
            cut,
            path: target.path,
            is_dir: target.is_dir,
        });
        cx.notify();
    }

    pub fn tree_paste_selected(&mut self, cx: &mut Context<Self>) {
        let target = self.tree_target_for_keys();
        self.paste_in_tree(&target.path, cx);
    }

    pub fn tree_rename_selected(&mut self, cx: &mut Context<Self>) {
        if self.tree_edit_active() {
            return;
        }
        let target = self.tree_target_for_keys();
        self.start_tree_rename(&target.path, cx);
    }

    pub fn tree_delete_selected(&mut self, cx: &mut Context<Self>) {
        let target = self.tree_target_for_keys();
        self.request_tree_delete(&target.path, cx);
    }

    /// Esc drops a cut (MonoCode).
    pub fn tree_clear_cut(&mut self, cx: &mut Context<Self>) {
        if self.file_tree.clip.as_ref().is_some_and(|c| c.cut) {
            self.file_tree.clip = None;
            cx.notify();
        }
    }

    /// The pending "Delete …?" (MonoCode's confirm text).
    pub fn render_file_tree_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (path, is_dir) = self.file_tree.pending_delete.clone()?;
        let label = Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let message = if is_dir {
            format!("Delete folder “{label}” and everything inside it?")
        } else {
            format!("Delete “{label}”?")
        };
        let close = app_callback(cx, |this, cx| {
            this.file_tree.pending_delete = None;
            cx.notify();
        });
        let confirm = app_callback(cx, move |this, cx| {
            this.file_tree.pending_delete = None;
            this.delete_tree_entry(path.clone(), is_dir, cx);
        });
        Some(
            ConfirmDialog::new("dialog-delete", "Delete", message, close)
                .confirm("Delete")
                .destructive()
                .on_confirm(confirm)
                .into_any_element(),
        )
    }
}
