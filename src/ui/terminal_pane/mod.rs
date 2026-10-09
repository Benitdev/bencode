//! Terminal dock (MonoCode `ProjectTerminalDock`): each project keeps its
//! own terminals, opened in the project folder, under a tab strip with
//! New Terminal (⌘`). Toggled from the footer or with ⌘J. It docks on any
//! edge of the workspace (Move Terminal), the sash on its inner edge
//! resizes it, and each project's side, size and shown state are saved.
//! A tab is named for the job it runs or the folder its shell is in, and
//! one whose shell ended stays open with MonoCode's `[process exited]`.
//! A tab dragged onto an edge of a terminal shows the two side by side
//! (`split`), as panes dock in the chat.

mod ime;
mod layout;
mod menu;
mod running;
mod split;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::layout::SplitPane;
use ely_gpui_component::primitives::{DragGhost, Icon, IconName, Tooltip};
use ely_gpui_component::terminal::{Launch, Terminal, TerminalEvent};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Axis, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Subscription, Window, div, prelude::*,
};

use crate::ui::scale::px;

pub use layout::{DockLayout, DockSide};
pub use running::{POLL_EVERY, RunningTerminal, chip_label};

use crate::app::BenCodeApp;
use crate::ui::drag_drop::render_pane_drop_hint;
use crate::ui::icons::ExtraIcon;
use crate::ui::layout::{
    LayoutNode, PaneEdge, SplitDir, pane_edge_from_point, set_split_sizes, split_shares,
};

const TAB_HEIGHT: f32 = 28.0;
/// The least a split's sash leaves a terminal.
const PANE_MIN: f32 = 120.0;

pub struct TerminalTab {
    pub id: u64,
    pub entity: Entity<Terminal>,
    /// The terminal as the dock shows it, with text composition.
    view: Entity<ime::TerminalIme>,
    /// The folder its shell started in.
    pub cwd: String,
    /// The process Ely started for it (`login` on macOS) until it ends.
    process: Option<u32>,
    /// The shell under `process`, once found.
    shell: Option<u32>,
    /// The job in its foreground other than the shell.
    foreground: Option<crate::terminal_process::Foreground>,
    /// The folder its shell is in now.
    dir: Option<String>,
    _events: Subscription,
}

impl TerminalTab {
    /// MonoCode `terminalTabLabel`: the running job, else the folder.
    fn title(&self) -> String {
        match &self.foreground {
            Some(job) => job.process.clone(),
            None => self.folder(),
        }
    }

    /// MonoCode `defaultTerminalTitle` of the shell's folder.
    fn folder(&self) -> String {
        running::default_title(self.dir.as_deref().unwrap_or(&self.cwd))
    }
}

/// One project's terminals.
#[derive(Default)]
pub struct TerminalDock {
    pub tabs: Vec<TerminalTab>,
    /// The terminal showing; in a split, the pane last used.
    pub active: u64,
    /// Terminals that show side by side, a tree for each such set. The
    /// dock shows the one `active` is in, else `active` alone.
    splits: Vec<LayoutNode>,
}

impl TerminalDock {
    /// Takes terminal `id` out of its split; its tab stays. The active
    /// terminal moves to a neighbour when it was `id`.
    fn leave_split(&mut self, id: u64) {
        if let Some(active) = split::without(&mut self.splits, self.active, id) {
            self.active = active;
        }
    }
}

/// Terminal docks keyed by project folder.
#[derive(Default)]
pub struct TerminalDocks {
    docks: HashMap<String, TerminalDock>,
    next_id: u64,
    /// Each project's side, size and shown state, as saved; a project
    /// missing here has the default, hidden bottom dock.
    pub layouts: BTreeMap<String, DockLayout>,
    /// The pointer (along the dock's axis) and the dock's size when the
    /// sash was pressed.
    resize_from: Option<(f32, f32)>,
    /// A sash drag is under way, so the sash stays lit.
    resizing: bool,
    menu: Option<menu::TerminalMenu>,
    /// Terminals waiting on "Close anyway?" because a job runs in them.
    close_confirm: Option<running::CloseConfirm>,
    /// The process poll is running (while any terminal's shell is).
    polling: bool,
    /// The footer chip's list was open when the chip was pressed.
    pub(crate) running_menu_at_press: bool,
    /// The window's size at the last render, for picks made by key.
    viewport: (f32, f32),
    /// The terminal a dragged tab is over, and the edge it would dock on.
    pane_drop: Option<(u64, PaneEdge)>,
}

/// Drag payload of the dock's resize sash.
#[derive(Clone, Copy, Debug)]
pub struct TerminalResize;

impl gpui::Render for TerminalResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// Drag payload of a terminal tab (or its pane's header): the terminal and
/// its index in the strip.
#[derive(Clone, Copy, Debug)]
struct TerminalTabDrag {
    id: u64,
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

    /// Shows or hides `project`'s dock (MonoCode `withDockOpen`).
    fn set_open(&mut self, project: &str, open: bool) {
        let layout = DockLayout {
            open,
            ..self.layout(project)
        };
        self.set_layout(project, layout);
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
    (
        crate::ui::scale::logical(size.width),
        crate::ui::scale::logical(size.height),
    )
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
    /// Whether the current project's dock is shown.
    pub fn is_terminal_open(&self) -> bool {
        self.terminals.layout(&self.current_cwd).open
    }

    /// Shows or hides the current project's dock; showing it starts a
    /// shell when the project has none.
    pub fn set_terminal_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        self.terminals.set_open(&project, open);
        if open {
            self.ensure_project_terminal(cx);
        }
        self.save_settings(cx);
        cx.notify();
    }

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

    /// A terminal has the keyboard: its keys (Escape above all) are its own.
    pub(crate) fn terminal_focused(&self, window: &gpui::Window, cx: &gpui::App) -> bool {
        self.terminals
            .docks
            .values()
            .flat_map(|dock| &dock.tabs)
            .any(|tab| gpui::Focusable::focus_handle(tab.entity.read(cx), cx).is_focused(window))
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
        let before = crate::terminal_process::children();
        let entity = cx.new(|cx| spawn_shell(&cwd, cx));
        let process = crate::terminal_process::spawned(&before);
        if process.is_none() {
            log::debug!(
                "terminal in {cwd}: its process was not found; no job or folder in its tab"
            );
        }
        let view = cx.new(|_| ime::TerminalIme::new(entity.clone()));
        self.terminals.next_id += 1;
        let id = self.terminals.next_id;
        let key = project.clone();
        let events = cx.subscribe(
            &entity,
            move |this, _, event: &TerminalEvent, cx| match event {
                TerminalEvent::Exited(_) => this.terminal_exited(&key, id, cx),
                _ => {}
            },
        );
        let dock = self.terminals.docks.entry(project).or_default();
        dock.tabs.push(TerminalTab {
            id,
            entity,
            view,
            cwd,
            process,
            shell: None,
            foreground: None,
            dir: None,
            _events: events,
        });
        dock.active = id;
        if !self.is_terminal_open() {
            self.set_terminal_open(true, cx);
        }
        self.start_terminal_poll(cx);
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
        if split::group_of(&dock.splits, id).is_some() {
            dock.leave_split(id);
        } else if dock.active == id {
            let next = dock.tabs.get(ix).or_else(|| dock.tabs.last());
            dock.active = next.map_or(0, |t| t.id);
        }
        // MonoCode `closeTerminalInDock` drops an emptied dock, which hides it.
        if dock.tabs.is_empty() && self.terminals.layout(project).open {
            self.terminals.set_open(project, false);
            self.save_settings(cx);
        }
        cx.notify();
    }

    /// The end of MonoCode `onCloseOtherProjectTerminals`: closes `closing`
    /// and selects `keep`.
    fn close_terminals_but(
        &mut self,
        project: &str,
        closing: &[u64],
        keep: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(dock) = self.terminals.docks.get_mut(project) {
            dock.tabs.retain(|tab| !closing.contains(&tab.id));
            for &id in closing {
                dock.leave_split(id);
            }
            if dock.tabs.iter().any(|tab| tab.id == keep) {
                dock.active = keep;
            }
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

    /// A press inside the pane of terminal `id` makes it the active one.
    fn focus_terminal_pane(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd)
            && dock.active != id
        {
            dock.active = id;
            cx.notify();
        }
    }

    /// A tab dropped on the pane of terminal `target`: it docks on the
    /// hinted edge and becomes the active terminal.
    fn drop_terminal_on_pane(&mut self, dragged: u64, target: u64, cx: &mut Context<Self>) {
        let edge = self
            .terminals
            .pane_drop
            .take()
            .filter(|(over, _)| *over == target)
            .map(|(_, edge)| edge);
        if let (Some(edge), Some(dock)) = (edge, self.terminals.docks.get_mut(&self.current_cwd))
            && dragged != target
            && dock.tabs.iter().any(|tab| tab.id == dragged)
        {
            split::dock(&mut dock.splits, target, dragged, edge);
            dock.active = dragged;
        }
        cx.notify();
    }

    /// A split pane's Close: the terminal leaves the split and keeps its tab.
    fn close_terminal_pane(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            dock.leave_split(id);
            cx.notify();
        }
    }

    fn resize_terminal_split(&mut self, split_id: &str, shares: &[f32], cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            // Split ids are unique: only the tree that has it changes.
            for tree in &mut dock.splits {
                *tree = set_split_sizes(tree, split_id, shares);
            }
            cx.notify();
        }
    }

    /// MonoCode `onProjectTerminalSide`: moves the current project's dock.
    fn set_dock_side(&mut self, side: DockSide, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        let layout = self
            .terminals
            .layout(&project)
            .with_side(side, self.terminals.viewport);
        self.terminals.set_layout(&project, layout);
        self.save_settings(cx);
        cx.notify();
    }

    /// The workspace (chat and file pane) with the current project's dock
    /// on its side (MonoCode `dockGridStyle`), or alone while it is hidden.
    pub fn render_workspace_with_dock(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let main = div()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_workspace_split(cx));
        let layout = self.terminals.layout(&self.current_cwd);
        if !layout.open {
            return main.into_any_element();
        }
        self.terminals.viewport = window_size(window);
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
        // A drag let go anywhere but on a pane leaves its hint behind.
        if !cx.has_active_drag() {
            self.terminals.pane_drop = None;
        }
        let (split, active) = self
            .terminals
            .dock(&self.current_cwd)
            .map_or((None, None), |dock| {
                let active = dock
                    .tabs
                    .iter()
                    .any(|t| t.id == dock.active)
                    .then_some(dock.active);
                (split::group_of(&dock.splits, dock.active).cloned(), active)
            });
        let body = match (split, active) {
            (Some(tree), _) => self.render_terminal_split(&tree, cx),
            (None, Some(id)) => self.render_terminal_pane(id, false, cx),
            (None, None) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(cx.theme().colors.fg_muted)
                .child("No terminal. Press ⌘` to open one.")
                .into_any_element(),
        };
        let colors = &cx.theme().colors;
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
                    let next = layout.dragged(
                        start_size,
                        start,
                        crate::ui::scale::logical(point),
                        window_size(window),
                    );
                    if next != layout || !this.terminals.resizing {
                        this.terminals.set_layout(&project, next);
                        this.terminals.resizing = true;
                        cx.notify();
                    }
                },
            ))
            .child(self.render_terminal_tabs(layout.side, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(body),
            )
            .child(self.render_terminal_sash(layout.side, cx))
    }

    /// The split's tree as Ely `SplitPane`s, like the chat's panes.
    fn render_terminal_split(&self, node: &LayoutNode, cx: &Context<Self>) -> AnyElement {
        let (split_id, dir, children, sizes) = match node {
            LayoutNode::Leaf { id } => {
                return match id.parse() {
                    Ok(id) => self.render_terminal_pane(id, true, cx),
                    Err(_) => div().into_any_element(),
                };
            }
            LayoutNode::Split {
                id,
                dir,
                children,
                sizes,
            } => (id, *dir, children, sizes),
        };
        let axis = match dir {
            SplitDir::Right => Axis::Horizontal,
            SplitDir::Down => Axis::Vertical,
        };
        let weak = cx.entity().downgrade();
        let owned_id = split_id.clone();
        let mut split = SplitPane::new(
            SharedString::from(format!("terminal-{split_id}")),
            axis,
            px(PANE_MIN),
        )
        .sizes(&split_shares(children.len(), sizes))
        .on_resize(move |shares, _, cx| {
            if let Err(err) = weak.update(cx, |this, cx| {
                this.resize_terminal_split(&owned_id, shares, cx)
            }) {
                log::debug!("terminal split resize after app drop: {err:#}");
            }
        });
        for child in children {
            split = split.pane(
                div()
                    .size_full()
                    .child(self.render_terminal_split(child, cx)),
            );
        }
        split.into_any_element()
    }

    /// One terminal: its grid, the exit line, and in a split its header.
    /// A dragged tab docks on the edge of it the pointer is nearest.
    fn render_terminal_pane(&self, id: u64, in_split: bool, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let Some((ix, tab, active)) = self.terminals.dock(&self.current_cwd).and_then(|dock| {
            let ix = dock.tabs.iter().position(|tab| tab.id == id)?;
            Some((ix, &dock.tabs[ix], dock.active == id))
        }) else {
            return div().into_any_element();
        };
        let exited = tab.entity.read(cx).exit_status();
        let drop_hint = self
            .terminals
            .pane_drop
            .filter(|(over, _)| *over == id)
            .map(|(_, edge)| edge);
        div()
            .id(("terminal-pane", id))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .min_w_0()
            .min_h_0()
            // In the capture phase: the terminal keeps its own presses.
            .capture_any_mouse_down(
                cx.listener(move |this, _, _, cx| this.focus_terminal_pane(id, cx)),
            )
            // Fires for every pane, so each checks the pointer is inside.
            .on_drag_move::<TerminalTabDrag>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<TerminalTabDrag>, _, cx| {
                    let (bounds, at) = (event.bounds, event.event.position);
                    let over = bounds.contains(&at) && event.drag(cx).id != id;
                    let hint = over.then(|| {
                        let edge = pane_edge_from_point(
                            at.x.into(),
                            at.y.into(),
                            bounds.origin.x.into(),
                            bounds.origin.y.into(),
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                        );
                        (id, edge)
                    });
                    let mine = this.terminals.pane_drop.is_some_and(|(over, _)| over == id);
                    if hint.is_some() && this.terminals.pane_drop != hint {
                        this.terminals.pane_drop = hint;
                        cx.notify();
                    } else if hint.is_none() && mine {
                        this.terminals.pane_drop = None;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(move |this, drag: &TerminalTabDrag, _, cx| {
                this.drop_terminal_on_pane(drag.id, id, cx)
            }))
            .when(in_split, |el| {
                el.child(self.render_terminal_pane_header(ix, tab, active, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .p_1()
                    .child(div().flex_1().min_h_0().min_w_0().child(tab.view.clone())),
            )
            // MonoCode writes this line into the terminal; Ely's grid
            // takes no text once its program is gone, so it sits under it.
            .children(exited.map(|status| {
                div()
                    .flex_none()
                    .px_2()
                    .pb_1()
                    .font_family(cx.theme().mono_family.clone())
                    .text_size(px(12.0))
                    .text_color(colors.fg_muted)
                    .child(running::exited_line(status.code()))
            }))
            .when_some(drop_hint, |el, edge| {
                el.child(render_pane_drop_hint(edge, cx))
            })
            .into_any_element()
    }

    /// A split pane's header, as the chat's: a grip to drag the terminal
    /// to another edge, a dot lit on the active pane, its title, and Close.
    fn render_terminal_pane_header(
        &self,
        ix: usize,
        tab: &TerminalTab,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = &cx.theme().colors;
        let id = tab.id;
        let title = SharedString::from(tab.title());
        let ghost = title.clone();
        div()
            .id(("terminal-pane-header", id))
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .h(px(TAB_HEIGHT))
            .pl_2()
            .pr_1()
            .border_b_1()
            .border_color(colors.border)
            .cursor_grab()
            .on_drag(TerminalTabDrag { id, from: ix }, move |_, _, _, cx| {
                DragGhost::new(ghost.clone(), Some(IconName::Terminal), cx)
            })
            .child(
                Icon::new(IconName::GripVertical)
                    .size(IconSize::Xs)
                    .color(colors.fg.opacity(0.35)),
            )
            .child(
                div()
                    .size(px(8.0))
                    .flex_none()
                    .rounded_full()
                    .bg(if active {
                        colors.accent
                    } else {
                        gpui::transparent_black()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(if active { colors.fg } else { colors.fg_muted })
                    .child(title),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("terminal-pane-close-{id}")),
                    IconName::X,
                )
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip("Close Pane (the terminal keeps its tab)")
                .on_click(cx.listener(move |this, _, _, cx| this.close_terminal_pane(id, cx))),
            )
    }

    /// MonoCode's "Resize terminal" separator on the dock's inner edge; a
    /// double click restores the side's default size. The size is saved
    /// when the drag ends.
    fn render_terminal_sash(&self, side: DockSide, cx: &Context<Self>) -> impl IntoElement + use<> {
        let fg = cx.theme().colors.fg;
        let dragging = self.terminals.resizing;
        let end_drag =
            |this: &mut Self, _: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Self>| {
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
                DockSide::Bottom => el
                    .left_0()
                    .right_0()
                    .top(px(-1.0))
                    .h(px(6.0))
                    .cursor_row_resize(),
                DockSide::Top => el
                    .left_0()
                    .right_0()
                    .bottom(px(-1.0))
                    .h(px(6.0))
                    .cursor_row_resize(),
                DockSide::Left => el
                    .top_0()
                    .bottom_0()
                    .right(px(-1.0))
                    .w(px(6.0))
                    .cursor_col_resize(),
                DockSide::Right => el
                    .top_0()
                    .bottom_0()
                    .left(px(-1.0))
                    .w(px(6.0))
                    .cursor_col_resize(),
            })
            .when(dragging, |el| el.bg(fg.opacity(0.15)))
            .when(!dragging, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    let project = this.current_cwd.clone();
                    let layout = this.terminals.layout(&project);
                    if event.click_count >= 2 {
                        let reset = DockLayout {
                            size: layout.side.default_size(),
                            ..layout
                        };
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
                    this.terminals.resize_from =
                        Some((crate::ui::scale::logical(point), layout.size));
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
                    .map(|(ix, tab)| {
                        let shown = split::together(&dock.splits, dock.active, tab.id);
                        self.render_terminal_tab(ix, tab, tab.id == dock.active, shown, cx)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let menu_open = self
            .terminals
            .menu
            .as_ref()
            .is_some_and(|menu| menu.is_side());
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
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                        window.prevent_default()
                    })
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
        // Showing in a split beside the active terminal.
        shown: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = &cx.theme().colors;
        let title = SharedString::from(tab.title());
        let (id, project) = (tab.id, self.current_cwd.clone());
        let close = IconButton::new(
            SharedString::from(format!("terminal-close-{id}")),
            IconName::X,
        )
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip("Close Terminal")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.request_close_terminals(&project, vec![id], None, cx)
        }));
        let ghost = title.clone();
        let edge = colors.accent;
        div()
            .id(SharedString::from(format!("terminal-tab-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .h(px(TAB_HEIGHT))
            .pl_2()
            .pr_1()
            .max_w(px(200.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(if active || shown {
                colors.fg
            } else {
                colors.fg_muted
            })
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
            .on_drag(TerminalTabDrag { id, from: ix }, move |_, _, _, cx| {
                DragGhost::new(ghost.clone(), Some(IconName::Terminal), cx)
            })
            .drag_over::<TerminalTabDrag>(move |style, drag, _, _| {
                if drag.from == ix {
                    style
                } else {
                    style.border_color(edge)
                }
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
