//! The Changes panel's two dropdowns, drawn the way MonoCode draws them
//! (its own `role="menu"` lists, not a library dropdown): "Commit
//! options" under the Commit button's arrow, and "Branch actions" under
//! the header's `…`.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::IconSize;
use gpui::{
    AnyElement, Context, Div, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, SharedString, Stateful, Styled, canvas, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuStyle, MenuView};
use crate::ui::git_changes_panel::{Busy, PendingCommit};
use crate::ui::scale::px;

/// MonoCode `min-w-48` (Commit options) and `min-w-36` (Branch actions).
const COMMIT_MENU_WIDTH: f32 = 192.0;
const BRANCH_MENU_WIDTH: f32 = 144.0;
/// MonoCode `mt-1` between a trigger and its menu.
const MENU_GAP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitMenuKind {
    Commit,
    Branch,
}

#[derive(Clone, Debug)]
pub struct GitMenu {
    pub kind: GitMenuKind,
    pub position: Point<Pixels>,
    pub active: usize,
}

impl GitMenuKind {
    /// The menu's floor; it grows with its rows (`min-w-*`).
    fn width(self) -> f32 {
        match self {
            GitMenuKind::Commit => COMMIT_MENU_WIDTH,
            GitMenuKind::Branch => BRANCH_MENU_WIDTH,
        }
    }
}

impl BenCodeApp {
    pub fn git_menu_open(&self) -> bool {
        self.changes_ui.menu.is_some()
    }

    fn git_menu_entries(&self, kind: GitMenuKind, cx: &gpui::App) -> Vec<MenuEntry> {
        match kind {
            GitMenuKind::Commit => {
                let (push, push_pr) = self.commit_push_allowed(cx);
                vec![
                    MenuEntry::Item(MenuAction::new("commit-push", "Commit & Push").disabled(!push)),
                    MenuEntry::Item(
                        MenuAction::new("commit-push-pr", "Commit, Push & Create PR").disabled(!push_pr),
                    ),
                    MenuEntry::Separator,
                    MenuEntry::Item(
                        MenuAction::new("amend", "Amend Last Commit").checked(self.changes_ui.amend.is_some()),
                    ),
                ]
            }
            GitMenuKind::Branch => {
                let sync = &self.git_sync;
                let can_pull = sync.remote.is_some() && sync.upstream.is_some();
                let pulling = self.changes_ui.busy == Some(Busy::Pull);
                // MonoCode's `title` on the disabled row.
                let why = (!can_pull)
                    .then(|| "This branch needs a remote and upstream before it can pull".to_string());
                let icon = if pulling { IconName::LoaderCircle } else { IconName::RefreshCw };
                vec![MenuEntry::Item(
                    MenuAction::new("pull", if pulling { "Pulling…" } else { "Pull" })
                        .icon(icon, pulling)
                        .disabled(self.changes_ui.busy.is_some() || !can_pull)
                        .tooltip(why),
                )]
            }
        }
    }

    /// Opens `kind` under its trigger, right-aligned like MonoCode's
    /// `absolute top-full right-0`; a second press closes it.
    fn toggle_git_menu(&mut self, kind: GitMenuKind, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.changes_ui.menu_trigger_hit = true;
        if self.changes_ui.menu.as_ref().is_some_and(|m| m.kind == kind) {
            self.changes_ui.menu = None;
            cx.notify();
            return;
        }
        let entries = self.git_menu_entries(kind, cx);
        self.changes_ui.menu = Some(GitMenu {
            kind,
            position: match self.changes_ui.menu_anchor(kind).get() {
                Some(bounds) => Point::new(bounds.right(), bounds.bottom() + px(MENU_GAP)),
                None => Point::new(at.x + px(10.0), at.y + px(14.0)),
            },
            active: explorer_menu::first_item(&entries),
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub fn close_git_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.changes_ui.menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn pick_git_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.changes_ui.menu.clone() else {
            return;
        };
        let entries = self.git_menu_entries(menu.kind, cx);
        let Some(id) = explorer_menu::pick(&entries, index) else {
            return;
        };
        self.changes_ui.menu = None;
        self.refocus_prompt(cx);
        match id {
            "commit-push" => self.commit_from_panel(PendingCommit { push: true, pr: false }, false, false, cx),
            "commit-push-pr" => self.commit_from_panel(PendingCommit { push: true, pr: true }, false, false, cx),
            "amend" => self.toggle_amend(cx),
            "pull" => self.pull_changes(cx),
            _ => {}
        }
        cx.notify();
    }

    /// Keys while a Changes menu holds focus.
    pub fn git_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.changes_ui.menu.clone() else {
            return false;
        };
        let entries = self.git_menu_entries(menu.kind, cx);
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.changes_ui.menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, menu.active, dir);
                }
            }
            "enter" | "space" => self.pick_git_menu(menu.active, cx),
            "escape" => {
                self.close_git_menu(cx);
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub fn render_git_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.changes_ui.menu.as_ref()?;
        let entries = self.git_menu_entries(menu.kind, cx);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu_styled(
            MenuView {
                id: "git-menu",
                entries: &entries,
                active: menu.active,
                place: MenuPlace::UnderRight(menu.position),
                width: menu.kind.width(),
                focus: &self.composer_menus.focus,
                header: None,
            },
            MenuStyle::Changes,
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.changes_ui.menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("git menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_git_menu(ix, cx)) {
                    log::debug!("git menu pick after app drop: {err:#}");
                }
            },
            // A press on the trigger toggles the menu itself; only once
            // every handler ran is it known whether it was outside.
            move |window, cx| {
                let updated = close_app.update(cx, |_, cx| {
                    cx.defer_in(window, |this, _, cx| {
                        if !std::mem::take(&mut this.changes_ui.menu_trigger_hit) {
                            this.close_git_menu(cx);
                        }
                    });
                });
                if let Err(err) = updated {
                    log::debug!("git menu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }

    /// A dropdown trigger filling the box its caller sizes and rounds:
    /// `look`'s icon over `look.fill`, lit while its menu is open
    /// (`aria-expanded:…`), inert when disabled. It records its bounds so
    /// the menu opens under it (MonoCode `absolute top-full right-0 mt-1`).
    pub(crate) fn git_menu_trigger(
        &self,
        kind: GitMenuKind,
        look: TriggerLook,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let open = self.changes_ui.menu.as_ref().is_some_and(|m| m.kind == kind);
        let (id, tip) = match kind {
            GitMenuKind::Commit => ("git-commit-options", "Commit options"),
            GitMenuKind::Branch => ("git-branch-actions", "Branch actions"),
        };
        let anchor = self.changes_ui.menu_anchor(kind);
        let icon = if look.spin {
            crate::ui::git_changes_panel::spinning_icon(
                SharedString::from(format!("{id}-spin")),
                look.icon,
                look.size,
                look.color,
            )
        } else {
            Icon::new(look.icon)
                .size(look.size)
                .color(if open { look.hover_color } else { look.color })
                .group_hover_color(id, look.hover_color)
                .into_any_element()
        };
        // `aria-expanded:` outranks `hover:`.
        let hover = if open { look.open } else { look.hover };
        div()
            .id(SharedString::from(id))
            .group(id)
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .tooltip(Tooltip::text(tip))
            .when_some(look.fill, |el, fill| el.bg(fill))
            .when(!enabled && look.dim_disabled, |el| el.opacity(0.4))
            .when(open, |el| el.bg(look.open))
            .when(enabled, move |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.toggle_git_menu(kind, event.position, cx);
                        }),
                    )
            })
            .child(
                canvas(move |bounds, _, _| anchor.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(icon)
    }
}

/// How a [`BenCodeApp::git_menu_trigger`] looks.
pub(crate) struct TriggerLook {
    pub icon: IconName,
    pub size: IconSize,
    /// Spun like MonoCode's `animate-spin` loader.
    pub spin: bool,
    /// The icon's colour, and its colour on hover or while open.
    pub color: Hsla,
    pub hover_color: Hsla,
    /// The resting fill, if any.
    pub fill: Option<Hsla>,
    pub hover: Hsla,
    pub open: Hsla,
    /// MonoCode `disabled:opacity-40` (the Commit arrow only stops
    /// taking the pointer).
    pub dim_disabled: bool,
}

