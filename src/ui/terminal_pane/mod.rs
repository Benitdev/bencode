//! Terminal dock (MonoCode `ProjectTerminalDock`): each project keeps its
//! own terminals, opened in the project folder, under a tab strip with
//! New Terminal (⌘`). Toggled from the footer or with ⌘J. It docks on any
//! edge of the workspace (Move Terminal), the sash on its inner edge
//! resizes it, and each project's side and size are saved.

mod layout;
mod menu;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{DragGhost, Icon, IconName, Tooltip};
use ely_gpui_component::terminal::{Launch, Terminal, TerminalEvent};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, Subscription, Window, div, prelude::*, px,
};

pub use layout::{DockLayout, DockSide};

use crate::app::BenCodeApp;
use crate::ui::icons::ExtraIcon;

const TAB_HEIGHT: gpui::Pixels = px(28.0);

pub struct TerminalTab {
    pub id: u64,
    pub entity: Entity<Terminal>,
    /// The folder its shell started in.
    pub cwd: String,
    _events: Subscription,
}

/// One project's terminals.
#[derive(Default)]
pub struct TerminalDock {
    pub tabs: Vec<TerminalTab>,
    pub active: u64,
}

/// Terminal docks keyed by project folder.
#[derive(Default)]
pub struct TerminalDocks {
    docks: HashMap<String, TerminalDock>,
    next_id: u64,
    /// Each project's side and size, as saved; a project missing here has
    /// the default bottom dock.
    pub layouts: BTreeMap<String, DockLayout>,
    /// The pointer (along the dock's axis) and the dock's size when the
    /// sash was pressed.
    resize_from: Option<(f32, f32)>,
    /// A sash drag is under way, so the sash stays lit.
    resizing: bool,
    menu: Option<menu::TerminalMenu>,
    /// The window's size at the last render, for picks made by key.
    viewport: (f32, f32),
}

/// Drag payload of the dock's resize sash.
#[derive(Clone, Copy, Debug)]
pub struct TerminalResize;

impl gpui::Render for TerminalResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// Drag payload of a terminal tab: its index in the strip.
#[derive(Clone, Copy, Debug)]
struct TerminalTabDrag {
    from: usize,
}

impl TerminalDocks {
    pub fn dock(&self, project: &str) -> Option<&TerminalDock> {
        self.docks.get(project)
    }

    pub fn layout(&self, project: &str) -> DockLayout {
        self.layouts.get(project).copied().unwrap_or_default()
    }

    /// Stores a project's layout, dropping it again once it is the default.
    fn set_layout(&mut self, project: &str, layout: DockLayout) {
        if layout == DockLayout::default() {
            self.layouts.remove(project);
        } else {
            self.layouts.insert(project.to_string(), layout);
        }
    }

    /// Whether a terminal was started inside `path` (MonoCode
    /// `PtyHost::has_working_dir`).
    pub fn any_in(&self, path: &str) -> bool {
        self.docks
            .values()
            .flat_map(|dock| &dock.tabs)
            .any(|tab| crate::app::is_path_in_project(&tab.cwd, path))
    }
}

fn spawn_shell(cwd: &str, cx: &mut Context<Terminal>) -> Terminal {
    let launch = Launch {
        program: None,
        cwd: Some(PathBuf::from(cwd)).filter(|p| p.is_dir()),
        env: vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ],
    };
    Terminal::spawn(launch, cx).unwrap_or_else(|err| {
        log::error!("failed to start terminal in {cwd}: {err:#}");
        Terminal::replay(b"Terminal unavailable\r\n", 80, 24, cx)
    })
}

fn window_size(window: &Window) -> (f32, f32) {
    let size = window.viewport_size();
    (f32::from(size.width), f32::from(size.height))
}

/// MonoCode `sideIcon`: the Move Terminal button shows where the dock is.
fn side_icon(side: DockSide) -> Icon {
    match side {
        DockSide::Top => ExtraIcon::PanelTop.icon(),
        DockSide::Bottom => Icon::new(IconName::PanelBottom),
        DockSide::Left => Icon::new(IconName::PanelLeft),
        DockSide::Right => Icon::new(IconName::PanelRight),
    }
}

/// MonoCode `hideIcon`: Hide Terminal points at the edge it folds into.
fn hide_icon(side: DockSide) -> IconName {
    match side {
        DockSide::Top => IconName::ChevronUp,
        DockSide::Bottom => IconName::ChevronDown,
        DockSide::Left => IconName::ChevronLeft,
        DockSide::Right => IconName::ChevronRight,
    }
}

impl BenCodeApp {
    /// Makes sure the current project has at least one terminal.
    pub fn ensure_project_terminal(&mut self, cx: &mut Context<Self>) {
        let has_tab = self
            .terminals
            .dock(&self.current_cwd)
            .is_some_and(|dock| !dock.tabs.is_empty());
        if !has_tab {
            self.new_terminal(cx);
        }
    }

    /// Opens another terminal for the current project and shows the dock.
    pub fn new_terminal(&mut self, cx: &mut Context<Self>) {
        let cwd = self.current_cwd.clone();
        self.new_terminal_at(&cwd, cx);
    }

    /// A terminal for the current project started in `cwd` (MonoCode
    /// Explorer "Open in Terminal").
    pub fn new_terminal_at(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        let cwd = cwd.to_string();
        let entity = cx.new(|cx| spawn_shell(&cwd, cx));
        self.terminals.next_id += 1;
        let id = self.terminals.next_id;
        let key = project.clone();
        let events = cx.subscribe(
            &entity,
            move |this, _, event: &TerminalEvent, cx| match event {
                TerminalEvent::Exited(_) => this.close_terminal(&key, id, cx),
                TerminalEvent::Title(_) => cx.notify(),
                _ => {}
            },
        );
        let dock = self.terminals.docks.entry(project).or_default();
        dock.tabs.push(TerminalTab {
            id,
            entity,
            cwd,
            _events: events,
        });
        dock.active = id;
        if !self.is_terminal_open {
            self.set_terminal_open(true, cx);
        }
        cx.notify();
    }

    /// Closes terminal `id` of `project`; its neighbour becomes active.
    pub fn close_terminal(&mut self, project: &str, id: u64, cx: &mut Context<Self>) {
        let Some(dock) = self.terminals.docks.get_mut(project) else {
            return;
        };
        let Some(ix) = dock.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        dock.tabs.remove(ix);
        if dock.active == id {
            let next = dock.tabs.get(ix).or_else(|| dock.tabs.last());
            dock.active = next.map_or(0, |t| t.id);
        }
        cx.notify();
    }

    /// MonoCode `onCloseOtherProjectTerminals`: keeps `id` and selects it.
    fn close_other_terminals(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            dock.tabs.retain(|tab| tab.id == id);
            dock.active = id;
            cx.notify();
        }
    }

    /// MonoCode `reorderDockTerminals`: a tab dropped on another.
    fn reorder_terminals(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            layout::reorder(&mut dock.tabs, from, to);
            cx.notify();
        }
    }

    fn select_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            dock.active = id;
            cx.notify();
        }
    }

    /// MonoCode `onProjectTerminalSide`: moves the current project's dock.
    fn set_dock_side(&mut self, side: DockSide, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        let layout = self.terminals.layout(&project).with_side(side, self.terminals.viewport);
        self.terminals.set_layout(&project, layout);
        self.save_settings(cx);
        cx.notify();
    }

    /// The workspace (chat and file pane) with the current project's dock
    /// on its side (MonoCode `dockGridStyle`), or alone while it is hidden.
    pub fn render_workspace_with_dock(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let main = div()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_workspace_split(cx));
        if !self.is_terminal_open {
            return main.into_any_element();
        }
        self.terminals.viewport = window_size(window);
        let layout = self.terminals.layout(&self.current_cwd);
        let dock = self.render_terminal_dock(layout, self.terminals.viewport, cx);
        let frame = div().flex().flex_1().min_w_0().min_h_0().overflow_hidden();
        match layout.side {
            DockSide::Bottom => frame.flex_col().child(main).child(dock),
            DockSide::Top => frame.flex_col().child(dock).child(main),
            DockSide::Left => frame.flex_row().child(dock).child(main),
            DockSide::Right => frame.flex_row().child(main).child(dock),
        }
        .into_any_element()
    }

    fn render_terminal_dock(
        &mut self,
        layout: DockLayout,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = &cx.theme().colors;
        let active = self
            .terminals
            .dock(&self.current_cwd)
            .and_then(|dock| dock.tabs.iter().find(|t| t.id == dock.active))
            .map(|tab| tab.entity.clone());
        let body = match active {
            Some(terminal) => div().flex_1().min_h_0().min_w_0().p_1().child(terminal),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(colors.fg_muted)
                .child("No terminal. Press ⌘` to open one."),
        };
        // A window made smaller since keeps the dock within its bounds.
        let size = px(layout::clamp_size(layout.side, layout.size, viewport));
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .min_h_0()
            .map(|el| match layout.side {
                DockSide::Bottom => el.w_full().h(size).border_t_1(),
                DockSide::Top => el.w_full().h(size).border_b_1(),
                DockSide::Left => el.h_full().w(size).border_r_1(),
                DockSide::Right => el.h_full().w(size).border_l_1(),
            })
            .border_color(colors.border)
            .bg(colors.bg)
            .on_drag_move::<TerminalResize>(cx.listener(
                |this, event: &gpui::DragMoveEvent<TerminalResize>, window, cx| {
                    let Some((start, start_size)) = this.terminals.resize_from else {
                        return;
                    };
                    let project = this.current_cwd.clone();
                    let layout = this.terminals.layout(&project);
                    let point = if layout.side.is_vertical() {
                        event.event.position.y
                    } else {
                        event.event.position.x
                    };
                    let next = layout.dragged(start_size, start, f32::from(point), window_size(window));
                    if next != layout || !this.terminals.resizing {
                        this.terminals.set_layout(&project, next);
                        this.terminals.resizing = true;
                        cx.notify();
                    }
                },
            ))
            .child(self.render_terminal_tabs(layout.side, cx))
            .child(body)
            .child(self.render_terminal_sash(layout.side, cx))
    }

    /// MonoCode's "Resize terminal" separator on the dock's inner edge; a
    /// double click restores the side's default size. The size is saved
    /// when the drag ends.
    fn render_terminal_sash(&self, side: DockSide, cx: &Context<Self>) -> impl IntoElement + use<> {
        let fg = cx.theme().colors.fg;
        let dragging = self.terminals.resizing;
        let end_drag = |this: &mut Self, _: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Self>| {
            this.terminals.resize_from = None;
            if std::mem::take(&mut this.terminals.resizing) {
                this.save_settings(cx);
                cx.notify();
            }
        };
        div()
            .id("terminal-resize")
            .absolute()
            .map(|el| match side {
                DockSide::Bottom => el.left_0().right_0().top(px(-1.0)).h(px(6.0)).cursor_row_resize(),
                DockSide::Top => el.left_0().right_0().bottom(px(-1.0)).h(px(6.0)).cursor_row_resize(),
                DockSide::Left => el.top_0().bottom_0().right(px(-1.0)).w(px(6.0)).cursor_col_resize(),
                DockSide::Right => el.top_0().bottom_0().left(px(-1.0)).w(px(6.0)).cursor_col_resize(),
            })
            .when(dragging, |el| el.bg(fg.opacity(0.15)))
            .when(!dragging, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    let project = this.current_cwd.clone();
                    let layout = this.terminals.layout(&project);
                    if event.click_count >= 2 {
                        let reset = DockLayout { size: layout.side.default_size(), ..layout };
                        this.terminals.set_layout(&project, reset);
                        this.terminals.resize_from = None;
                        this.save_settings(cx);
                        cx.notify();
                        return;
                    }
                    let point = if layout.side.is_vertical() {
                        event.position.y
                    } else {
                        event.position.x
                    };
                    this.terminals.resize_from = Some((f32::from(point), layout.size));
                }),
            )
            .on_drag(TerminalResize, |drag, _, _, cx| cx.new(|_| *drag))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(end_drag))
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(end_drag))
    }

    fn render_terminal_tabs(&self, side: DockSide, cx: &Context<Self>) -> impl IntoElement + use<> {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let tabs = self
            .terminals
            .dock(&self.current_cwd)
            .map(|dock| {
                dock.tabs
                    .iter()
                    .enumerate()
                    .map(|(ix, tab)| self.render_terminal_tab(ix, tab, tab.id == dock.active, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let menu_open = self.terminals.menu.as_ref().is_some_and(|menu| menu.is_side());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .px_1p5()
            .h(px(32.0))
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .id("terminal-tab-strip")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_0p5()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                IconButton::new("terminal-new", IconName::Plus)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("New Terminal (⌘`)")
                    .on_click(cx.listener(|this, _, _, cx| this.new_terminal(cx))),
            )
            // An Ely `IconButton` takes Ely's icons only; Lucide's
            // `panel-top` is one of BenCode's own.
            .child(
                div()
                    .id("terminal-move")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .when(menu_open, |el| el.bg(colors.hover))
                    .hover(|s| s.bg(colors.hover))
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        this.toggle_dock_side_menu(event.position(), cx)
                    }))
                    .tooltip(Tooltip::text("Move Terminal"))
                    .child(side_icon(side).size(IconSize::Sm).color(fg.opacity(0.7))),
            )
            .child(
                IconButton::new("terminal-drawer-hide", hide_icon(side))
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Hide Terminal (⌘J)")
                    .on_click(cx.listener(|this, _, _, cx| this.set_terminal_open(false, cx))),
            )
    }

    fn render_terminal_tab(
        &self,
        ix: usize,
        tab: &TerminalTab,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = &cx.theme().colors;
        let title = tab.entity.read(cx).title().clone();
        let title = if title.is_empty() {
            SharedString::from("Terminal")
        } else {
            title
        };
        let (id, project) = (tab.id, self.current_cwd.clone());
        let close = IconButton::new(
            SharedString::from(format!("terminal-close-{id}")),
            IconName::X,
        )
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip("Close Terminal")
        .on_click(cx.listener(move |this, _, _, cx| this.close_terminal(&project, id, cx)));
        let ghost = title.clone();
        let edge = colors.accent;
        div()
            .id(SharedString::from(format!("terminal-tab-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .h(TAB_HEIGHT)
            .pl_2()
            .pr_1()
            .max_w(px(200.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(if active { colors.fg } else { colors.fg_muted })
            .when(active, |el| el.bg(colors.active))
            .hover(|s| s.bg(colors.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select_terminal(id, cx)))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_terminal_tab_menu(id, event.position, cx);
                }),
            )
            .on_drag(TerminalTabDrag { from: ix }, move |_, _, _, cx| {
                DragGhost::new(ghost.clone(), Some(IconName::Terminal), cx)
            })
            .drag_over::<TerminalTabDrag>(move |style, drag, _, _| {
                if drag.from == ix { style } else { style.border_color(edge) }
            })
            .on_drop(cx.listener(move |this, drag: &TerminalTabDrag, _, cx| {
                this.reorder_terminals(drag.from, ix, cx)
            }))
            .child(
                Icon::new(IconName::Terminal)
                    .size(IconSize::Xs)
                    .color(colors.fg_muted),
            )
            .child(div().min_w_0().truncate().child(title))
            .child(close)
    }
}
