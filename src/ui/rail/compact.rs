//! MonoCode `CompactProjectRail` and `CompactRailAction` (`Sidebar.tsx`):
//! the column of icons the navigator leaves behind when collapsed with
//! Appearance › "Collapsed navigator" on "Icon rail". It expands the
//! navigator, switches project and opens the views.

use std::rc::Rc;

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, Div, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, SharedString, Stateful, Styled, anchored, canvas, deferred, div, prelude::*,
};

use super::widgets::project_mascot_icon;
use super::{TITLEBAR_HEIGHT, path_key};
use crate::app::{BenCodeApp, Surface, same_project_path};
use crate::ui::appearance::CollapsedRailMode;
use crate::ui::scale::px;
use crate::ui::sidebar_menus::SidebarMenuKind;
use crate::ui::thumbnail::{LOGO, thumbnail};
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

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
        cfg!(target_os = "macos")
            && self.compact_rail_active()
            && self.surface.is_none()
            && self.bots.view.is_none()
    }

    /// MonoCode `action(active, open)`: a view's icon opens it, or leaves
    /// it when it is the one open.
    fn toggle_surface(&mut self, surface: Surface, cx: &mut Context<Self>) {
        if self.surface_open(surface) {
            self.close_surface(cx);
            return;
        }
        match surface {
            Surface::Inbox => self.open_inbox_modal(cx),
            Surface::Notes => self.open_notes(cx),
            Surface::Automations => self.open_automations(cx),
            Surface::Settings => self.open_settings(cx),
        }
    }

    /// MonoCode `CompactProjectRail`: `sidebar-glass flex h-full w-12
    /// shrink-0 flex-col items-center`, a hairline down its right edge;
    /// `usage` is the focused thread's usage icon, above Settings.
    pub fn render_compact_rail(
        &self,
        usage: Option<AnyElement>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let glass = self.glass(cx).sidebar(colors.bg, theme.is_dark());
        let title_bar_above = self.compact_title_bar();
        let surface_action =
            |id: &'static str, icon: IconName, label: &'static str, surface: Surface, dot: bool| {
                compact_action(id, icon, label, self.surface_open(surface), dot, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_surface(surface, cx)))
            };
        let needs_you = self.inbox_needs_you(cx);
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
            .border_color(fg.fill(Fill::Line))
            // Without the title bar above, its height is held (and drags
            // the window): `h-10 w-full shrink-0 border-b border-stroke`.
            .when(!title_bar_above, |el| {
                el.child(
                    self.window_drag_region(div(), cx)
                        .flex_none()
                        .w_full()
                        .h(px(TITLEBAR_HEIGHT))
                        .border_b_1()
                        .border_color(fg.fill(Fill::Line)),
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
                        compact_action(
                            "compact-rail-expand",
                            IconName::PanelLeft,
                            "Show Navigator (⌘B)",
                            false,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.is_rail_open = true;
                            cx.notify();
                        })),
                    )
                    .children(self.render_compact_bots(cx))
                    .child(self.render_compact_project_button(cx))
                    .children(self.render_agents_badge(true, cx))
                    .child(
                        compact_action(
                            "compact-rail-search",
                            IconName::Search,
                            "Search (⌘K)",
                            self.search.open,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_search(cx))),
                    )
                    .child(
                        surface_action(
                            "compact-rail-inbox",
                            IconName::Inbox,
                            if needs_you {
                                "Inbox, items need you"
                            } else {
                                "Inbox"
                            },
                            Surface::Inbox,
                            needs_you,
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
                    .child(surface_action(
                        "compact-rail-notes",
                        IconName::StickyNote,
                        "Notes",
                        Surface::Notes,
                        false,
                    ))
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
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .gap_1()
                    .py_1p5()
                    .children(usage)
                    .child(
                        compact_action(
                            "compact-rail-settings",
                            IconName::Settings,
                            "Settings (⌘,)",
                            false,
                            false,
                            cx,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx)),
                        ),
                    ),
            )
    }

    /// MonoCode `railMonosPinned`: each Bot at the top of the icon rail,
    /// above a divider (`CompactMonoButton`), its figure moving with its
    /// mood. Unpinned, the icon rail shows none (MonoCode lists them in
    /// its project picker, which BenCode's compact picker does not).
    fn render_compact_bots(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.bots_enabled() || !self.settings.rail_bots_pinned || self.settings.bots.is_empty()
        {
            return None;
        }
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
        let buttons = self.settings.bots.iter().map(|bot| {
            let selected = self.surface.is_none() && self.bots.view.as_deref() == Some(&bot.id);
            let id = bot.id.clone();
            div()
                .id(SharedString::from(format!("compact-bot-{}", bot.id)))
                .flex()
                .flex_none()
                .size(px(32.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .map(|el| {
                    if selected {
                        el.bg(selection)
                    } else {
                        el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                    }
                })
                .tooltip(Tooltip::text(bot.display_name()))
                .on_click(cx.listener(move |this, _, _, cx| this.open_bot(&id, cx)))
                .child(crate::ui::figure::figure_view(
                    bot.figure(),
                    16.0,
                    self.bot_color(bot),
                    self.bot_look(bot),
                    cx,
                ))
        });
        Some(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1p5()
                .children(buttons)
                .child(div().w(px(20.0)).h(px(1.0)).bg(fg.fill(Fill::Strong)))
                .into_any_element(),
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
            .when(open, |el| el.bg(fg.fill(Fill::Strong)))
            .hover(move |el| el.bg(fg.fill(Fill::Strong)))
            .tooltip(Tooltip::text(label))
            .child(
                canvas(
                    move |bounds, _, _| button_anchor.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(self.project_glyph(&cwd, cx))
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
    pub(crate) fn project_glyph(&self, path: &str, cx: &Context<Self>) -> AnyElement {
        if path.trim().is_empty() {
            let fg = cx.theme().colors.fg;
            return Icon::new(IconName::FolderPlus)
                .size(IconSize::Md)
                .color(fg.ink(Ink::Secondary))
                .into_any_element();
        }
        let busy = self
            .runs
            .keys()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .any(|s| crate::app::is_path_in_project(&s.cwd, path));
        match self
            .settings
            .rail
            .tab_group_logos
            .get(&path_key(path))
            .filter(|_| !busy)
        {
            // `ProjectLogoIcon className="size-4 rounded-sm"`
            Some(file) => thumbnail(file, LOGO)
                .size_4()
                .rounded(px(4.0))
                .into_any_element(),
            None => project_mascot_icon(
                self.project_mascot(path),
                self.project_color(path),
                busy,
                cx,
            ),
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
    /// current one marked, then "Open folder…" and "Open folder on a
    /// machine…". MonoCode's picker also
    /// searches and carries each project's menu; those stay on the full
    /// rail here.
    pub(crate) fn render_compact_projects_menu(
        &self,
        position: Point<Pixels>,
        cx: &Context<Self>,
    ) -> AnyElement {
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
                .text_size(Text::Md.px())
                .text_color(fg.ink(Ink::Strong))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
        };
        let projects = self.rail_order().into_iter().enumerate().map(|(ix, path)| {
            let current = same_project_path(&path, &self.current_cwd);
            let target = path.clone();
            row(SharedString::from(format!("compact-project-{ix}")))
                .when(current, |el| el.bg(fg.fill(Fill::Selected)).text_color(fg))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .size_4()
                        .items_center()
                        .justify_center()
                        .child(self.project_glyph(&path, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(self.rail_project_label(&path)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    if this.surface.is_some() {
                        this.close_surface(cx);
                    }
                    this.switch_project(target.clone(), cx);
                }))
        });
        let open_folder = row("compact-project-open-folder".into())
            .child(
                Icon::new(IconName::FolderPlus)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Strong)),
            )
            .child("Open folder…")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_project_dialog(cx);
            }));
        let open_remote = row("compact-project-open-remote-folder".into())
            .child(
                Icon::new(IconName::Globe)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Strong)),
            )
            .child("Open folder on a machine…")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_remote_folder_dialog(cx);
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
            .child(div().my_1().h(px(1.0)).bg(fg.fill(Fill::Strong)))
            .child(open_folder)
            .child(open_remote);
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
                el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
            }
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Md)
                .color(if active { fg } else { fg.ink(Ink::Secondary) })
                .when(!active, |icon| icon.group_hover_color(id, fg)),
        )
        .when(dot, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(accent),
            )
        })
}
