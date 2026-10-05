//! Bottom terminal dock (MonoCode `ProjectTerminalDock`): each project keeps
//! its own terminals, opened in the project folder, under a tab strip with
//! New Terminal (⌘`). Toggled from the footer or with ⌘J.

use std::collections::HashMap;
use std::path::PathBuf;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::terminal::{Launch, Terminal, TerminalEvent};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Subscription, div, prelude::*, px,
};

use crate::app::BenCodeApp;

/// MonoCode's default height for a top/bottom dock.
const DOCK_HEIGHT: gpui::Pixels = px(220.0);
const TAB_HEIGHT: gpui::Pixels = px(28.0);

pub struct TerminalTab {
    pub id: u64,
    pub entity: Entity<Terminal>,
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
}

impl TerminalDocks {
    pub fn dock(&self, project: &str) -> Option<&TerminalDock> {
        self.docks.get(project)
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

    fn select_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) {
            dock.active = id;
            cx.notify();
        }
    }

    pub fn render_terminal_drawer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let active = self
            .terminals
            .dock(&self.current_cwd)
            .and_then(|dock| dock.tabs.iter().find(|t| t.id == dock.active))
            .map(|tab| tab.entity.clone());
        let body = match active {
            Some(terminal) => div().flex_1().min_h_0().p_1().child(terminal),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(colors.fg_muted)
                .child("No terminal. Press ⌘` to open one."),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .h(DOCK_HEIGHT)
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.bg)
            .child(self.render_terminal_tabs(cx))
            .child(body)
    }

    fn render_terminal_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let tabs = self
            .terminals
            .dock(&self.current_cwd)
            .map(|dock| {
                dock.tabs
                    .iter()
                    .map(|tab| self.render_terminal_tab(tab, tab.id == dock.active, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .px_1p5()
            .h(px(32.0))
            .border_b_1()
            .border_color(colors.border)
            .children(tabs)
            .child(
                IconButton::new("terminal-new", IconName::Plus)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("New Terminal (⌘`)")
                    .on_click(cx.listener(|this, _, _, cx| this.new_terminal(cx))),
            )
            .child(div().flex_1())
            .child(
                IconButton::new("terminal-drawer-hide", IconName::ChevronDown)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Hide Terminal (⌘J)")
                    .on_click(cx.listener(|this, _, _, cx| this.set_terminal_open(false, cx))),
            )
    }

    fn render_terminal_tab(
        &self,
        tab: &TerminalTab,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let title = tab.entity.read(cx).title().clone();
        let title = if title.is_empty() {
            SharedString::from("Terminal")
        } else {
            title
        };
        let (select_id, close_id, project) = (tab.id, tab.id, self.current_cwd.clone());
        let close = IconButton::new(
            SharedString::from(format!("terminal-close-{}", tab.id)),
            IconName::X,
        )
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip("Close Terminal")
        .on_click(cx.listener(move |this, _, _, cx| this.close_terminal(&project, close_id, cx)));
        div()
            .id(SharedString::from(format!("terminal-tab-{}", tab.id)))
            .flex()
            .items_center()
            .gap_1p5()
            .h(TAB_HEIGHT)
            .pl_2()
            .pr_1()
            .max_w(px(200.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(if active { colors.fg } else { colors.fg_muted })
            .when(active, |el| el.bg(colors.active))
            .hover(|s| s.bg(colors.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select_terminal(select_id, cx)))
            .child(
                Icon::new(IconName::Terminal)
                    .size(IconSize::Xs)
                    .color(colors.fg_muted),
            )
            .child(div().min_w_0().truncate().child(title))
            .child(close)
    }
}
