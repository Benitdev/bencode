//! MonoCode `FileTree.tsx`, the sidebar's Explorer tab: the toolbar (New
//! File, New Folder, Collapse All, Search), the root row, and the lazily
//! listed tree. Names are created and renamed inline, the context menu and
//! keys cut / copy / paste / duplicate / delete, Finder files dropped or
//! pasted on a folder are copied in, and rows are tinted by git status.

mod fs;
mod icons;
mod menu;
mod name;
mod ops;
mod tints;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, HighlightStyle, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, ScrollHandle, SharedString, Styled, StyledText,
    div, px, relative, rgb,
};

use crate::app::BenCodeApp;
use crate::ui::icons::ExtraIcon;
use crate::ui::virtual_rows;

pub use icons::resolve_entry_icon;
pub use name::NameIssue;
use tints::GitTints;

/// MonoCode's row metrics: 30px rows (`h-7.5`), 12px per level, 8px in.
const ROW_HEIGHT: f32 = 30.0;
/// Tailwind preflight's `line-height: 1.5`, which MonoCode's `text-[Npx]`
/// rows inherit (GPUI's default is ~1.618).
const PREFLIGHT_LEADING: f32 = 1.5;
/// MonoCode `--leading-label: 1.4`, the entry name's line height.
const LABEL_LEADING: f32 = 1.4;
/// MonoCode `FileTypeIcon`'s default 16px box.
const ENTRY_ICON: IconSize = IconSize::Md;
/// Lucide `size-3.5` (chevrons, toolbar glyphs).
const GLYPH: IconSize = IconSize::Sm;
const INDENT: f32 = 12.0;
const INSET: f32 = 8.0;
/// A note row: one truncated `text-[12px]` line at the 1.5 leading.
const NOTE_HEIGHT: f32 = 18.0;

/// The icon and tint a file named `name` takes in the tree (MonoCode
/// `FileTypeIcon`), for other views that show file names.
pub fn entry_icon(name: &str) -> (ely_gpui_component::primitives::IconName, gpui::Hsla) {
    icons::resolve_entry_icon(name, false, false)
}

/// A filesystem entry (directory or file) in the workspace tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    /// Git-ignored (or `.git`): hidden, like MonoCode with "Show excluded
    /// files" off.
    pub ignored: bool,
}

/// The inline name field (MonoCode `NameRow`): a new entry under `parent`,
/// or a rename of `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeEdit {
    Create { parent: String, is_dir: bool },
    Rename { path: String, is_dir: bool },
}

/// What the field shows besides the name.
#[derive(Clone, Debug, Default)]
pub struct EditState {
    /// Enter was pressed on an empty name (MonoCode `attempted`).
    pub attempted: bool,
    pub busy: bool,
    pub submit_error: Option<String>,
}

/// MonoCode `Clip`: a cut or copied entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub cut: bool,
    pub path: String,
    pub is_dir: bool,
}

/// MonoCode `MenuTarget`: `path` `""` is the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuTarget {
    pub path: String,
    pub is_dir: bool,
}

impl MenuTarget {
    pub fn is_root(&self) -> bool {
        self.path.is_empty()
    }
}

pub struct TreeMenu {
    pub target: MenuTarget,
    pub position: Point<Pixels>,
    pub active: usize,
}

/// One root's folding and selection, kept while another root shows
/// (MonoCode `expandedByProject` / `selectedByProject`).
#[derive(Clone, Debug, Default)]
struct SavedTree {
    expanded: HashSet<String>,
    selected: Option<String>,
    root_collapsed: bool,
    /// Listings shown at once on return, while fresh ones load.
    dir_cache: HashMap<String, Vec<FsEntry>>,
}

/// The Explorer's state. Paths are relative to `root`; `""` is the root.
#[derive(Default)]
pub struct FileTreeState {
    /// The folder this state describes (the workspace cwd).
    pub root: String,
    pub expanded_paths: HashSet<String>,
    /// `Some("")` is the root itself.
    pub selected_path: Option<String>,
    pub root_collapsed: bool,
    pub dir_cache: HashMap<String, Vec<FsEntry>>,
    pub dir_errors: HashMap<String, String>,
    pub loading: HashSet<String>,
    saved: HashMap<String, SavedTree>,
    pub edit: Option<TreeEdit>,
    pub edit_state: EditState,
    pub clip: Option<Clip>,
    pub menu: Option<TreeMenu>,
    /// Last failed operation, shown above the tree.
    pub op_error: Option<String>,
    /// The folder a Finder drag would land in.
    pub drop_target: Option<String>,
    /// An entry waiting on "Delete …?".
    pub pending_delete: Option<(String, bool)>,
    /// A file to open in the editor on the next frame.
    pub pending_open: Option<String>,
    /// The tree's scroll pane, read to build only the rows in view.
    scroll: ScrollHandle,
    /// The git tints for the current `git_status`, built once per status
    /// change rather than per frame; cleared by [`Self::invalidate_tints`].
    tints: Option<Rc<GitTints>>,
}

impl FileTreeState {
    /// Drops the cached git tints; called when the git status changes.
    pub(crate) fn invalidate_tints(&mut self) {
        self.tints = None;
    }

    /// Switches to `root`, keeping the old root's folding and selection.
    fn switch_root(&mut self, root: &str) {
        let old = std::mem::take(&mut self.root);
        if !old.is_empty() {
            self.saved.insert(
                old,
                SavedTree {
                    expanded: std::mem::take(&mut self.expanded_paths),
                    selected: self.selected_path.take(),
                    root_collapsed: self.root_collapsed,
                    dir_cache: std::mem::take(&mut self.dir_cache),
                },
            );
        }
        let saved = self.saved.remove(root).unwrap_or_default();
        self.root = root.to_string();
        self.expanded_paths = saved.expanded;
        self.selected_path = saved.selected;
        self.root_collapsed = saved.root_collapsed;
        self.dir_cache = saved.dir_cache;
        self.dir_errors.clear();
        self.loading.clear();
        self.edit = None;
        self.edit_state = EditState::default();
        self.clip = None;
        self.menu = None;
        self.op_error = None;
        self.drop_target = None;
        self.scroll = ScrollHandle::default();
    }

    fn is_dir(&self, rel: &str) -> bool {
        if rel.is_empty() {
            return true;
        }
        let parent = name::parent_of(rel);
        self.dir_cache
            .get(&parent)
            .and_then(|entries| entries.iter().find(|e| e.rel_path == rel))
            .map_or_else(|| self.dir_cache.contains_key(rel), |e| e.is_dir)
    }

    /// MonoCode `createParentOf`: the selected folder, or a selected
    /// file's folder, or the root.
    fn create_parent_of(&self, rel: Option<&str>) -> String {
        match rel {
            None | Some("") => String::new(),
            Some(rel) if self.is_dir(rel) => rel.to_string(),
            Some(rel) => name::parent_of(rel),
        }
    }

    /// Names already in `dir`, for the name field's checks.
    fn names_in(&self, dir: &str, except: Option<&str>) -> Vec<String> {
        self.dir_cache
            .get(dir)
            .into_iter()
            .flatten()
            .filter(|e| Some(e.name.as_str()) != except)
            .map(|e| e.name.clone())
            .collect()
    }
}

/// One line of the flattened tree.
enum Row<'a> {
    Entry { entry: &'a FsEntry, depth: usize },
    Name { depth: usize, is_dir: bool },
    Note { text: String, depth: usize },
}

impl Row<'_> {
    /// The row's laid-out height; `None` for the name field, whose
    /// message line can wrap.
    fn height(&self) -> Option<f32> {
        match self {
            Row::Entry { .. } => Some(ROW_HEIGHT),
            Row::Note { .. } => Some(NOTE_HEIGHT),
            Row::Name { .. } => None,
        }
    }
}

impl BenCodeApp {
    /// Keeps the tree on the workspace folder; a new root starts fresh.
    fn sync_tree_root(&mut self, cx: &mut Context<Self>) -> String {
        let cwd = self.workspace_cwd();
        if self.file_tree.root != cwd {
            self.file_tree.switch_root(&cwd);
            self.refresh_file_tree(cx);
        } else if !self.file_tree.dir_cache.contains_key("") && !self.file_tree.loading.contains("") {
            self.refresh_file_tree(cx);
        }
        cwd
    }

    /// MonoCode `TreeChildren` order: an error, a new folder's field, "…"
    /// while listing, folders, a new file's field, files.
    fn collect_rows<'a>(&'a self, rel_dir: &str, depth: usize, out: &mut Vec<Row<'a>>) {
        let tree = &self.file_tree;
        let creating = match &tree.edit {
            Some(TreeEdit::Create { parent, is_dir }) if parent == rel_dir => Some(*is_dir),
            _ => None,
        };
        if let Some(error) = tree.dir_errors.get(rel_dir) {
            out.push(Row::Note {
                text: error.clone(),
                depth,
            });
        }
        if creating == Some(true) {
            out.push(Row::Name { depth, is_dir: true });
        }
        let entries = tree.dir_cache.get(rel_dir);
        if entries.is_none() && !tree.dir_errors.contains_key(rel_dir) {
            out.push(Row::Note {
                text: "…".into(),
                depth,
            });
        }
        let visible: Vec<&FsEntry> = entries.into_iter().flatten().filter(|e| !e.ignored).collect();
        for entry in visible.iter().filter(|e| e.is_dir) {
            self.push_entry(entry, depth, out);
        }
        if creating == Some(false) {
            out.push(Row::Name {
                depth,
                is_dir: false,
            });
        }
        for entry in visible.iter().filter(|e| !e.is_dir) {
            self.push_entry(entry, depth, out);
        }
    }

    fn push_entry<'a>(&'a self, entry: &'a FsEntry, depth: usize, out: &mut Vec<Row<'a>>) {
        let renaming = matches!(&self.file_tree.edit, Some(TreeEdit::Rename { path, .. }) if *path == entry.rel_path);
        if renaming {
            out.push(Row::Name {
                depth,
                is_dir: entry.is_dir,
            });
        } else {
            out.push(Row::Entry { entry, depth });
        }
        if entry.is_dir && self.file_tree.expanded_paths.contains(&entry.rel_path) {
            self.collect_rows(&entry.rel_path, depth + 1, out);
        }
    }

    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let cwd = self.sync_tree_root(cx);
        let colors = cx.theme().colors.clone();
        let fg = colors.fg;
        if matches!(cwd.trim(), "" | "~") {
            return div()
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.5))
                .child("No project folder")
                .into_any_element();
        }
        let tints = self.file_tree_tints();
        let root_open = !self.file_tree.root_collapsed;
        let mut rows = Vec::new();
        if root_open {
            self.collect_rows("", 0, &mut rows);
        }
        // Only the rows in view are built; spacers stand in for the rest.
        // The error line above the rows has no known height, so everything
        // is built while it shows.
        let visible = if self.file_tree.op_error.is_some() {
            virtual_rows::Window::all(rows.len())
        } else {
            let heights: Vec<Option<f32>> = rows.iter().map(Row::height).collect();
            virtual_rows::for_scroll(&heights, &self.file_tree.scroll, 0.0)
        };
        let rendered: Vec<AnyElement> = rows[visible.range.clone()]
            .iter()
            .map(|row| match row {
                Row::Entry { entry, depth } => self.render_tree_entry(entry, *depth, &tints, cx),
                Row::Name { depth, is_dir } => self.render_name_row(*depth, *is_dir, cx),
                Row::Note { text, depth } => div()
                    .pl(px(28.0 + *depth as f32 * INDENT))
                    .pr_2()
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.5))
                    .child(text.clone())
                    .into_any_element(),
            })
            .collect();
        div()
            .id("file-tree")
            .key_context("FileTree")
            .track_focus(&self.file_tree_focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .line_height(relative(PREFLIGHT_LEADING))
            .child(self.render_tree_toolbar(cx))
            .child(self.render_tree_root_row(&cwd, cx))
            .child(crate::ui::scrollbar::framed(
                "explorer-tree-scrollbar",
                &self.file_tree.scroll,
                div()
                    .id("explorer-tree-scroll-pane")
                    .track_scroll(&self.file_tree.scroll)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .pr(crate::ui::scrollbar::gutter())
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    // Right-clicking empty space opens the root's menu.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.file_tree_focus, cx);
                            let target = MenuTarget {
                                path: String::new(),
                                is_dir: true,
                            };
                            this.open_tree_menu(target, event.position, cx);
                        }),
                    )
                    .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                        this.drop_external_files(paths.paths().to_vec(), "", cx);
                    }))
                    .when_some(self.file_tree.op_error.clone(), |el, error| {
                        el.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_size(px(12.0))
                                .line_height(px(16.0))
                                .text_color(rgb(0xf87171))
                                .child(error),
                        )
                    })
                    .when(visible.above > 0.0, |el| el.child(div().flex_none().h(px(visible.above))))
                    .children(rendered)
                    .when(visible.below > 0.0, |el| el.child(div().flex_none().h(px(visible.below)))),
            ))
            .into_any_element()
    }

    /// The git tints for the current status, rebuilt only after
    /// [`FileTreeState::invalidate_tints`].
    fn file_tree_tints(&mut self) -> Rc<GitTints> {
        let status = &self.git_status;
        self.file_tree
            .tints
            .get_or_insert_with(|| {
                Rc::new(GitTints::new(
                    status
                        .staged
                        .iter()
                        .chain(&status.unstaged)
                        .map(|c| (c.path.clone(), c.status.clone())),
                ))
            })
            .clone()
    }

    /// MonoCode's toolbar: `h-9 gap-px border-b border-stroke px-2` and
    /// four equal `HeaderIcon`s (`h-6 flex-1 rounded-md text-content/50
    /// hover:bg-content/5 hover:text-content`, `size-3.5` glyphs).
    fn render_tree_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let button = |id: &'static str, tip: &'static str, icon: AnyElement| {
            div()
                .id(id)
                .group(id)
                .flex_1()
                .min_w_0()
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(fg.opacity(0.05)))
                .tooltip(Tooltip::text(tip))
                .child(icon)
        };
        let muted = fg.opacity(0.5);
        let icon = |group: &'static str, name: IconName| {
            Icon::new(name)
                .size(GLYPH)
                .color(muted)
                .group_hover_color(group, fg)
                .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .h(px(36.0))
            .items_center()
            .gap(px(1.0))
            .px_2()
            .border_b_1()
            // `border-stroke`: content at 7%.
            .border_color(fg.opacity(0.07))
            .child(
                button(
                    "tree-toolbar-new-file",
                    "New File",
                    icon("tree-toolbar-new-file", IconName::FilePlus),
                )
                    .on_click(cx.listener(|this, _, _, cx| this.start_tree_create(false, None, cx))),
            )
            .child(
                button(
                    "tree-toolbar-new-folder",
                    "New Folder",
                    icon("tree-toolbar-new-folder", IconName::FolderPlus),
                )
                    .on_click(cx.listener(|this, _, _, cx| this.start_tree_create(true, None, cx))),
            )
            .child(
                button(
                    "tree-toolbar-collapse-all",
                    "Collapse All",
                    ExtraIcon::FoldVertical.icon().size(IconSize::Sm).color(muted).into_any_element(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.collapse_all_folders(cx))),
            )
            .child(
                button(
                    "tree-toolbar-search",
                    "Search in files (⌘Shift+F)",
                    icon("tree-toolbar-search", IconName::Search),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_search_modal(cx))),
            )
    }

    /// MonoCode's root row: the branch when the thread runs in a worktree,
    /// else the folder name; a click selects and folds it. `h-8`, a
    /// `gap-1 pl-2` button with no right padding, and a `text-[11px]
    /// font-semibold tracking-[0.08em] text-content/50 uppercase` label
    /// (GPUI has no letter spacing, so the tracking is dropped).
    fn render_tree_root_row(&self, cwd: &str, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let open = !self.file_tree.root_collapsed;
        let label = self.tree_root_label(cwd);
        let drop = self.file_tree.drop_target.as_deref() == Some("");
        div()
            .id("explorer-root-header")
            .flex()
            .flex_none()
            .h(px(32.0))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_1()
            .pl(px(INSET))
            .when(drop, |el| el.bg(cx.theme().colors.active))
            .tooltip(Tooltip::text(cwd.to_string()))
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.file_tree_focus, cx);
                this.file_tree.selected_path = Some(String::new());
                this.file_tree.root_collapsed = !this.file_tree.root_collapsed;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.file_tree_focus, cx);
                    let target = MenuTarget {
                        path: String::new(),
                        is_dir: true,
                    };
                    this.open_tree_menu(target, event.position, cx);
                }),
            )
            .on_drag_move::<gpui::ExternalPaths>(cx.listener(
                |this, event: &gpui::DragMoveEvent<gpui::ExternalPaths>, _, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    this.set_tree_drop(String::new(), inside, cx);
                },
            ))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                cx.stop_propagation();
                this.drop_external_files(paths.paths().to_vec(), "", cx);
            }))
            .child(
                div().flex_none().size(px(16.0)).flex().items_center().justify_center().child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(GLYPH)
                    .color(fg.opacity(0.5)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(fg.opacity(0.5))
                    .child(label.to_uppercase()),
            )
    }

    /// MonoCode `explorerRootLabel`: a thread running in a worktree names
    /// the root by its branch.
    fn tree_root_label(&self, cwd: &str) -> String {
        let worktree = self.worktree_focus().map(|f| f.branch.clone()).or_else(|| {
            self.selected_session_id
                .as_deref()
                .and_then(|id| self.sessions.iter().find(|s| s.id == id))
                .filter(|s| s.worktree_cwd.as_deref().is_some_and(|w| !w.is_empty()) && !s.worktree_removed)
                .map(|s| s.branch.clone())
        });
        worktree
            .flatten()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| {
                Path::new(cwd)
                    .file_name()
                    .map_or_else(|| cwd.to_string(), |n| n.to_string_lossy().into_owned())
            })
    }

    fn render_tree_entry(
        &self,
        entry: &FsEntry,
        depth: usize,
        tints: &GitTints,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let tree = &self.file_tree;
        let rel = entry.rel_path.clone();
        let is_dir = entry.is_dir;
        let selected = tree.selected_path.as_deref() == Some(rel.as_str());
        let open = is_dir && tree.expanded_paths.contains(&rel);
        let cut = tree.clip.as_ref().is_some_and(|c| c.cut && c.path == rel);
        let drop = tree.drop_target.as_deref() == Some(rel.as_str());
        let (icon, icon_color) = resolve_entry_icon(&entry.name, is_dir, open);
        let name_color = tints.color(&rel, is_dir, cx.theme().is_dark()).unwrap_or(fg);
        let selection = colors.active;
        let (click_rel, menu_rel, hover_rel, drop_rel) = (rel.clone(), rel.clone(), rel.clone(), rel.clone());
        let mut row = div()
            .id(SharedString::from(format!("tree-item-{rel}")))
            .flex()
            .flex_none()
            .h(px(ROW_HEIGHT))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_1()
            .pl(px(INSET + depth as f32 * INDENT))
            .pr_2()
            // `text-[14px] leading-none`
            .text_size(px(14.0))
            .line_height(relative(1.0))
            .when(cut, |el| el.opacity(0.5))
            .map(|el| {
                if selected || drop {
                    el.bg(selection)
                } else {
                    el.hover(move |s| s.bg(fg.opacity(0.05)))
                }
            })
            .tooltip(Tooltip::text(rel.clone()))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                window.focus(&this.file_tree_focus, cx);
                this.file_tree.selected_path = Some(click_rel.clone());
                if is_dir {
                    this.toggle_folder_expanded(&click_rel, cx);
                } else if event.click_count() < 2 {
                    this.open_file_in_editor(&click_rel, window, cx);
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.file_tree_focus, cx);
                    let target = MenuTarget {
                        path: menu_rel.clone(),
                        is_dir,
                    };
                    this.open_tree_menu(target, event.position, cx);
                }),
            )
            .on_drag_move::<gpui::ExternalPaths>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<gpui::ExternalPaths>, _, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    let target = this.file_tree.create_parent_of(Some(&hover_rel));
                    this.set_tree_drop(target, inside, cx);
                },
            ))
            .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, _, cx| {
                cx.stop_propagation();
                let target = this.file_tree.create_parent_of(Some(&drop_rel));
                this.drop_external_files(paths.paths().to_vec(), &target, cx);
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
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(GLYPH)
                            .color(fg.opacity(0.5)),
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
                    .child(Icon::new(icon).size(ENTRY_ICON).color(icon_color)),
            )
            .child(
                // `min-w-0 truncate leading-label`
                div()
                    .min_w_0()
                    .truncate()
                    .line_height(relative(LABEL_LEADING))
                    .text_color(name_color)
                    .child(entry.name.clone()),
            );
        if !is_dir {
            row = row.on_drag(
                crate::ui::drag_drop::DraggedFile {
                    path: rel,
                    name: entry.name.clone(),
                },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            );
        }
        row.into_any_element()
    }

    /// MonoCode `NameRow` + `NameIssueView`: the field with the leaf's live
    /// icon, and the problem with the name under it.
    fn render_name_row(&self, depth: usize, is_dir: bool, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let raw = self.file_dialog_input.read(cx).text().to_string();
        let (icon, tint) = resolve_entry_icon(&name::leaf(&raw), is_dir, false);
        let issue = self.tree_name_issue(&raw);
        let state = &self.file_tree.edit_state;
        let message = state.submit_error.clone().or_else(|| {
            let issue = issue.as_ref()?;
            let show = !issue.is_error()
                || (*issue != NameIssue::Empty && !raw.is_empty())
                || (*issue == NameIssue::Empty && state.attempted);
            show.then(|| issue.message())
        });
        // MonoCode sets the clashing name in `font-semibold`.
        let emphasis = state
            .submit_error
            .is_none()
            .then(|| issue.as_ref().and_then(NameIssue::emphasis))
            .flatten();
        let error = state.submit_error.is_some() || issue.as_ref().is_none_or(NameIssue::is_error);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .h(px(ROW_HEIGHT))
                    .items_center()
                    .gap_1()
                    .pl(px(INSET + depth as f32 * INDENT))
                    .pr_2()
                    .bg(fg.opacity(0.10))
                    .child(
                        div().size(px(16.0)).flex().flex_none().items_center().justify_center().when(is_dir, |el| {
                            el.child(Icon::new(IconName::ChevronRight).size(GLYPH).color(fg.opacity(0.5)))
                        }),
                    )
                    .child(
                        div()
                            .size(px(16.0))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(icon).size(ENTRY_ICON).color(tint)),
                    )
                    .child(
                        // `h-5 rounded-sm bg-content/10 px-1 text-[14px]
                        // leading-none ring-1 ring-accent`. The ring sits
                        // outside the 20px box, so the border is drawn 1px
                        // out on every side to keep the box at 20px.
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(22.0))
                            .mx(px(-1.0))
                            .flex()
                            .items_center()
                            .px(px(5.0))
                            .rounded(px(5.0))
                            .bg(fg.opacity(0.10))
                            .border_1()
                            .border_color(colors.accent)
                            .text_size(px(14.0))
                            .line_height(relative(1.0))
                            .when(state.busy, |el| el.opacity(0.6))
                            .child(self.file_dialog_input.clone()),
                    ),
            )
            .when_some(message, |el, message| {
                el.child(
                    div()
                        .pl(px(28.0 + depth as f32 * INDENT))
                        .pr_2()
                        .pb_1()
                        .text_size(px(12.0))
                        .line_height(px(16.0))
                        .text_color(rgb(if error { 0xf87171 } else { 0xfbbf24 }))
                        .child(match emphasis {
                            Some(range) => StyledText::new(message).with_highlights([(
                                range,
                                HighlightStyle {
                                    font_weight: Some(FontWeight::SEMIBOLD),
                                    ..Default::default()
                                },
                            )]),
                            None => StyledText::new(message),
                        }),
                )
            })
            .into_any_element()
    }

    /// The name field's problem against the target folder's names.
    pub(crate) fn tree_name_issue(&self, raw: &str) -> Option<NameIssue> {
        let tree = &self.file_tree;
        match tree.edit.as_ref()? {
            TreeEdit::Create { parent, .. } => name::validate(raw, &tree.names_in(parent, None)),
            TreeEdit::Rename { path, .. } => {
                let current = Path::new(path).file_name().and_then(|n| n.to_str());
                name::validate(raw, &tree.names_in(&name::parent_of(path), current))
            }
        }
    }

    fn set_tree_drop(&mut self, target: String, over: bool, cx: &mut Context<Self>) {
        let current = self.file_tree.drop_target.as_ref();
        if over && current != Some(&target) {
            self.file_tree.drop_target = Some(target);
            cx.notify();
        } else if !over && current == Some(&target) {
            self.file_tree.drop_target = None;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_roots_keeps_each_roots_folding() {
        let mut tree = FileTreeState::default();
        tree.switch_root("/a");
        tree.expanded_paths.insert("src".into());
        tree.selected_path = Some("src/main.rs".into());
        tree.dir_cache.insert(String::new(), Vec::new());
        tree.switch_root("/b");
        assert!(tree.expanded_paths.is_empty() && tree.dir_cache.is_empty());
        tree.switch_root("/a");
        assert!(tree.expanded_paths.contains("src"));
        assert_eq!(tree.selected_path.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn new_entries_go_into_the_selected_folder() {
        let mut tree = FileTreeState::default();
        tree.dir_cache.insert(
            String::new(),
            vec![
                FsEntry {
                    name: "src".into(),
                    rel_path: "src".into(),
                    is_dir: true,
                    ignored: false,
                },
                FsEntry {
                    name: "a.rs".into(),
                    rel_path: "a.rs".into(),
                    is_dir: false,
                    ignored: false,
                },
            ],
        );
        assert_eq!(tree.create_parent_of(Some("src")), "src");
        assert_eq!(tree.create_parent_of(Some("a.rs")), "");
        assert_eq!(tree.create_parent_of(Some("src/x/y.rs")), "src/x");
        assert_eq!(tree.create_parent_of(None), "");
    }
}
