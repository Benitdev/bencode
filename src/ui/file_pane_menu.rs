//! A file pane tab's context menu, as an IDE's editor tabs have: close
//! it, the others, those to its right, the saved ones or all, and copy or
//! reveal the file it shows. Drawn at the pointer like MonoCode's
//! `ExplorerMenu`.

use std::path::{Path, PathBuf};

use gpui::{AnyElement, ClipboardItem, Context, Pixels, Point};

use crate::app::BenCodeApp;
use crate::app::file_pane::{CloseMany, PaneTab};
use crate::ui::editor_pane::files::resolve_path;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuView};

/// MonoCode's explorer menu is 228px wide.
const MENU_WIDTH: f32 = 228.0;

/// MonoCode `REVEAL_LABEL`.
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};

#[derive(Clone, Debug)]
pub struct PaneTabMenu {
    /// The tab's key in `file_pane`.
    key: String,
    position: Point<Pixels>,
    active: usize,
}

impl BenCodeApp {
    /// Right-click on a tab: its menu at the pointer, first row lit.
    pub(crate) fn open_pane_tab_menu(
        &mut self,
        key: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let entries = self.pane_tab_menu_entries(key, cx);
        self.file_pane_menu = Some(PaneTabMenu {
            key: key.to_string(),
            position,
            active: explorer_menu::first_item(&entries),
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub fn pane_tab_menu_open(&self) -> bool {
        self.file_pane_menu.is_some()
    }

    pub fn close_pane_tab_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_pane_menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// The file a tab shows, on disk (or on its machine), if it shows one.
    fn pane_tab_file(&self, tab: &PaneTab, cx: &Context<Self>) -> Option<PathBuf> {
        match tab {
            PaneTab::File { path } => Some(match self.editor.files.get(path) {
                Some(file) => file.handle.abs_path.clone(),
                None => resolve_path(&self.workspace(cx).cwd, path),
            }),
            PaneTab::Review { cwd, path, .. } | PaneTab::FileHistory { cwd, path } => {
                Some(resolve_path(cwd, path))
            }
            _ => None,
        }
    }

    /// The file's path in the workspace, or its whole path outside it.
    fn pane_tab_relative(&self, tab: &PaneTab, cx: &Context<Self>) -> Option<String> {
        let file = self.pane_tab_file(tab, cx)?;
        let workspace = self.workspace(cx);
        let root = match tab {
            PaneTab::Review { cwd, .. } | PaneTab::FileHistory { cwd, .. } => Path::new(cwd),
            _ => Path::new(&workspace.cwd),
        };
        let relative = file.strip_prefix(root).unwrap_or(&file);
        Some(relative.to_string_lossy().into_owned())
    }

    fn pane_tab_dirty(&self, key: &str) -> bool {
        matches!(self.file_pane.get(key), Some(PaneTab::File { path }) if self.editor.files.is_dirty(path))
    }

    fn pane_tab_menu_entries(&self, key: &str, cx: &Context<Self>) -> Vec<MenuEntry> {
        let Some(tab) = self.file_pane.get(key) else {
            return Vec::new();
        };
        let none = |which| self.file_pane.close_keys(key, which).is_empty();
        let any_saved = (self.file_pane.close_keys(key, CloseMany::All).iter())
            .any(|k| !self.pane_tab_dirty(k));
        let preview = (self.file_pane.entries().iter()).any(|e| e.preview && e.tab.key() == key);
        let item = |id: &'static str, label: &str| MenuAction::new(id, label);
        let mut entries = vec![
            MenuEntry::Item(item("close", "Close").shortcut("⌘W")),
            MenuEntry::Item(item("others", "Close Others").disabled(none(CloseMany::Others))),
            MenuEntry::Item(item("right", "Close to the Right").disabled(none(CloseMany::Right))),
            MenuEntry::Item(item("saved", "Close Saved").disabled(!any_saved)),
            MenuEntry::Item(item("all", "Close All")),
        ];
        if preview {
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Item(item("keep", "Keep Open")),
            ]);
        }
        if let Some(file) = self.pane_tab_file(tab, cx) {
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Item(item("copy-path", "Copy Path")),
                MenuEntry::Item(item("copy-relative-path", "Copy Relative Path")),
            ]);
            // Finder cannot show a file on a machine.
            if !crate::remote::is_remote(&file.to_string_lossy()) {
                entries.push(MenuEntry::Item(item("reveal", REVEAL_LABEL)));
            }
        }
        if let PaneTab::Commit { .. } = tab {
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Item(item("copy-sha", "Copy Commit SHA")),
            ]);
        }
        entries
    }

    /// Closes tabs by key. A file with unsaved edits stays open; unless
    /// `saved_only`, the first of them asks whether to discard its edits.
    fn close_pane_tabs(&mut self, keys: &[String], saved_only: bool, cx: &mut Context<Self>) {
        let mut unsaved = None;
        for key in keys {
            if self.pane_tab_dirty(key) {
                unsaved.get_or_insert(key);
                continue;
            }
            match self.file_pane.get(key).cloned() {
                Some(PaneTab::File { path }) => self.close_editor_file(&path, cx),
                Some(_) => self.drop_pane_tab(key, cx),
                None => {}
            }
        }
        if let Some(key) = unsaved.filter(|_| !saved_only) {
            self.request_close_pane_tab(key, cx);
        }
    }

    fn pick_pane_tab_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.file_pane_menu.take() else {
            return;
        };
        let entries = self.pane_tab_menu_entries(&menu.key, cx);
        let Some(id) = explorer_menu::pick(&entries, index) else {
            self.file_pane_menu = Some(menu);
            return;
        };
        self.refocus_prompt(cx);
        let key = menu.key;
        let Some(tab) = self.file_pane.get(&key).cloned() else {
            cx.notify();
            return;
        };
        let copy = |text: String, cx: &mut Context<Self>| {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        };
        match id {
            "close" => self.request_close_pane_tab(&key, cx),
            "others" => {
                let keys = self.file_pane.close_keys(&key, CloseMany::Others);
                self.close_pane_tabs(&keys, false, cx);
            }
            "right" => {
                let keys = self.file_pane.close_keys(&key, CloseMany::Right);
                self.close_pane_tabs(&keys, false, cx);
            }
            "saved" | "all" => {
                let keys = self.file_pane.close_keys(&key, CloseMany::All);
                self.close_pane_tabs(&keys, id == "saved", cx);
            }
            "keep" => self.file_pane.keep(&key),
            "copy-path" => {
                if let Some(file) = self.pane_tab_file(&tab, cx) {
                    copy(file.to_string_lossy().into_owned(), cx);
                }
            }
            "copy-relative-path" => {
                if let Some(path) = self.pane_tab_relative(&tab, cx) {
                    copy(path, cx);
                }
            }
            "reveal" => {
                if let Some(file) = self.pane_tab_file(&tab, cx) {
                    cx.reveal_path(&file);
                }
            }
            "copy-sha" => {
                if let PaneTab::Commit { sha, .. } = tab {
                    copy(sha, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Keys while the tab's menu holds focus (MonoCode `ExplorerMenu.onMenuKey`).
    pub fn pane_tab_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.file_pane_menu.as_ref() else {
            return false;
        };
        let entries = self.pane_tab_menu_entries(&menu.key, cx);
        let active = menu.active;
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.file_pane_menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, active, dir);
                }
            }
            "enter" | "space" => self.pick_pane_tab_menu(active, cx),
            "escape" => {
                self.close_pane_tab_menu(cx);
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn render_pane_tab_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.file_pane_menu.as_ref()?;
        let entries = self.pane_tab_menu_entries(&menu.key, cx);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "file-pane-tab-menu",
                entries: &entries,
                active: menu.active,
                place: MenuPlace::At(menu.position),
                width: MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.file_pane_menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("file tab menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_pane_tab_menu(ix, cx)) {
                    log::debug!("file tab menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                if let Err(err) = close_app.update(cx, |this, cx| this.close_pane_tab_menu(cx)) {
                    log::debug!("file tab menu dismiss after app drop: {err:#}");
                }
            },
            cx,
        ))
    }
}
