//! MonoCode `useProjectMenu`: opening a project's (or a project group's)
//! menu, its name, colour and mascot edits, its actions and the submenus
//! they open. Drawing it is in `menu_view`, removal in `remove`.

use std::sync::atomic::{AtomicU64, Ordering};

use ely_gpui_component::primitives::IconName;
use gpui::{Bounds, Context, Pixels, Point};

use super::menu_view::{ExtraItem, submenu_origin};
use super::model::{ProjectGroup, mute_actions, mute_deadline, notification_id};
use super::project_name;
use super::state::{MenuTarget, RailMenu, RailSubmenu, SubmenuKind};
use crate::app::BenCodeApp;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry};
use crate::ui::icons::ExtraIcon;

/// MonoCode `REVEAL_LABEL`.
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};

static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

impl BenCodeApp {
    pub fn rail_menu_open(&self) -> bool {
        let rail = &self.rail_ui;
        rail.menu.is_some() || rail.inbox_menu.is_some() || rail.mute_picker.is_some()
    }

    /// MonoCode `projectMenu.open`: the name field takes focus, selected.
    pub(crate) fn open_rail_project_menu(&mut self, path: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        let label = self.rail_project_label(path);
        self.open_rail_menu(MenuTarget::Project(path.to_string()), label, at, cx);
    }

    pub(crate) fn open_rail_group_menu(&mut self, id: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(group) = self.settings.rail.project_groups.iter().find(|g| g.id == id) else {
            return;
        };
        let name = group.name.clone();
        self.open_rail_menu(MenuTarget::Group(id.to_string()), name, at, cx);
    }

    fn open_rail_menu(&mut self, target: MenuTarget, name: String, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.close_sidebar_menu(cx);
        let rail = &mut self.rail_ui;
        rail.menu = Some(RailMenu { target, position: at });
        rail.custom_color_open = false;
        rail.menu_error = None;
        let len = name.len();
        rail.name_input.update(cx, |input, cx| {
            input.set_text(name, cx);
            input.select(0..len, cx);
        });
        let handle = gpui::Focusable::focus_handle(self.rail_ui.name_input.read(cx), cx);
        crate::ui::composer::focus_later(handle, cx);
        cx.notify();
    }

    /// MonoCode `projectMenu.createGroup`: a new group, `project` moved
    /// into it, and the group's menu open where the click was.
    pub(crate) fn create_rail_group(&mut self, at: Point<Pixels>, project: Option<String>, cx: &mut Context<Self>) {
        let id = format!(
            "group-{:x}-{:x}",
            crate::app::now_ms(),
            NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
        );
        let group_id = id.clone();
        self.update_rail_prefs(
            |prefs| {
                let next = prefs.with_new_group(group_id.clone());
                match &project {
                    Some(path) => next.with_assignment(path, Some(&group_id)),
                    None => next,
                }
            },
            cx,
        );
        self.open_rail_group_menu(&id, at, cx);
    }

    /// Closes the project/group menu, the Inbox menu and the date picker.
    pub fn close_rail_menu(&mut self, cx: &mut Context<Self>) -> bool {
        self.commit_rail_menu_name(cx);
        let rail = &mut self.rail_ui;
        let was_open = rail.menu.take().is_some()
            | rail.inbox_menu.take().is_some()
            | rail.mute_picker.take().is_some();
        rail.submenu = None;
        rail.custom_color_open = false;
        rail.menu_error = None;
        if was_open {
            cx.notify();
        }
        was_open
    }

    /// MonoCode `onRename` (on blur and Enter): a project's label, a
    /// group's name (a blank one keeps the old name).
    pub(crate) fn commit_rail_menu_name(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.rail_ui.menu.as_ref() else {
            return;
        };
        let text = self.rail_ui.name_input.read(cx).text().trim().to_string();
        match menu.target.clone() {
            MenuTarget::Project(path) => {
                if text != self.rail_project_label(&path) {
                    self.update_rail_prefs(|prefs| prefs.with_label(&path, &text), cx);
                }
            }
            MenuTarget::Group(id) => {
                let unchanged = self
                    .settings
                    .rail
                    .project_groups
                    .iter()
                    .any(|g| g.id == id && (g.name == text || text.is_empty()));
                if !unchanged {
                    self.update_rail_prefs(
                        |prefs| {
                            prefs.with_group(&id, |g| ProjectGroup {
                                name: text.clone(),
                                ..g.clone()
                            })
                        },
                        cx,
                    );
                }
            }
        }
    }

    /// Keys while a rail menu is open: Esc closes a submenu, then the menu;
    /// the Inbox menu's ↑/↓/Enter (MonoCode `ExplorerMenu`).
    pub fn rail_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if key == "escape" {
            if self.rail_ui.submenu.take().is_some() {
                cx.notify();
                return true;
            }
            let closed = self.close_rail_menu(cx);
            self.refocus_prompt(cx);
            return closed;
        }
        if let Some(submenu) = self.rail_ui.submenu.clone() {
            let entries = self.rail_submenu_entries(submenu.kind);
            return self.step_or_pick(key, &entries, submenu.active, cx, |this, ix, cx| {
                this.pick_rail_submenu(ix, cx)
            }, |this, ix| {
                if let Some(s) = this.rail_ui.submenu.as_mut() {
                    s.active = ix;
                }
            });
        }
        if self.rail_ui.inbox_menu.is_some() {
            let entries = self.inbox_menu_entries();
            let active = self.rail_ui.inbox_active;
            return self.step_or_pick(key, &entries, active, cx, |this, ix, cx| {
                this.pick_inbox_menu(ix, cx)
            }, |this, ix| this.rail_ui.inbox_active = ix);
        }
        false
    }

    fn step_or_pick(
        &mut self,
        key: &str,
        entries: &[MenuEntry],
        active: usize,
        cx: &mut Context<Self>,
        pick: impl FnOnce(&mut Self, usize, &mut Context<Self>),
        set: impl FnOnce(&mut Self, usize),
    ) -> bool {
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                set(self, explorer_menu::step(entries, active, dir));
            }
            "enter" | "space" => pick(self, active, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// MonoCode `resolveTabGroupLabel` with the folder name as fallback.
    pub(crate) fn rail_project_label(&self, path: &str) -> String {
        self.settings.rail.label(path, project_name(path)).to_string()
    }

    /// MonoCode `projectMenuExtraItems`. Background image and Notification
    /// settings are left out: BenCode has neither per-project chat
    /// backgrounds nor a notification settings page.
    pub(super) fn project_extra_items(&self, path: &str) -> Vec<ExtraItem> {
        let pinned = self
            .settings
            .pinned_projects
            .iter()
            .any(|p| crate::app::same_project_path(p, path));
        let pin = if pinned {
            ExtraItem::new("unpin", "Unpin project", IconName::PinOff)
        } else {
            ExtraItem::new("pin", "Pin project", IconName::Pin)
        };
        vec![
            ExtraItem {
                submenu: Some(SubmenuKind::MoveToGroup),
                ..ExtraItem::new("project-group", "Move to group", ExtraIcon::FolderTree)
            },
            pin,
            ExtraItem::new("reveal", REVEAL_LABEL, IconName::FolderOpen),
            ExtraItem {
                submenu: Some(SubmenuKind::Editor),
                disabled: self.integrations.editors.is_none(),
                ..ExtraItem::new("external-editor", "Open in editor", IconName::AppWindow)
            },
            ExtraItem {
                submenu: Some(SubmenuKind::Mute),
                sep_before: true,
                ..ExtraItem::new("notifications-mute", "Mute notifications", ExtraIcon::BellOff)
            },
            ExtraItem {
                sep_before: true,
                ..ExtraItem::new("archive", "Archive", IconName::Archive)
            },
            ExtraItem {
                danger: true,
                ..ExtraItem::new("delete", "Delete", IconName::Trash2)
            },
        ]
    }

    pub(super) fn group_extra_items() -> Vec<ExtraItem> {
        vec![ExtraItem {
            description: Some("Projects will become ungrouped".into()),
            danger: true,
            ..ExtraItem::new("delete-project-group", "Delete group", IconName::Trash2)
        }]
    }

    /// The rows of a submenu (MonoCode `groupSubmenu`, the editor list,
    /// `notificationMuteActions`).
    pub(super) fn rail_submenu_entries(&self, kind: SubmenuKind) -> Vec<MenuEntry> {
        match kind {
            SubmenuKind::MoveToGroup => {
                let path = match self.rail_ui.menu.as_ref().map(|m| &m.target) {
                    Some(MenuTarget::Project(path)) => path.clone(),
                    _ => return Vec::new(),
                };
                let prefs = &self.settings.rail;
                let current = prefs.group_of(&path).map(|g| g.id.clone());
                let groups = &prefs.project_groups;
                let mut entries = vec![MenuEntry::Item(MenuAction::new("project-group:new", "New group…"))];
                if !groups.is_empty() {
                    entries.push(MenuEntry::Separator);
                }
                entries.extend(groups.iter().map(|g| {
                    MenuEntry::Item(
                        MenuAction::new("project-group", g.name.clone())
                            .checked(current.as_deref() == Some(g.id.as_str()))
                            .value(g.id.clone()),
                    )
                }));
                if !groups.is_empty() {
                    entries.push(MenuEntry::Separator);
                }
                entries.push(MenuEntry::Item(
                    MenuAction::new("project-group:none", "Ungrouped").checked(current.is_none()),
                ));
                entries
            }
            SubmenuKind::Editor => match &self.integrations.editors {
                None => vec![MenuEntry::Item(
                    MenuAction::new("external-editor:loading", "Looking for editors…").disabled(true),
                )],
                Some(editors) if editors.is_empty() => vec![MenuEntry::Item(
                    MenuAction::new("external-editor:none", "No supported editors found").disabled(true),
                )],
                Some(editors) => editors
                    .iter()
                    .map(|e| MenuEntry::Item(MenuAction::new("external-editor", e.name).value(e.id)))
                    .collect(),
            },
            SubmenuKind::Mute | SubmenuKind::InboxMute => mute_actions(crate::app::now_ms())
                .into_iter()
                .map(|a| MenuEntry::Item(MenuAction::new(a.id, a.label)))
                .collect(),
        }
    }

    pub(super) fn open_rail_submenu(&mut self, kind: SubmenuKind, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self
            .rail_ui
            .submenu
            .as_ref()
            .is_some_and(|s| s.kind == kind && s.position == position)
        {
            return;
        }
        let active = explorer_menu::first_item(&self.rail_submenu_entries(kind));
        self.rail_ui.submenu = Some(RailSubmenu { kind, position, active });
        cx.notify();
    }

    pub(super) fn pick_rail_submenu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(submenu) = self.rail_ui.submenu.clone() else {
            return;
        };
        let entries = self.rail_submenu_entries(submenu.kind);
        let Some(action) = explorer_menu::pick_action(&entries, index).cloned() else {
            return;
        };
        if submenu.kind == SubmenuKind::InboxMute {
            let ids = self.rail_notification_ids();
            self.apply_mute_action(action.id, ids, "All projects".into(), submenu.position, cx);
            return;
        }
        let Some(RailMenu {
            target: MenuTarget::Project(path),
            position,
        }) = self.rail_ui.menu.clone()
        else {
            return;
        };
        match action.id {
            "project-group:new" => {
                self.close_rail_menu(cx);
                self.create_rail_group(position, Some(path), cx);
            }
            "project-group:none" => {
                self.update_rail_prefs(|prefs| prefs.with_assignment(&path, None), cx);
                self.close_rail_menu(cx);
            }
            "project-group" => {
                if let Some(group) = action.value.as_deref() {
                    self.update_rail_prefs(|prefs| prefs.with_assignment(&path, Some(group)), cx);
                }
                self.close_rail_menu(cx);
            }
            "external-editor" => {
                if let Some(editor) = action.value.as_deref() {
                    self.open_project_in_named_editor(&path, editor, cx);
                }
            }
            id => {
                let ids = vec![notification_id(&path)];
                let title = self.rail_project_label(&path);
                self.apply_mute_action(id, ids, title, position, cx);
            }
        }
    }

    /// A `mute:*` pick: a preset mutes now, "Choose date and time" opens
    /// the picker where the menu was.
    fn apply_mute_action(
        &mut self,
        id: &str,
        ids: Vec<String>,
        title: String,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if id == "mute:custom" {
            self.close_rail_menu(cx);
            self.open_mute_picker(ids, title, at, cx);
            return;
        }
        let now = crate::app::now_ms();
        if let Some(until) = mute_deadline(id, now) {
            self.update_rail_prefs(|prefs| prefs.with_mute(&ids, Some(until), now), cx);
        }
        self.close_rail_menu(cx);
    }

    /// MonoCode `onExtraPick`; returns whether the menu stays open.
    pub(super) fn pick_rail_extra(&mut self, id: &'static str, anchor: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
        let Some(menu) = self.rail_ui.menu.clone() else {
            return;
        };
        match (menu.target, id) {
            (MenuTarget::Group(group), "delete-project-group") => {
                self.rail_ui.menu = None;
                self.update_rail_prefs(|prefs| prefs.without_group(&group), cx);
                self.close_rail_menu(cx);
            }
            (MenuTarget::Project(path), _) => self.pick_project_extra(&path, id, anchor, cx),
            _ => {}
        }
    }

    fn pick_project_extra(
        &mut self,
        path: &str,
        id: &'static str,
        anchor: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let submenu = match id {
            "project-group" => Some(SubmenuKind::MoveToGroup),
            "external-editor" => Some(SubmenuKind::Editor),
            "notifications-mute" => Some(SubmenuKind::Mute),
            _ => None,
        };
        if let Some(kind) = submenu {
            if let Some(bounds) = anchor {
                self.open_rail_submenu(kind, submenu_origin(bounds), cx);
            }
            return;
        }
        let now = crate::app::now_ms();
        match id {
            "notifications-resume" => {
                let ids = vec![notification_id(path)];
                self.update_rail_prefs(|prefs| prefs.with_mute(&ids, None, now), cx);
            }
            "pin" | "unpin" => self.toggle_project_pin(path, cx),
            "reveal" => super::reveal_project(path),
            "archive" => self.archive_rail_project(path, cx),
            "delete" => self.request_remove_project(path, cx),
            _ => {}
        }
        self.close_rail_menu(cx);
    }

    fn open_project_in_named_editor(&mut self, path: &str, editor_id: &str, cx: &mut Context<Self>) {
        let Some(editor) = self
            .integrations
            .editors
            .as_ref()
            .and_then(|eds| eds.iter().find(|e| e.id == editor_id))
            .cloned()
        else {
            return;
        };
        let cwd = path.to_string();
        let task = cx.background_executor().spawn(async move {
            crate::external_editor::launch_external_editor_with(&editor, &cwd, None, None)
                .map_err(|err| format!("{err:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let shown = this.update(cx, |this, cx| match result {
                // MonoCode closes the menu once the editor opened.
                Ok(()) => {
                    this.close_rail_menu(cx);
                }
                Err(err) => {
                    log::warn!("failed to launch external editor: {err}");
                    this.rail_ui.menu_error = Some(err);
                    cx.notify();
                }
            });
            if let Err(err) = shown {
                log::debug!("editor launch finished after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub(super) fn pick_project_logo(&mut self, path: &str, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose Logo".into()),
        });
        let project = path.to_string();
        self.close_rail_menu(cx);
        cx.spawn(async move |this, cx| {
            let file = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(err)) => {
                    log::error!("logo picker failed: {err:#}");
                    None
                }
            };
            let Some(file) = file else {
                return;
            };
            let file = file.to_string_lossy().into_owned();
            let saved = this.update(cx, |this, cx| {
                this.update_rail_prefs(|prefs| prefs.with_logo(&project, Some(&file)), cx);
            });
            if let Err(err) = saved {
                log::debug!("logo picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub(super) fn set_rail_color(&mut self, target: &MenuTarget, index: Option<usize>, cx: &mut Context<Self>) {
        match target {
            MenuTarget::Project(path) => self.update_rail_prefs(|p| p.with_color(path, index), cx),
            MenuTarget::Group(id) => self.update_rail_prefs(
                |p| {
                    p.with_group(id, |g| ProjectGroup {
                        color_index: index,
                        custom_color: None,
                        ..g.clone()
                    })
                },
                cx,
            ),
        }
    }

    pub(super) fn set_rail_custom_color(&mut self, target: &MenuTarget, hex: &str, cx: &mut Context<Self>) {
        match target {
            MenuTarget::Project(path) => self.update_rail_prefs(|p| p.with_custom_color(path, hex), cx),
            MenuTarget::Group(id) => self.update_rail_prefs(
                |p| {
                    p.with_group(id, |g| ProjectGroup {
                        color_index: None,
                        custom_color: Some(hex.to_lowercase()),
                        ..g.clone()
                    })
                },
                cx,
            ),
        }
    }

    pub(super) fn set_rail_mascot(&mut self, target: &MenuTarget, name: &str, cx: &mut Context<Self>) {
        match target {
            MenuTarget::Project(path) => self.update_rail_prefs(|p| p.with_mascot(path, Some(name)), cx),
            MenuTarget::Group(id) => self.update_rail_prefs(
                |p| {
                    p.with_group(id, |g| ProjectGroup {
                        mascot: Some(name.to_string()),
                        ..g.clone()
                    })
                },
                cx,
            ),
        }
    }

}
