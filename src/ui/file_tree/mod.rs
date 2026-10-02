//! Files sidebar: native file tree matching MonoCode's Explorer tab.

mod ops;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ely_gpui_component::menus::{ContextMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, Hsla, IntoElement, ParentElement, SharedString,
    Styled, div, px, rgb,
};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

/// A filesystem entry (directory or file) in the workspace tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub ignored: bool,
}

/// Action awaiting user input in a modal/prompt dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogAction {
    NewFile { parent_dir: String },
    NewFolder { parent_dir: String },
    Rename { target_path: String, is_dir: bool },
    Delete { target_path: String, is_dir: bool },
}

/// Persistent state of the file explorer tree.
pub struct FileTreeState {
    pub expanded_paths: HashSet<String>,
    pub selected_path: Option<String>,
    pub root_expanded: bool,
    pub dir_cache: HashMap<String, Vec<FsEntry>>,
    pub dialog: Option<FileDialogAction>,
    /// Last failed create/rename/delete, shown above the tree.
    pub op_error: Option<String>,
}

impl Default for FileTreeState {
    fn default() -> Self {
        Self {
            expanded_paths: HashSet::new(),
            selected_path: None,
            root_expanded: true,
            dir_cache: HashMap::new(),
            dialog: None,
            op_error: None,
        }
    }
}

/// Reads the directory entries for `rel_dir` (or root if empty) from `root`.
/// Filters out system junk (.DS_Store) and marks standard ignored dirs (.git, node_modules, target, etc.).
pub fn read_dir_entries(root: &Path, rel_dir: &str) -> Vec<FsEntry> {
    let dir_path = if rel_dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel_dir)
    };

    let entries = match std::fs::read_dir(&dir_path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name == ".DS_Store" || name == "Thumbs.db" {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let rel_path = if rel_dir.is_empty() {
            name.to_string()
        } else {
            format!("{rel_dir}/{name}")
        };

        let ignored = name == ".git"
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "build";

        out.push(FsEntry {
            name: name.to_string(),
            rel_path,
            is_dir,
            ignored,
        });
    }

    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });

    out
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Resolves the icon and color matching MonoCode's Material Icon Theme palette.
pub fn resolve_entry_icon(name: &str, is_dir: bool, is_open: bool) -> (IconName, Hsla) {
    let lower = name.to_lowercase();
    if is_dir {
        if lower == ".agents" {
            return (IconName::Bot, rgb(0xf87171).into());
        }
        if lower == ".claude" {
            return (IconName::Sparkles, rgb(0xf97316).into());
        }
        if lower == ".github" {
            return (IconName::GitBranch, rgb(0xa855f7).into());
        }
        if is_open {
            return (IconName::FolderOpen, rgb(0x60a5fa).into());
        }
        return (IconName::Folder, rgb(0x60a5fa).into());
    }

    if lower == ".gitignore" || lower == ".gitmodules" || lower == ".gitattributes" {
        return (IconName::GitBranch, rgb(0xf97316).into());
    }
    if lower == "readme.md" || lower == "readme" {
        return (IconName::Info, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        return (IconName::FileText, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".json") {
        return (IconName::FileJson, rgb(0xfacc15).into());
    }
    if lower.ends_with(".lock") {
        return (IconName::Lock, rgb(0xa1a1aa).into());
    }
    if lower.ends_with(".rs") {
        return (IconName::FileCode, rgb(0xf97316).into());
    }
    if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        return (IconName::FileCode, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".js") || lower.ends_with(".jsx") {
        return (IconName::FileCode, rgb(0xfacc15).into());
    }
    if lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower == ".env"
    {
        return (IconName::FileCog, rgb(0xeab308).into());
    }
    if lower.ends_with(".sh")
        || lower.ends_with(".bash")
        || lower.ends_with(".zsh")
        || lower == "artisan"
    {
        return (IconName::FileTerminal, rgb(0x4ade80).into());
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".svg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
    {
        return (IconName::FileImage, rgb(0xc084fc).into());
    }

    (IconName::File, rgb(0x94a3b8).into())
}

impl BenCodeApp {
    /// Opens a repo-relative `path` in the native code editor.
    #[allow(dead_code)]
    pub fn select_tree_file(
        &mut self,
        path: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_in_editor(path, window, cx);
    }

    /// Renders the complete Explorer tab matching MonoCode 1:1.
    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Trigger root loading if not yet cached
        if !self.file_tree.dir_cache.contains_key("") {
            self.refresh_file_tree(cx);
        }

        let theme = cx.theme();
        let cwd = self.workspace_cwd();

        if cwd.is_empty() || cwd == "~" {
            return div().flex().flex_col().flex_1().min_h_0().child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(12.0))
                    .text_color(theme.colors.fg_muted)
                    .child("No project folder"),
            );
        }

        let root_name = Path::new(&cwd).file_name().map_or_else(
            || "WORKSPACE".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        // 1. Toolbar (4 action buttons evenly distributed across the row)
        let toolbar = div()
            .flex()
            .h(px(36.0))
            .items_center()
            .gap(px(2.0))
            .px_2()
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .id("tree-toolbar-new-file")
                    .flex_1()
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_pointer()
                    .text_color(theme.colors.fg_muted)
                    .hover(|s| s.bg(theme.colors.hover).text_color(theme.colors.fg))
                    .tooltip(Tooltip::text("New File"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let target_dir = this.selected_folder_target();
                        this.prompt_new_file(target_dir, cx);
                    }))
                    .child(Icon::new(IconName::FilePlus).size(IconSize::Xs)),
            )
            .child(
                div()
                    .id("tree-toolbar-new-folder")
                    .flex_1()
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_pointer()
                    .text_color(theme.colors.fg_muted)
                    .hover(|s| s.bg(theme.colors.hover).text_color(theme.colors.fg))
                    .tooltip(Tooltip::text("New Folder"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let target_dir = this.selected_folder_target();
                        this.prompt_new_folder(target_dir, cx);
                    }))
                    .child(Icon::new(IconName::FolderPlus).size(IconSize::Xs)),
            )
            .child(
                div()
                    .id("tree-toolbar-collapse-all")
                    .flex_1()
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_pointer()
                    .text_color(theme.colors.fg_muted)
                    .hover(|s| s.bg(theme.colors.hover).text_color(theme.colors.fg))
                    .tooltip(Tooltip::text("Collapse All"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.collapse_all_folders(cx);
                    }))
                    .child(Icon::new(IconName::ChevronsUpDown).size(IconSize::Xs)),
            )
            .child(
                div()
                    .id("tree-toolbar-search")
                    .flex_1()
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_pointer()
                    .text_color(theme.colors.fg_muted)
                    .hover(|s| s.bg(theme.colors.hover).text_color(theme.colors.fg))
                    .tooltip(Tooltip::text("Search in files (Cmd+Shift+F)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_search_modal(cx);
                    }))
                    .child(Icon::new(IconName::Search).size(IconSize::Xs)),
            );

        // 2. Root folder row
        let root_open = self.file_tree.root_expanded;
        let root_row = div()
            .id("explorer-root-header")
            .flex()
            .h(px(32.0))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap(px(4.0))
            .pl(px(8.0))
            .pr(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.colors.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.file_tree.root_expanded = !this.file_tree.root_expanded;
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(if root_open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Xs)
                        .color(theme.colors.fg_muted),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.fg_muted)
                    .child(root_name.to_uppercase()),
            );

        // 3. Tree items list
        let mut rows: Vec<AnyElement> = Vec::new();
        if root_open {
            self.collect_dir_rows("", 0, cx, &mut rows);
        }

        let tree_content = div()
            .id("explorer-tree-scroll-pane")
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .overflow_x_hidden()
            .children(rows);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .when_some(self.file_tree.op_error.clone(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(12.0))
                        .text_color(theme.colors.danger)
                        .child(error),
                )
            })
            .child(root_row)
            .child(tree_content)
    }

    fn selected_folder_target(&self) -> String {
        match &self.file_tree.selected_path {
            Some(path) => {
                let cwd = self.workspace_cwd();
                let full = Path::new(&cwd).join(path);
                if full.is_dir() {
                    path.clone()
                } else {
                    Path::new(path)
                        .parent()
                        .map_or("", |p| p.to_str().unwrap_or(""))
                        .to_string()
                }
            }
            None => String::new(),
        }
    }

    fn collect_dir_rows(
        &self,
        rel_dir: &str,
        depth: usize,
        cx: &Context<Self>,
        out: &mut Vec<AnyElement>,
    ) {
        if let Some(entries) = self.file_tree.dir_cache.get(rel_dir) {
            for entry in entries {
                if entry.ignored {
                    continue;
                }
                out.push(self.render_tree_entry(entry, depth, cx));
                if entry.is_dir && self.file_tree.expanded_paths.contains(&entry.rel_path) {
                    self.collect_dir_rows(&entry.rel_path, depth + 1, cx, out);
                }
            }
        }
    }

    fn render_tree_entry(&self, entry: &FsEntry, depth: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let is_selected = self.file_tree.selected_path.as_deref() == Some(&entry.rel_path);
        let is_open = entry.is_dir && self.file_tree.expanded_paths.contains(&entry.rel_path);
        let (icon, icon_color) = resolve_entry_icon(&entry.name, entry.is_dir, is_open);
        let name_color = self.entry_color(&entry.rel_path, entry.is_dir, theme);
        let rel_path = entry.rel_path.clone();
        let is_dir = entry.is_dir;

        let pad_left = 8.0 + depth as f32 * 12.0;

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "tree-item-{}",
                rel_path
            ))))
            .flex()
            .h(px(28.0))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap(px(6.0))
            .pl(px(pad_left))
            .pr(px(8.0))
            .cursor_pointer()
            .when(is_selected, |el| el.bg(theme.colors.active))
            .hover(|s| s.bg(theme.colors.hover))
            .on_click(cx.listener({
                let rel_path = rel_path.clone();
                move |this, _, window, cx| {
                    this.file_tree.selected_path = Some(rel_path.clone());
                    if is_dir {
                        this.toggle_folder_expanded(&rel_path, cx);
                    } else {
                        this.open_file_in_editor(&rel_path, window, cx);
                    }
                }
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(is_dir, |el| {
                        el.child(
                            Icon::new(if is_open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(IconSize::Xs)
                            .color(theme.colors.fg_muted),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon).size(IconSize::Xs).color(icon_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.0))
                    .text_color(name_color)
                    .child(entry.name.clone()),
            );

        if !is_dir {
            row = row.on_drag(
                crate::ui::drag_drop::DraggedFile {
                    path: rel_path.clone(),
                    name: entry.name.clone(),
                },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            );
        }

        let menu = self.file_tree_context_menu(&rel_path, is_dir, cx);
        ContextMenu::new(SharedString::from(format!("ctx-menu-{}", rel_path)), menu)
            .child(row)
            .into_any_element()
    }

    fn entry_color(
        &self,
        rel_path: &str,
        is_dir: bool,
        theme: &ely_gpui_component::theme::Theme,
    ) -> Hsla {
        if is_dir {
            let prefix = format!("{rel_path}/");
            let has_change = self
                .git_status
                .staged
                .iter()
                .chain(&self.git_status.unstaged)
                .any(|c| c.path.starts_with(&prefix));
            if has_change {
                return theme.colors.warning;
            }
            return theme.colors.fg;
        }

        let change = self
            .git_status
            .staged
            .iter()
            .chain(&self.git_status.unstaged)
            .find(|c| c.path == rel_path);

        if let Some(change) = change {
            match change.status {
                crate::git::GitFileStatus::Modified => theme.colors.warning,
                crate::git::GitFileStatus::Added | crate::git::GitFileStatus::Untracked => {
                    theme.colors.success
                }
                crate::git::GitFileStatus::Deleted => theme.colors.danger,
                crate::git::GitFileStatus::Renamed => theme.colors.accent,
            }
        } else {
            theme.colors.fg
        }
    }

    fn file_tree_context_menu(&self, rel_path: &str, is_dir: bool, cx: &Context<Self>) -> Menu {
        let mut menu = Menu::new();
        let target = rel_path.to_string();

        if is_dir {
            let parent_file = target.clone();
            let parent_folder = target.clone();
            menu = menu
                .item(
                    MenuItem::new("New File…")
                        .icon(IconName::FilePlus)
                        .on_click(app_callback(cx, move |this, cx| {
                            this.prompt_new_file(parent_file.clone(), cx);
                        })),
                )
                .item(
                    MenuItem::new("New Folder…")
                        .icon(IconName::FolderPlus)
                        .on_click(app_callback(cx, move |this, cx| {
                            this.prompt_new_folder(parent_folder.clone(), cx);
                        })),
                )
                .separator();
        }

        let ren_target = target.clone();
        menu = menu.item(
            MenuItem::new("Rename…")
                .icon(IconName::Pencil)
                .on_click(app_callback(cx, move |this, cx| {
                    this.prompt_rename(ren_target.clone(), is_dir, cx);
                })),
        );

        let del_target = target.clone();
        menu = menu
            .item(
                MenuItem::new("Delete…")
                    .icon(IconName::Trash2)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.prompt_delete(del_target.clone(), is_dir, cx);
                    })),
            )
            .separator();

        let path_to_copy = target.clone();
        let cwd = self.workspace_cwd();
        menu = menu
            .item(MenuItem::new("Copy Path").icon(IconName::Copy).on_click({
                let cwd = cwd.clone();
                let path_to_copy = path_to_copy.clone();
                move |_, cx| {
                    let abs = Path::new(&cwd).join(&path_to_copy);
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        abs.to_string_lossy().to_string(),
                    ));
                }
            }))
            .item(
                MenuItem::new("Copy Relative Path")
                    .icon(IconName::Copy)
                    .on_click({
                        let path_to_copy = path_to_copy.clone();
                        move |_, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                path_to_copy.clone(),
                            ));
                        }
                    }),
            );

        let reveal_path = target.clone();
        let cwd_reveal = cwd.clone();
        menu = menu.item(
            MenuItem::new("Reveal in Finder")
                .icon(IconName::ExternalLink)
                .on_click(move |_, _| {
                    let full = Path::new(&cwd_reveal).join(&reveal_path);
                    #[cfg(target_os = "macos")]
                    if let Err(err) = std::process::Command::new("open")
                        .arg("-R")
                        .arg(&full)
                        .spawn()
                    {
                        log::error!("could not reveal {}: {err}", full.display());
                    }
                }),
        );

        menu
    }

    /// Renders any active file tree overlay dialog (New File, New Folder, Rename, Delete).
    pub fn refresh_file_tree(&mut self, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        if cwd.is_empty() || cwd == "~" {
            return;
        }
        let expanded: Vec<String> = std::iter::once(String::new())
            .chain(self.file_tree.expanded_paths.iter().cloned())
            .collect();
        let task = cx.background_executor().spawn(async move {
            let root = Path::new(&cwd);
            let mut cache = HashMap::new();
            for rel in expanded {
                let entries = read_dir_entries(root, &rel);
                cache.insert(rel, entries);
            }
            cache
        });
        cx.spawn(async move |this, cx| {
            let cache = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_tree.dir_cache.extend(cache);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn toggle_folder_expanded(&mut self, rel_path: &str, cx: &mut Context<Self>) {
        if self.file_tree.expanded_paths.contains(rel_path) {
            self.file_tree.expanded_paths.remove(rel_path);
            cx.notify();
        } else {
            self.file_tree.expanded_paths.insert(rel_path.to_string());
            if !self.file_tree.dir_cache.contains_key(rel_path) {
                self.load_directory(rel_path, cx);
            } else {
                cx.notify();
            }
        }
    }

    pub fn load_directory(&mut self, rel_path: &str, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        let rel = rel_path.to_string();
        let target_rel = rel.clone();
        let task = cx.background_executor().spawn(async move {
            let root = Path::new(&cwd);
            read_dir_entries(root, &target_rel)
        });
        cx.spawn(async move |this, cx| {
            let entries = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_tree.dir_cache.insert(rel, entries);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn collapse_all_folders(&mut self, cx: &mut Context<Self>) {
        self.file_tree.expanded_paths.clear();
        cx.notify();
    }

    pub fn prompt_new_file(&mut self, parent_dir: String, cx: &mut Context<Self>) {
        self.file_dialog_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.file_tree.dialog = Some(FileDialogAction::NewFile { parent_dir });
        cx.notify();
    }

    pub fn prompt_new_folder(&mut self, parent_dir: String, cx: &mut Context<Self>) {
        self.file_dialog_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.file_tree.dialog = Some(FileDialogAction::NewFolder { parent_dir });
        cx.notify();
    }

    pub fn prompt_rename(&mut self, target_path: String, is_dir: bool, cx: &mut Context<Self>) {
        let current_name = Path::new(&target_path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.file_dialog_input
            .update(cx, |input, cx| input.set_text(current_name, cx));
        self.file_tree.dialog = Some(FileDialogAction::Rename {
            target_path,
            is_dir,
        });
        cx.notify();
    }

    pub fn prompt_delete(&mut self, target_path: String, is_dir: bool, cx: &mut Context<Self>) {
        self.file_tree.dialog = Some(FileDialogAction::Delete {
            target_path,
            is_dir,
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_entry_icon() {
        let (icon, _) = resolve_entry_icon(".agents", true, false);
        assert_eq!(icon, IconName::Bot);

        let (icon, _) = resolve_entry_icon(".claude", true, false);
        assert_eq!(icon, IconName::Sparkles);

        let (icon, _) = resolve_entry_icon(".github", true, false);
        assert_eq!(icon, IconName::GitBranch);

        let (icon, _) = resolve_entry_icon("src", true, false);
        assert_eq!(icon, IconName::Folder);

        let (icon, _) = resolve_entry_icon("src", true, true);
        assert_eq!(icon, IconName::FolderOpen);

        let (icon, _) = resolve_entry_icon(".gitignore", false, false);
        assert_eq!(icon, IconName::GitBranch);

        let (icon, _) = resolve_entry_icon("README.md", false, false);
        assert_eq!(icon, IconName::Info);

        let (icon, _) = resolve_entry_icon("CLAUDE.md", false, false);
        assert_eq!(icon, IconName::FileText);

        let (icon, _) = resolve_entry_icon("package.json", false, false);
        assert_eq!(icon, IconName::FileJson);

        let (icon, _) = resolve_entry_icon("Cargo.lock", false, false);
        assert_eq!(icon, IconName::Lock);

        let (icon, _) = resolve_entry_icon("main.rs", false, false);
        assert_eq!(icon, IconName::FileCode);

        let (icon, _) = resolve_entry_icon("artisan", false, false);
        assert_eq!(icon, IconName::FileTerminal);
    }

    #[test]
    fn test_read_dir_entries_and_sorting() {
        let dir = std::env::temp_dir().join(format!("bencode-tree-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join(".agents")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join(".gitignore"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();
        std::fs::write(dir.join(".DS_Store"), "").unwrap();

        let entries = read_dir_entries(&dir, "");

        // .DS_Store must be excluded entirely
        assert!(!entries.iter().any(|e| e.name == ".DS_Store"));

        // target must be marked ignored
        let target_entry = entries.iter().find(|e| e.name == "target").unwrap();
        assert!(target_entry.ignored);

        // .agents, .claude, backend must NOT be ignored
        let agents = entries.iter().find(|e| e.name == ".agents").unwrap();
        assert!(!agents.ignored);
        assert!(agents.is_dir);

        let claude = entries.iter().find(|e| e.name == ".claude").unwrap();
        assert!(!claude.ignored);
        assert!(claude.is_dir);

        // Folders must appear before files
        let first_file_ix = entries.iter().position(|e| !e.is_dir).unwrap();
        for (i, entry) in entries.iter().enumerate() {
            if entry.is_dir {
                assert!(i < first_file_ix);
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
