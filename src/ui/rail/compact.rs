//! MonoCode `CompactProjectRail` and `CompactRailAction` (`Sidebar.tsx`):
//! the column of icons the project rail leaves behind when collapsed with
//! Appearance › "Collapsed project rail" on "Icon rail". It expands the
//! rail, switches project, opens a sidebar tab (as a drawer while the
//! sidebar is hidden) and the views.

use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, Div, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, SharedString, Stateful, Styled, anchored, canvas, deferred, div,
    prelude::*,
};

use super::widgets::project_mascot_icon;
use super::{STROKE_OPACITY, TITLEBAR_HEIGHT, path_key};
use crate::app::{BenCodeApp, SidebarMode, Surface, same_project_path};
use crate::ui::appearance::CollapsedRailMode;
use crate::ui::scale::px;
use crate::ui::thumbnail::{LOGO, thumbnail};
use crate::ui::sidebar_menus::SidebarMenuKind;

/// MonoCode `w-12`.
pub const COMPACT_RAIL_WIDTH: f32 = 48.0;
/// The project popover: `Popover width={230}`, like "Open project".
const PROJECTS_WIDTH: f32 = 230.0;
const POPOVER_GAP: f32 = 6.0;

impl BenCodeApp {
    /// MonoCode `compactRailVisible`: the rail is collapsed to icons.
    /// Settings brings its own navigation instead.
    pub fn compact_rail_active(&self) -> bool {
        self.appearance.collapsed_rail == CollapsedRailMode::Compact
            && !self.is_rail_open
            && self.surface != Some(Surface::Settings)
    }

    /// MonoCode `compactTitleBar`: on macOS the title bar runs above the
    /// icon rail, which is narrower than the traffic lights.
    pub fn compact_title_bar(&self) -> bool {
        cfg!(target_os = "macos") && self.compact_rail_active() && self.surface.is_none()
    }

    /// MonoCode `drawerMode`: beside the icon rail a hidden sidebar opens
    /// for a while instead of staying.
    fn sidebar_drawer_mode(&self) -> bool {
        self.compact_rail_active() && !self.is_sidebar_open
    }

    /// The sidebar shows: opened, or drawn out from the icon rail.
    pub fn sidebar_shown(&self) -> bool {
        self.is_sidebar_open || (self.sidebar_drawer_open && self.sidebar_drawer_mode())
    }

    /// Whether the sidebar on screen is the drawer.
    pub fn sidebar_is_drawer(&self) -> bool {
        !self.is_sidebar_open && self.sidebar_shown()
    }

    /// MonoCode's drawer `pointerdown`: a press outside the drawer closes
    /// it, except on the icon rail, whose buttons toggle it themselves.
    pub fn dismiss_sidebar_drawer(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if !self.sidebar_drawer_open || event.position.x < px(COMPACT_RAIL_WIDTH) {
            return;
        }
        self.sidebar_drawer_open = false;
        cx.notify();
    }

    /// Escape closes the drawer; `false` when there was none.
    pub fn close_sidebar_drawer(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.sidebar_is_drawer() {
            return false;
        }
        self.sidebar_drawer_open = false;
        cx.notify();
        true
    }

    /// MonoCode `openWorkspaceTab` + `onCompactTabPick`: back to the
    /// workspace on that tab; as a drawer the shown tab's icon closes it.
    fn pick_compact_tab(&mut self, tab: SidebarMode, cx: &mut Context<Self>) {
        if self.surface.is_some() {
            self.close_surface(cx);
        }
        if self.sidebar_drawer_mode() {
            self.sidebar_drawer_open = !(self.sidebar_drawer_open && self.sidebar_mode == tab);
        }
        self.show_sidebar(tab, cx);
        if tab == SidebarMode::Changes {
            self.refresh_workspace(cx);
        }
    }

    /// MonoCode `action(active, open)`: a view's icon opens it, or leaves
    /// it when it is the one open.
    fn toggle_surface(&mut self, surface: Surface, cx: &mut Context<Self>) {
        if self.surface_open(surface) {
            self.close_surface(cx);
            return;
        }
        match surface {
            Surface::Search => self.open_search_modal(cx),
            Surface::Inbox => self.open_inbox_modal(cx),
            Surface::Notes => self.open_notes(cx),
            Surface::Automations => self.open_automations(cx),
            Surface::Settings => self.open_settings(cx),
        }
    }

    /// MonoCode `CompactProjectRail`: `sidebar-glass flex h-full w-12
    /// shrink-0 flex-col items-center`, a hairline down its right edge.
    pub fn render_compact_rail(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let glass = self.glass(cx).sidebar(colors.bg, theme.is_dark());
        let title_bar_above = self.compact_title_bar();
        let workspace_active = self.surface.is_none();
        let tab_shown = self.sidebar_shown();
        let has_changes = !self.git_status.staged.is_empty() || !self.git_status.unstaged.is_empty();
        let tabs = self.sidebar_tab_order.iter().map(|&tab| {
            let (id, icon, label) = match tab {
                SidebarMode::Sessions => ("compact-rail-sessions", IconName::MessageSquare, "Sessions"),
                SidebarMode::Files => ("compact-rail-files", IconName::FileCode, "Explorer"),
                SidebarMode::Changes => ("compact-rail-changes", IconName::GitBranch, "Changes"),
            };
            let active = workspace_active && tab_shown && self.sidebar_mode == tab;
            compact_action(id, icon, label, active, tab == SidebarMode::Changes && has_changes, cx)
                .on_click(cx.listener(move |this, _, _, cx| this.pick_compact_tab(tab, cx)))
        });
        let surface_action = |id: &'static str, icon: IconName, label: &'static str, surface: Surface, dot: bool| {
            compact_action(id, icon, label, self.surface_open(surface), dot, cx)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_surface(surface, cx)))
        };
        let unseen = self.inbox_has_unseen();
        div()
            .id("compact-project-rail")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(COMPACT_RAIL_WIDTH))
            .h_full()
            .bg(glass)
            .border_r_1()
            .border_color(fg.opacity(STROKE_OPACITY))
            // Without the title bar above, its height is held (and drags
            // the window): `h-10 w-full shrink-0 border-b border-stroke`.
            .when(!title_bar_above, |el| {
                el.child(
                    self.window_drag_region(div(), cx)
                        .flex_none()
                        .w_full()
                        .h(px(TITLEBAR_HEIGHT))
                        .border_b_1()
                        .border_color(fg.opacity(STROKE_OPACITY)),
                )
            })
            .child(
                // `flex w-full shrink-0 flex-col items-center gap-1.5 py-1.5`
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .gap_1p5()
                    .py_1p5()
                    .child(
                        compact_action("compact-rail-expand", IconName::PanelLeft, "Expand projects", false, false, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_rail_open = true;
                                this.sidebar_drawer_open = false;
                                cx.notify();
                            })),
                    )
                    .child(self.render_compact_project_button(cx))
                    .children(tabs)
                    .child(surface_action("compact-rail-search", IconName::Search, "Search (⌘K)", Surface::Search, false))
                    .child(
                        surface_action(
                            "compact-rail-inbox",
                            IconName::Inbox,
                            if unseen { "Inbox, new items" } else { "Inbox" },
                            Surface::Inbox,
                            unseen,
                        )
                        // MonoCode `onOpenContextMenu`: the notification menu.
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_inbox_menu(event.position, cx);
                            }),
                        ),
                    )
                    .child(surface_action("compact-rail-notes", IconName::StickyNote, "Notes", Surface::Notes, false))
                    .child(surface_action(
                        "compact-rail-automations",
                        IconName::Zap,
                        "Automations",
                        Surface::Automations,
                        false,
                    )),
            )
            // `min-h-2 flex-1`
            .child(div().flex_1().min_h_2())
            .child(
                // `flex w-full flex-col items-center gap-1 py-1.5`
                div().flex().flex_col().items_center().w_full().gap_1().py_1p5().child(
                    compact_action("compact-rail-settings", IconName::Settings, "Settings (⌘,)", false, false, cx)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
                ),
            )
    }

    /// The current project's logo or mascot (MonoCode's `compact` project
    /// picker); a press lists the rail's projects to switch to.
    fn render_compact_project_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let cwd = self.current_cwd.clone();
        let open = matches!(
            self.sidebar_menu.as_ref().map(|menu| &menu.kind),
            Some(SidebarMenuKind::CompactProjects)
        );
        let label = if cwd.trim().is_empty() {
            "Open project".to_string()
        } else {
            self.rail_project_label(&cwd)
        };
        let anchor: Rc<std::cell::Cell<Option<gpui::Bounds<Pixels>>>> = Rc::default();
        let button_anchor = anchor.clone();
        div()
            .id("compact-rail-project")
            .relative()
            .flex()
            .flex_none()
            .size(px(32.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .when(open, |el| el.bg(fg.opacity(0.1)))
            .hover(move |el| el.bg(fg.opacity(0.1)))
            .tooltip(Tooltip::text(label))
            .child(canvas(move |bounds, _, _| button_anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
            .child(self.project_glyph(&cwd, "compact-rail-project-icon", cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let at = anchor.get().map_or(event.position, |bounds| {
                        Point::new(bounds.right() + px(POPOVER_GAP), bounds.top())
                    });
                    this.toggle_compact_projects_menu(at, cx);
                }),
            )
    }

    /// A project's logo, else its mascot; a folder with no project.
    fn project_glyph(&self, path: &str, id: &str, cx: &Context<Self>) -> AnyElement {
        if path.trim().is_empty() {
            let fg = cx.theme().colors.fg;
            return Icon::new(IconName::FolderPlus).size(IconSize::Md).color(fg.opacity(0.5)).into_any_element();
        }
        let busy = self
            .runs
            .keys()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .any(|s| crate::app::is_path_in_project(&s.cwd, path));
        match self.settings.rail.tab_group_logos.get(&path_key(path)).filter(|_| !busy) {
            // `ProjectLogoIcon className="size-4 rounded-sm"`
            Some(file) => thumbnail(file, LOGO).size_4().rounded(px(4.0)).into_any_element(),
            None => project_mascot_icon(self.project_mascot(path), self.project_color(path), busy, id),
        }
    }

    fn toggle_compact_projects_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let was_open = matches!(
            self.sidebar_menu.as_ref().map(|menu| &menu.kind),
            Some(SidebarMenuKind::CompactProjects)
        );
        self.close_sidebar_menu(cx);
        self.close_rail_menu(cx);
        if was_open {
            // The popover's outside-press check sees this and stays closed.
            self.sessions_ui.filter_button_hit = true;
            return;
        }
        self.open_sidebar_menu(SidebarMenuKind::CompactProjects, at, cx);
    }

    /// The icon rail's project list: the rail's projects in its order, the
    /// current one marked, then "Open folder…". MonoCode's picker also
    /// searches and carries each project's menu; those stay on the full
    /// rail here.
    pub(crate) fn render_compact_projects_menu(&self, position: Point<Pixels>, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        // `flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5
        // text-[13px] text-content/80 hover:bg-content/8 hover:text-content`
        let row = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .w_full()
                .items_center()
                .gap(px(10.0))
                .rounded(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(fg.opacity(0.8))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.opacity(0.08)).text_color(fg))
        };
        let projects = self.rail_order().into_iter().enumerate().map(|(ix, path)| {
            let current = same_project_path(&path, &self.current_cwd);
            let target = path.clone();
            row(SharedString::from(format!("compact-project-{ix}")))
                .when(current, |el| el.bg(fg.opacity(0.08)).text_color(fg))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .size_4()
                        .items_center()
                        .justify_center()
                        .child(self.project_glyph(&path, &format!("compact-project-icon-{ix}"), cx)),
                )
                .child(div().flex_1().min_w_0().truncate().child(self.rail_project_label(&path)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    if this.surface.is_some() {
                        this.close_surface(cx);
                    }
                    this.switch_project(target.clone(), cx);
                }))
        });
        let open_folder = row("compact-project-open-folder".into())
            .child(Icon::new(IconName::FolderPlus).size(IconSize::Sm).color(fg.opacity(0.8)))
            .child("Open folder…")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_project_dialog(cx);
            }));
        let list = div()
            .id("compact-projects-list")
            .flex()
            .flex_col()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .children(projects);
        let menu = crate::ui::sidebar_popovers::popover_frame(cx)
            .id("compact-projects-menu")
            .occlude()
            .w(px(PROJECTS_WIDTH))
            .p_1()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    if !std::mem::take(&mut this.sessions_ui.filter_button_hit) {
                        this.close_sidebar_menu(cx);
                    }
                });
            }))
            .child(list)
            .child(div().my_1().h(px(1.0)).bg(fg.opacity(0.1)))
            .child(open_folder);
        deferred(anchored().position(position).snap_to_window().child(menu))
            .with_priority(3)
            .into_any_element()
    }
}

/// MonoCode `CompactRailAction`: `grid size-8 place-items-center
/// rounded-md`, `bg-selection text-content` when active, else
/// `text-content/50 hover:bg-content/10 hover:text-content`; a `size-4`
/// icon and, with `dot`, a `size-1.5` accent dot at its top right.
fn compact_action(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    active: bool,
    dot: bool,
    cx: &Context<BenCodeApp>,
) -> Stateful<Div> {
    let theme = cx.theme();
    let (fg, accent) = (theme.colors.fg, theme.colors.accent);
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    div()
        .id(id)
        .group(id)
        .relative()
        .flex()
        .flex_none()
        .size(px(32.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .tooltip(Tooltip::text(label))
        .map(|el| {
            if active {
                el.bg(selection)
            } else {
                el.hover(move |s| s.bg(fg.opacity(0.10)))
            }
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Md)
                .color(if active { fg } else { fg.opacity(0.5) })
                .when(!active, |icon| icon.group_hover_color(id, fg)),
        )
        .when(dot, |el| {
            el.child(div().absolute().top(px(6.0)).right(px(6.0)).size(px(6.0)).rounded_full().bg(accent))
        })
}
