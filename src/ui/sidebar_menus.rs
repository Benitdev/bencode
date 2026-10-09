//! The Sessions tab's menus, drawn at the pointer like MonoCode's
//! `ExplorerMenu`: a card's menu (acting on every picked card), a
//! folder's menu, and the "Remind me" presets. The popovers they share
//! state with are in `sidebar_popovers`.

use gpui::{AnyElement, Context, Pixels, Point};

use crate::app::BenCodeApp;
use crate::app::reminders;
use crate::app::session_folders::folder_of;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuView};
use crate::ui::scale::px;
use crate::ui::sidebar::SessionDialog;

/// MonoCode's session menu is an `ExplorerMenu` (228px); the filter
/// popover is 228px too.
const MENU_WIDTH: f32 = 228.0;
/// MonoCode's folder menu, with its colour row, is 260px.
const FOLDER_MENU_WIDTH: f32 = 260.0;

#[derive(Clone, Debug, PartialEq)]
pub enum SidebarMenuKind {
    /// A card's menu; `ids` are its targets in list order.
    Session {
        clicked: String,
        ids: Vec<String>,
    },
    Folder(String),
    Filter,
    /// MonoCode `SidebarWorktreeSwitcher`'s list of working copies.
    Worktrees,
    /// MonoCode's "Remind me" presets (also the reminder panel's Snooze).
    Remind {
        ids: Vec<String>,
    },
    /// The rail's "Open project" popover.
    AddProject,
    /// The icon rail's project list.
    CompactProjects,
}

#[derive(Clone, Debug)]
pub struct SidebarMenu {
    pub kind: SidebarMenuKind,
    pub position: Point<Pixels>,
    pub active: usize,
}

impl BenCodeApp {
    pub(crate) fn sidebar_menu_is_filter(&self) -> bool {
        matches!(
            self.sidebar_menu.as_ref().map(|m| &m.kind),
            Some(SidebarMenuKind::Filter)
        )
    }

    pub fn sidebar_menu_open(&self) -> bool {
        self.sidebar_menu.is_some()
    }

    /// Closes any sidebar menu; a card picked only for the menu is let go
    /// (MonoCode `closeSessionMenu`).
    pub fn close_sidebar_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.sidebar_menu.take().is_none() {
            return false;
        }
        self.sessions_ui.folder_color_picker = false;
        if std::mem::take(&mut self.sessions_ui.folder_colors_unsaved) {
            self.save_settings(cx);
        }
        if std::mem::take(&mut self.sessions_ui.selection.from_menu) {
            self.sessions_ui.selection.clear();
        }
        cx.notify();
        true
    }

    pub(crate) fn open_sidebar_menu(
        &mut self,
        kind: SidebarMenuKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_inline_rename(cx);
        let mut menu = SidebarMenu {
            kind,
            position,
            active: 0,
        };
        menu.active = explorer_menu::first_item(&self.sidebar_menu_entries(&menu.kind));
        self.sidebar_menu = Some(menu);
        self.focus_composer_menu(cx);
        cx.notify();
    }

    /// Right-click on a card: an unpicked card becomes the lone pick for
    /// the menu's life (MonoCode `onSessionContextMenu`).
    pub(crate) fn open_session_menu(
        &mut self,
        id: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.close_sidebar_menu(cx);
        let selection = &mut self.sessions_ui.selection;
        if !selection.ids.contains(id) {
            selection.ids.clear();
            selection.ids.insert(id.to_string());
            selection.from_menu = true;
        }
        let ids = selection.action_ids(id, &self.sessions_ui.order);
        let kind = SidebarMenuKind::Session {
            clicked: id.to_string(),
            ids,
        };
        self.open_sidebar_menu(kind, position, cx);
    }

    pub(crate) fn open_folder_menu(
        &mut self,
        folder_id: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.close_sidebar_menu(cx);
        // A folder with a custom colour opens with its picker showing.
        self.sessions_ui.folder_color_picker = self
            .project_folders()
            .iter()
            .any(|f| f.id == folder_id && f.custom_color.is_some());
        self.open_sidebar_menu(SidebarMenuKind::Folder(folder_id.to_string()), position, cx);
    }

    /// MonoCode `onFilterButtonClick`: opens under the button, or closes.
    pub(crate) fn toggle_filter_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.sidebar_menu_is_filter() {
            self.close_sidebar_menu(cx);
            return;
        }
        self.close_sidebar_menu(cx);
        let position = Point::new(at.x - px(MENU_WIDTH - 12.0), at.y + px(14.0));
        self.sidebar_menu = Some(SidebarMenu {
            kind: SidebarMenuKind::Filter,
            position,
            active: 0,
        });
        cx.notify();
    }

    /// MonoCode `sessionMenuItems` (no reminders or GitHub links yet) and
    /// `folderMenuItems`.
    fn sidebar_menu_entries(&self, kind: &SidebarMenuKind) -> Vec<MenuEntry> {
        match kind {
            SidebarMenuKind::Session { ids, .. } => self.session_menu_entries(ids),
            SidebarMenuKind::Folder(_) => vec![
                MenuEntry::Item(MenuAction::new("rename", "Rename").shortcut("F2")),
                MenuEntry::Separator,
                MenuEntry::Item(MenuAction::new("ungroup", "Ungroup")),
            ],
            SidebarMenuKind::Remind { .. } => reminders::preset_rows(crate::app::now_ms())
                .into_iter()
                .map(|(preset, label, enabled)| {
                    MenuEntry::Item(
                        MenuAction::new("remind", label)
                            .disabled(!enabled)
                            .value(preset.id()),
                    )
                })
                .collect(),
            SidebarMenuKind::Filter
            | SidebarMenuKind::Worktrees
            | SidebarMenuKind::AddProject
            | SidebarMenuKind::CompactProjects => Vec::new(),
        }
    }

    fn session_menu_entries(&self, ids: &[String]) -> Vec<MenuEntry> {
        let targets: Vec<_> = ids
            .iter()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .collect();
        let many = ids.len() > 1;
        let all_pinned = !targets.is_empty() && targets.iter().all(|s| s.pinned);
        let all_archived = !targets.is_empty() && targets.iter().all(|s| s.archived);
        let folders = self.project_folders();
        let item = |id: &'static str, label: &str| MenuEntry::Item(MenuAction::new(id, label));
        let mut entries = Vec::new();
        // MonoCode "Cancel reminder", with when it is due.
        let mut times: Vec<i64> = ids
            .iter()
            .filter_map(|id| self.reminder_for(id).map(|r| r.due_at))
            .collect();
        times.sort_unstable();
        times.dedup();
        if !times.is_empty() {
            let when = match times.as_slice() {
                [one] => reminders::format_reminder_time(*one),
                _ => "Multiple reminder times".into(),
            };
            entries.push(MenuEntry::Item(
                MenuAction::new("reminder-cancel", "Cancel reminder").description(Some(when)),
            ));
            entries.push(MenuEntry::Separator);
        }
        entries.push(item("pin", if all_pinned { "Unpin" } else { "Pin" }));
        if !many {
            entries.push(MenuEntry::Item(
                MenuAction::new("rename", "Rename").shortcut("F2"),
            ));
            let harness_id = targets.first().and_then(|s| s.provider_session_id.clone());
            entries.push(MenuEntry::Item(
                MenuAction::new("copy-harness-session-id", "Copy harness session ID")
                    .disabled(harness_id.is_none()),
            ));
            entries.push(item("copy-bencode-session-id", "Copy BenCode session ID"));
            entries.push(item(
                "link-work-item",
                if targets
                    .first()
                    .is_some_and(|s| s.linked_work_item.is_some())
                {
                    "Edit GitHub issue or PR link…"
                } else {
                    "Link GitHub issue or PR…"
                },
            ));
        }
        // MonoCode only reminds about a thread that was sent.
        let unsent = targets.iter().any(|s| !s.has_user_message());
        entries.push(MenuEntry::Item(
            MenuAction::new("reminder", "Remind me…").disabled(unsent),
        ));
        entries.push(MenuEntry::Separator);
        entries.push(item("folder-new", "New folder"));
        if !folders.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        for folder in folders {
            let checked = !ids.is_empty()
                && ids
                    .iter()
                    .all(|id| folder.session_ids.iter().any(|s| s == id));
            entries.push(MenuEntry::Item(
                MenuAction::new("folder-add", format!("Add to {}", folder.name))
                    .checked(checked)
                    .value(folder.id.clone()),
            ));
        }
        let foldered = ids.iter().any(|id| folder_of(folders, id).is_some());
        if foldered {
            entries.push(item(
                "folder-remove",
                if many {
                    "Remove from folders"
                } else {
                    "Remove from folder"
                },
            ));
        }
        entries.push(MenuEntry::Separator);
        entries.push(item(
            "archive",
            if all_archived { "Unarchive" } else { "Archive" },
        ));
        entries.push(MenuEntry::Item(
            MenuAction::new("delete", "Delete").shortcut("⌫").danger(),
        ));
        entries
    }

    fn pick_sidebar_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.sidebar_menu.clone() else {
            return;
        };
        let entries = self.sidebar_menu_entries(&menu.kind);
        let Some(action) = explorer_menu::pick_action(&entries, index).cloned() else {
            return;
        };
        self.close_sidebar_menu(cx);
        self.refocus_prompt(cx);
        // "Remind me…" opens its presets where the menu was.
        if let SidebarMenuKind::Session { ids, .. } = &menu.kind
            && action.id == "reminder"
        {
            let kind = SidebarMenuKind::Remind { ids: ids.clone() };
            self.open_sidebar_menu(kind, menu.position, cx);
            return;
        }
        match menu.kind {
            SidebarMenuKind::Session { clicked, ids } => {
                self.run_session_menu_action(action, &clicked, &ids, cx)
            }
            SidebarMenuKind::Folder(folder_id) => match action.id {
                "rename" => self.start_folder_rename(&folder_id, cx),
                "ungroup" => self.ungroup_folder(&folder_id, cx),
                _ => {}
            },
            SidebarMenuKind::Remind { ids } => {
                let due = action
                    .value
                    .as_deref()
                    .and_then(reminders::Preset::from_id)
                    .and_then(|preset| reminders::reminder_time(preset, crate::app::now_ms()));
                if let Some(due_at) = due {
                    self.schedule_reminders(&ids, due_at, cx);
                }
            }
            SidebarMenuKind::Filter
            | SidebarMenuKind::Worktrees
            | SidebarMenuKind::AddProject
            | SidebarMenuKind::CompactProjects => {}
        }
        cx.notify();
    }

    fn run_session_menu_action(
        &mut self,
        action: MenuAction,
        clicked: &str,
        ids: &[String],
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<_> = ids
            .iter()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .collect();
        let all_pinned = !targets.is_empty() && targets.iter().all(|s| s.pinned);
        let all_archived = !targets.is_empty() && targets.iter().all(|s| s.archived);
        let harness_id = targets.first().and_then(|s| s.provider_session_id.clone());
        match action.id {
            "pin" => {
                for id in ids {
                    self.set_session_pinned(id, !all_pinned, cx);
                }
            }
            "rename" => self.start_session_rename(clicked, cx),
            "reminder-cancel" => self.cancel_reminders(ids, None, cx),
            "link-work-item" => self.open_link_dialog(clicked, cx),
            "copy-harness-session-id" => {
                if let Some(value) = harness_id {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(value));
                }
            }
            "copy-bencode-session-id" => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(clicked.to_string()));
            }
            "folder-new" => {
                if let Some(created) = self.new_folder_with_sessions(ids, cx) {
                    self.start_folder_rename(&created, cx);
                }
            }
            "folder-add" => {
                if let Some(folder_id) = action.value.as_deref() {
                    self.add_sessions_to_folder(ids, folder_id, cx);
                }
            }
            "folder-remove" => self.remove_sessions_from_folders(ids, cx),
            "archive" => {
                for id in ids {
                    self.set_session_archived(id, !all_archived, cx);
                }
            }
            "delete" => self.request_delete_sessions(ids.to_vec(), cx),
            _ => {}
        }
    }

    /// Delete asks first (BenCode keeps every destructive action behind
    /// a confirmation), with MonoCode's wording.
    pub(crate) fn request_delete_sessions(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.session_dialog = match ids.as_slice() {
            [] => None,
            [one] => Some(SessionDialog::Delete(one.clone())),
            _ => Some(SessionDialog::DeleteSelected(ids)),
        };
        self.sessions_ui.selection.clear();
        cx.notify();
    }

    /// Keys while a card or folder menu holds focus.
    pub fn sidebar_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.sidebar_menu.as_ref() else {
            return false;
        };
        let entries = self.sidebar_menu_entries(&menu.kind);
        let active = menu.active;
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.sidebar_menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, active, dir);
                }
            }
            "enter" | "space" => self.pick_sidebar_menu(active, cx),
            "escape" => {
                self.close_sidebar_menu(cx);
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub fn render_sidebar_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.sidebar_menu.as_ref()?;
        match menu.kind {
            SidebarMenuKind::Filter => return Some(self.render_filter_menu(menu.position, cx)),
            SidebarMenuKind::Worktrees => {
                return Some(self.render_worktree_menu(menu.position, cx));
            }
            SidebarMenuKind::AddProject => {
                return Some(self.render_add_project_menu(menu.position, cx));
            }
            SidebarMenuKind::CompactProjects => {
                return Some(self.render_compact_projects_menu(menu.position, cx));
            }
            _ => {}
        }
        let entries = self.sidebar_menu_entries(&menu.kind);
        let header = match &menu.kind {
            SidebarMenuKind::Folder(folder_id) => self.render_folder_swatches(folder_id, cx),
            _ => None,
        };
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "sidebar-menu",
                entries: &entries,
                active: menu.active,
                place: MenuPlace::At(menu.position),
                width: if header.is_some() {
                    FOLDER_MENU_WIDTH
                } else {
                    MENU_WIDTH
                },
                focus: &self.composer_menus.focus,
                header,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.sidebar_menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("sidebar menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_sidebar_menu(ix, cx)) {
                    log::debug!("sidebar menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                if let Err(err) = close_app.update(cx, |this, cx| this.close_sidebar_menu(cx)) {
                    log::debug!("sidebar menu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }
}
