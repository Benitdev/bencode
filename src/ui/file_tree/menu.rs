//! The Explorer's context menu (MonoCode `explorerItems` / `runAction`),
//! drawn at the pointer like MonoCode's `ExplorerMenu`.

use gpui::{AnyElement, Context, Pixels, Point};

use super::name::{is_within, parent_of};
use super::{Clip, MenuTarget, TreeMenu};
use crate::app::BenCodeApp;
use crate::ui::composer::focus_later;
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

impl BenCodeApp {
    /// MonoCode `openMenu`: ends any edit, selects the target.
    pub(crate) fn open_tree_menu(
        &mut self,
        target: MenuTarget,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.edit = None;
        self.file_tree.selected_path = Some(target.path.clone());
        let entries = self.tree_menu_entries(&target);
        self.file_tree.menu = Some(TreeMenu {
            active: explorer_menu::first_item(&entries),
            target,
            position,
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    /// MonoCode `explorerItems`.
    fn tree_menu_entries(&self, target: &MenuTarget) -> Vec<MenuEntry> {
        let root = target.is_root();
        let paste_parent = if target.is_dir {
            target.path.clone()
        } else {
            parent_of(&target.path)
        };
        let paste_blocked = self
            .file_tree
            .clip
            .as_ref()
            .is_some_and(|c| c.is_dir && is_within(&paste_parent, &c.path));
        let item = |id: &'static str, label: &str| MenuAction::new(id, label);
        let mut entries = vec![
            MenuEntry::Item(item("new-file", "New File")),
            MenuEntry::Item(item("new-folder", "New Folder")),
            MenuEntry::Separator,
            MenuEntry::Item(item("cut", "Cut").shortcut("⌘X").disabled(root)),
            MenuEntry::Item(item("copy", "Copy").shortcut("⌘C").disabled(root)),
            MenuEntry::Item(
                item("paste", "Paste")
                    .shortcut("⌘V")
                    .disabled(paste_blocked),
            ),
            MenuEntry::Item(item("duplicate", "Duplicate").disabled(root)),
            MenuEntry::Separator,
            MenuEntry::Item(item("copy-path", "Copy Path").shortcut("⌘⇧C")),
            MenuEntry::Item(item("copy-relative-path", "Copy Relative Path")),
            MenuEntry::Separator,
            MenuEntry::Item(item("rename", "Rename").shortcut("F2").disabled(root)),
            MenuEntry::Item(
                item("delete", "Delete")
                    .shortcut("⌫")
                    .disabled(root)
                    .danger(),
            ),
            MenuEntry::Separator,
            MenuEntry::Item(item("open-terminal", "Open in Terminal")),
        ];
        entries.push(MenuEntry::Item(item("reveal", REVEAL_LABEL)));
        entries
    }

    pub fn tree_menu_open(&self) -> bool {
        self.file_tree.menu.is_some()
    }

    pub fn close_tree_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_tree.menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// MonoCode `runAction`.
    fn pick_tree_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.file_tree.menu.as_ref() else {
            return;
        };
        let target = menu.target.clone();
        let entries = self.tree_menu_entries(&target);
        let Some(id) = explorer_menu::pick(&entries, index) else {
            return;
        };
        self.file_tree.menu = None;
        focus_later(self.file_tree_focus.clone(), cx);
        match id {
            "new-file" => self.start_tree_create(false, Some(target.path), cx),
            "new-folder" => self.start_tree_create(true, Some(target.path), cx),
            "cut" | "copy" if !target.is_root() => {
                self.file_tree.clip = Some(Clip {
                    cut: id == "cut",
                    path: target.path,
                    is_dir: target.is_dir,
                });
            }
            "paste" => self.paste_in_tree(&target.path, cx),
            "duplicate" => self.duplicate_in_tree(&target.path, cx),
            "copy-path" => {
                let text = self
                    .tree_abs_path(&target.path)
                    .to_string_lossy()
                    .into_owned();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            "copy-relative-path" => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(target.path));
            }
            "rename" => self.start_tree_rename(&target.path, cx),
            "delete" => self.request_tree_delete(&target.path, cx),
            "open-terminal" => {
                let rel = if target.is_dir {
                    target.path
                } else {
                    parent_of(&target.path)
                };
                let cwd = self.tree_abs_path(&rel).to_string_lossy().into_owned();
                self.new_terminal_at(&cwd, cx);
            }
            "reveal" => self.reveal_tree_path(&target.path, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Keys while the tree's menu holds focus.
    pub fn tree_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.file_tree.menu.as_ref() else {
            return false;
        };
        let entries = self.tree_menu_entries(&menu.target);
        let active = menu.active;
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.file_tree.menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, active, dir);
                }
            }
            "enter" | "space" => self.pick_tree_menu(active, cx),
            "escape" => {
                self.close_tree_menu(cx);
                focus_later(self.file_tree_focus.clone(), cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub fn render_tree_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.file_tree.menu.as_ref()?;
        let entries = self.tree_menu_entries(&menu.target);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "explorer-menu",
                entries: &entries,
                active: menu.active,
                place: MenuPlace::At(menu.position),
                width: MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.file_tree.menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("explorer menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_tree_menu(ix, cx)) {
                    log::debug!("explorer menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                if let Err(err) = close_app.update(cx, |this, cx| this.close_tree_menu(cx)) {
                    log::debug!("explorer menu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }
}
