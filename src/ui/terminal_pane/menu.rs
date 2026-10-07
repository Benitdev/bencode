//! The terminal dock's menus, both MonoCode `ExplorerMenu`s: a tab's right
//! click (`surfaceTabMenuItems` for a terminal: Close, Close Others) and
//! Move Terminal (`SIDE_ITEMS`).

use gpui::{AnyElement, Context, Pixels, Point};

use super::DockSide;
use crate::app::BenCodeApp;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry};

const MENU_WIDTH: f32 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuTarget {
    /// A tab's menu, by terminal id.
    Tab(u64),
    /// Move Terminal.
    Side,
}

pub(super) struct TerminalMenu {
    target: MenuTarget,
    position: Point<Pixels>,
    active: usize,
}

impl TerminalMenu {
    pub(super) fn is_side(&self) -> bool {
        self.target == MenuTarget::Side
    }
}

impl BenCodeApp {
    fn terminal_menu_entries(&self, target: MenuTarget) -> Vec<MenuEntry> {
        match target {
            MenuTarget::Tab(_) => {
                let tabs = self.terminals.dock(&self.current_cwd).map_or(0, |dock| dock.tabs.len());
                vec![
                    MenuEntry::Item(MenuAction::new("close", "Close")),
                    MenuEntry::Item(MenuAction::new("close-others", "Close Others").disabled(tabs < 2)),
                ]
            }
            MenuTarget::Side => {
                let current = self.terminals.layout(&self.current_cwd).side;
                DockSide::ALL
                    .into_iter()
                    .map(|side| {
                        MenuEntry::Item(MenuAction::new(side.id(), side.label()).checked(side == current))
                    })
                    .collect()
            }
        }
    }

    fn open_terminal_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        let entries = self.terminal_menu_entries(target);
        self.terminals.menu = Some(TerminalMenu {
            target,
            position,
            active: explorer_menu::first_item(&entries),
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub(super) fn open_terminal_tab_menu(&mut self, id: u64, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.open_terminal_menu(MenuTarget::Tab(id), position, cx);
    }

    /// Move Terminal: opens under the pointer, or closes when it is open.
    pub(super) fn toggle_dock_side_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.terminals.menu.as_ref().is_some_and(TerminalMenu::is_side) {
            self.close_terminal_menu(cx);
            return;
        }
        self.open_terminal_menu(MenuTarget::Side, position, cx);
    }

    pub fn terminal_menu_open(&self) -> bool {
        self.terminals.menu.is_some()
    }

    pub fn close_terminal_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.terminals.menu.take().is_none() {
            return false;
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    fn pick_terminal_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.terminals.menu.take() else {
            return;
        };
        let entries = self.terminal_menu_entries(menu.target);
        let Some(id) = explorer_menu::pick(&entries, index) else {
            self.terminals.menu = Some(menu);
            return;
        };
        self.refocus_prompt(cx);
        match (menu.target, id) {
            (MenuTarget::Tab(tab), "close") => {
                let project = self.current_cwd.clone();
                self.close_terminal(&project, tab, cx);
            }
            (MenuTarget::Tab(tab), "close-others") => self.close_other_terminals(tab, cx),
            (MenuTarget::Side, id) => {
                if let Some(side) = DockSide::from_id(id) {
                    self.set_dock_side(side, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Keys while a terminal menu holds focus (MonoCode `ExplorerMenu.onMenuKey`).
    pub fn terminal_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.terminals.menu.as_ref() else {
            return false;
        };
        let entries = self.terminal_menu_entries(menu.target);
        let active = menu.active;
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.terminals.menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, active, dir);
                }
            }
            "enter" | "space" => self.pick_terminal_menu(active, cx),
            "escape" => {
                self.close_terminal_menu(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub fn render_terminal_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.terminals.menu.as_ref()?;
        let entries = self.terminal_menu_entries(menu.target);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            explorer_menu::MenuView {
                id: "terminal-menu",
                entries: &entries,
                active: menu.active,
                place: explorer_menu::MenuPlace::At(menu.position),
                width: MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.terminals.menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("terminal menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_terminal_menu(ix, cx)) {
                    log::debug!("terminal menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                let closed = close_app.update(cx, |this, cx| {
                    this.close_terminal_menu(cx);
                });
                if let Err(err) = closed {
                    log::debug!("terminal menu dismiss after app drop: {err:#}");
                }
            },
            cx,
        ))
    }
}
