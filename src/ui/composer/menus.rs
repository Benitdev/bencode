//! Shared behaviour of the composer's popovers (MonoCode `Popover`): the
//! open menu takes keyboard focus, a mouse-down outside every popover and
//! its chip dismisses it, and closing with Esc or a pick hands focus back
//! to the prompt.

use gpui::{App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, Window};

use super::PERMISSION_MODES;
use super::model_picker::{ModelTab, Submenu};
use crate::app::BenCodeApp;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuView};

/// MonoCode `WorkspacePicker` popover width.
const WORKSPACE_MENU_WIDTH: f32 = 240.0;

pub struct MenuState {
    /// Focused while a keyboard-driven menu (access, model) is open.
    pub focus: FocusHandle,
    /// Highlighted row of the access picker.
    pub access_index: usize,
    /// Model menu: highlighted row, open flyout, highlighted choice in a
    /// setting flyout, and the provider tab of the models flyout.
    pub model_entry: usize,
    pub model_submenu: Option<Submenu>,
    pub setting_index: usize,
    pub model_tab: ModelTab,
    /// The context meter's card, pinned open by a click.
    pub context_pinned: bool,
    /// The checkout menu of a new thread, and its highlighted row.
    pub workspace_menu: Option<usize>,
    /// The recent-models menu (⌘.) and its highlighted row.
    pub recent_open: bool,
    pub recent_index: usize,
    /// A finished turn's Handoff menu.
    pub handoff: Option<super::handoff::HandoffMenu>,
    /// A mouse-down this dispatch landed inside a popover or its chip.
    click_inside: bool,
    /// The outside-click check for this dispatch is already queued.
    check_queued: bool,
}

impl MenuState {
    pub fn new(focus: FocusHandle) -> Self {
        Self {
            focus,
            access_index: 0,
            model_entry: 0,
            model_submenu: None,
            setting_index: 0,
            model_tab: ModelTab::default(),
            context_pinned: false,
            workspace_menu: None,
            recent_open: false,
            recent_index: 0,
            handoff: None,
            click_inside: false,
            check_queued: false,
        }
    }
}

/// Moves window focus to `handle` once the current update ends.
pub fn focus_later(handle: FocusHandle, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window()
            && let Err(err) = window.update(cx, |_, window, cx| window.focus(&handle, cx))
        {
            log::debug!("composer: could not move focus: {err:#}");
        }
    });
}

/// Draws a chip's popover over everything (MonoCode `Popover` portals to
/// the body); painted in place, whatever comes later in the composer would
/// show through it.
pub(super) fn over_composer(popover: impl IntoElement) -> gpui::Deferred {
    gpui::deferred(popover).with_priority(3)
}

/// Makes `el` part of the open popover: mouse-downs on it never dismiss,
/// mouse-downs outside it do unless they land on another part.
pub(super) fn popover_surface<E: InteractiveElement>(el: E, cx: &Context<BenCodeApp>) -> E {
    popover_anchor(el, cx).on_mouse_down_out(cx.listener(|this, _, window, cx| {
        this.queue_outside_check(window, cx);
    }))
}

/// A chip that toggles a popover: clicking it never counts as outside, so
/// its own click can close the popover instead of reopening it.
pub(super) fn popover_anchor<E: InteractiveElement>(el: E, cx: &Context<BenCodeApp>) -> E {
    el.capture_any_mouse_down(cx.listener(|this, _, window, cx| {
        this.composer_menus.click_inside = true;
        this.queue_outside_check(window, cx);
    }))
}

impl BenCodeApp {
    /// Every part of the popover has seen the mouse-down once the dispatch
    /// ends; only then is it known whether the press was outside them all.
    fn queue_outside_check(&mut self, window: &Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.composer_menus.check_queued, true) {
            return;
        }
        cx.defer_in(window, |this, _, cx| {
            this.composer_menus.check_queued = false;
            if !std::mem::take(&mut this.composer_menus.click_inside) {
                this.close_composer_popovers(cx);
            }
        });
    }

    pub fn focus_composer_menu(&self, cx: &mut App) {
        focus_later(self.composer_menus.focus.clone(), cx);
    }

    pub fn refocus_prompt(&self, cx: &mut App) {
        focus_later(self.prompt_input.read(cx).focus_handle(cx), cx);
    }

    /// Keys while a composer menu holds focus (MonoCode `onMenuKey`).
    pub fn handle_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.git_menu_open() {
            self.git_menu_key(key, cx)
        } else if self.tree_menu_open() {
            self.tree_menu_key(key, cx)
        } else if self.sidebar_menu_open() {
            self.sidebar_menu_key(key, cx)
        } else if self.rail_menu_open() {
            self.rail_menu_key(key, cx)
        } else if self.tab_menu_open() {
            self.tab_menu_key(key, cx)
        } else if self.composer_menus.handoff.is_some() {
            self.handoff_menu_key(key, cx)
        } else if self.composer_menus.workspace_menu.is_some() {
            self.workspace_menu_key(key, cx)
        } else if self.is_permission_picker_open {
            self.access_menu_key(key, cx)
        } else if self.is_model_picker_open {
            self.model_menu_key(key, cx)
        } else if self.composer_menus.recent_open {
            self.recent_menu_key(key, cx)
        } else {
            false
        }
    }

    /// MonoCode `AccessPicker`: ↑/↓ stop at the ends, Enter picks, Esc
    /// closes; both hand focus back to the prompt.
    fn access_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let last = PERMISSION_MODES.len() - 1;
        let index = &mut self.composer_menus.access_index;
        match key {
            "down" => *index = (*index + 1).min(last),
            "up" => *index = index.saturating_sub(1),
            "enter" => {
                let mode = PERMISSION_MODES[(*index).min(last)].0;
                self.is_permission_picker_open = false;
                self.set_permission_mode(mode, cx);
                self.refocus_prompt(cx);
            }
            "escape" => {
                self.is_permission_picker_open = false;
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }
}

impl BenCodeApp {
    /// MonoCode `WorkspacePicker` rows: the project checkout, a new
    /// worktree (made on the first send), then each existing worktree by
    /// branch with its folder underneath.
    fn workspace_menu_entries(&self) -> Vec<MenuEntry> {
        let focused = self.worktree_focus().map(|f| f.path.clone());
        let new_tree = self.new_worktree_base().is_some();
        let mut entries = vec![
            MenuEntry::Item(
                MenuAction::new("current", "Current checkout")
                    .checked(!new_tree && focused.is_none()),
            ),
            MenuEntry::Item(
                MenuAction::new("new-worktree", "New worktree")
                    .shortcut("⌘⇧G")
                    .checked(new_tree),
            ),
        ];
        let trees: Vec<_> = self
            .workspace
            .worktrees
            .iter()
            .filter(|w| !w.is_main && !w.missing)
            .collect();
        if !trees.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        entries.extend(trees.into_iter().map(|tree| {
            MenuEntry::Item(
                MenuAction::new(
                    "tree",
                    tree.branch.clone().unwrap_or_else(|| tree.head.clone()),
                )
                .description(Some(tree.path.clone()))
                .checked(!new_tree && focused.as_deref() == Some(tree.path.as_str())),
            )
        }));
        entries
    }

    pub(super) fn toggle_workspace_menu(&mut self, cx: &mut Context<Self>) {
        if self.composer_menus.workspace_menu.is_some() {
            self.composer_menus.workspace_menu = None;
            self.refocus_prompt(cx);
        } else {
            self.close_composer_popovers(cx);
            self.composer_menus.workspace_menu =
                Some(explorer_menu::first_item(&self.workspace_menu_entries()));
            self.focus_composer_menu(cx);
        }
        cx.notify();
    }

    /// Row `index` picked: the project checkout, a new worktree, or that
    /// existing worktree.
    fn pick_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        self.composer_menus.workspace_menu = None;
        self.refocus_prompt(cx);
        let entries = self.workspace_menu_entries();
        let Some(MenuEntry::Item(action)) = entries.get(index) else {
            return;
        };
        match action.id {
            "new-worktree" => self.set_new_worktree(true, cx),
            id => {
                self.set_new_worktree(false, cx);
                let focus = action
                    .description
                    .as_deref()
                    .filter(|_| id == "tree")
                    .and_then(|path| self.workspace.worktrees.iter().find(|w| w.path == path))
                    .map(|tree| crate::app::WorktreeFocus {
                        path: tree.path.clone(),
                        branch: tree.branch.clone(),
                    });
                self.select_workspace(focus, cx);
            }
        }
        cx.notify();
    }

    fn workspace_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.composer_menus.workspace_menu else {
            return false;
        };
        let entries = self.workspace_menu_entries();
        match key {
            "down" => {
                self.composer_menus.workspace_menu = Some(explorer_menu::step(&entries, active, 1))
            }
            "up" => {
                self.composer_menus.workspace_menu = Some(explorer_menu::step(&entries, active, -1))
            }
            "enter" | "space" => self.pick_workspace(active, cx),
            "escape" => {
                self.composer_menus.workspace_menu = None;
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn render_workspace_menu(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let entries = self.workspace_menu_entries();
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        explorer_menu::render_menu(
            MenuView {
                id: "composer-workspace-menu",
                entries: &entries,
                active: self.composer_menus.workspace_menu.unwrap_or(0),
                place: MenuPlace::Above,
                width: WORKSPACE_MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let hovered = hover_app.update(cx, |this, cx| {
                    if this.composer_menus.workspace_menu != Some(ix) {
                        this.composer_menus.workspace_menu = Some(ix);
                        cx.notify();
                    }
                });
                if let Err(err) = hovered {
                    log::debug!("workspace menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_workspace(ix, cx)) {
                    log::debug!("workspace pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                let closed = close_app.update(cx, |this, cx| {
                    this.composer_menus.workspace_menu = None;
                    cx.notify();
                });
                if let Err(err) = closed {
                    log::debug!("workspace menu dismiss after app drop: {err:#}");
                }
            },
            cx,
        )
    }
}
