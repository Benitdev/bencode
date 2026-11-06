//! Dock terminals moved out to a tab beside the chat (MonoCode
//! `shell/terminal_panes.rs`, where they become a pane or a title tab).
//! Dragging a dock tab onto the workspace, or its menu's Move to Tab,
//! shows the live shell in a file pane tab (`PaneTab::Terminal`) instead.
//! The project's dock keeps the terminal, so the job poll, the saved
//! sessions and the exit line work as they do in the dock; the dock shows
//! the others, and a fresh shell when none is left (MonoCode keeps the
//! panel's shell). Closing the tab ends the shell, asking first while a job
//! runs in it; Move to Terminal Panel takes it back. A restart shows it in
//! the dock again.

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, DragMoveEvent, InteractiveElement, IntoElement, ParentElement, Stateful,
    Styled, div, prelude::*,
};

use super::{TerminalTab, TerminalTabDrag, running};
use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::ui::tokens::Text;

impl BenCodeApp {
    /// The project whose dock holds terminal `id`, and the terminal.
    fn terminal_home<'a>(&self, id: u64, cx: &'a gpui::App) -> Option<(&'a str, &'a TerminalTab)> {
        self.terminals(cx).docks.iter().find_map(|(project, dock)| {
            let tab = dock.tabs.iter().find(|tab| tab.id == id)?;
            Some((project.as_str(), tab))
        })
    }

    /// Terminal `id` shows in a tab beside the chat.
    pub(crate) fn terminal_in_tab(&self, id: u64, cx: &gpui::App) -> bool {
        self.terminal_home(id, cx)
            .is_some_and(|(_, tab)| tab.in_pane)
    }

    /// Moves dock terminal `id` to a tab beside the chat, with the keyboard.
    pub(crate) fn move_terminal_to_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let moved = self.terminals_mut(cx, |docks| {
            let (project, dock) = (docks.docks.iter_mut())
                .find(|(_, dock)| dock.tabs.iter().any(|tab| tab.id == id))?;
            let ix = dock.tabs.iter().position(|tab| tab.id == id)?;
            if !dock.tabs[ix].in_pane {
                dock.leave_split(id);
                dock.tabs[ix].in_pane = true;
                if dock.active == id {
                    dock.active = dock.neighbour(ix);
                }
            }
            Some(project.clone())
        });
        let Some(project) = moved else {
            return;
        };
        // MonoCode `ensure_dock_terminal`: the open panel keeps a shell.
        let emptied =
            (self.terminals(cx).dock(&project)).is_some_and(|dock| dock.docked().next().is_none());
        if project == self.current_cwd && emptied && self.is_terminal_open(cx) {
            self.new_terminal(cx);
        }
        self.show_terminal_tab(id, cx);
    }

    /// Move Terminal to Tab: the current project's active dock terminal.
    pub(crate) fn move_active_terminal_to_tab(&mut self, cx: &mut Context<Self>) {
        let active = (self.terminals(cx).dock(&self.current_cwd))
            .filter(|_| self.is_terminal_open(cx))
            .and_then(|dock| dock.docked().find(|tab| tab.id == dock.active))
            .map(|tab| tab.id);
        if let Some(id) = active {
            self.move_terminal_to_tab(id, cx);
        }
    }

    /// Takes terminal `id` back from its tab to its dock, shown and active.
    pub(crate) fn move_terminal_to_dock(&mut self, id: u64, cx: &mut Context<Self>) {
        let project = self.terminals_mut(cx, |docks| {
            let (project, dock) = (docks.docks.iter_mut())
                .find(|(_, dock)| dock.tabs.iter().any(|tab| tab.id == id))?;
            let tab = dock.tabs.iter_mut().find(|tab| tab.id == id)?;
            tab.in_pane = false;
            dock.active = id;
            Some(project.clone())
        });
        self.drop_pane_tab(&PaneTab::Terminal { id }.key(), cx);
        if let Some(project) = project.filter(|project| *project == self.current_cwd) {
            self.terminals_state.as_mut(cx).set_open(&project, true);
            self.save_settings(cx);
            self.focus_terminal(id, cx);
        }
        cx.notify();
    }

    /// Opens (or focuses) terminal `id`'s tab and gives it the keyboard.
    pub(crate) fn show_terminal_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        self.open_pane_tab(PaneTab::Terminal { id }, true, cx);
        self.focus_terminal(id, cx);
    }

    fn focus_terminal(&self, id: u64, cx: &mut Context<Self>) {
        if let Some((_, tab)) = self.terminal_home(id, cx) {
            let focus = gpui::Focusable::focus_handle(tab.entity.read(cx), cx);
            crate::ui::composer::focus_later(focus, cx);
        }
    }

    /// After the file pane's tabs changed: a terminal whose tab went
    /// without its shell being closed goes back to its dock.
    pub(crate) fn settle_terminal_tabs(&mut self, cx: &mut Context<Self>) {
        let open: Vec<u64> = (self.file_pane.entries().iter())
            .filter_map(|entry| match entry.tab {
                PaneTab::Terminal { id } => Some(id),
                _ => None,
            })
            .collect();
        self.terminals_mut(cx, |docks| {
            for tab in docks.docks.values_mut().flat_map(|dock| &mut dock.tabs) {
                if tab.in_pane && !open.contains(&tab.id) {
                    tab.in_pane = false;
                }
            }
        });
    }

    /// The tab's label: the job it runs, else its folder.
    pub(crate) fn terminal_tab_label(&self, id: u64, cx: &gpui::App) -> Option<String> {
        self.terminal_home(id, cx).map(|(_, tab)| tab.title())
    }

    /// Terminal `id` filling its tab, with the exit line once its shell ended.
    pub(crate) fn render_terminal_tab_page(&self, id: u64, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let Some((_, tab)) = self.terminal_home(id, cx) else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(Text::Base.px())
                .text_color(colors.fg_muted)
                .child("This terminal was closed.")
                .into_any_element();
        };
        let exited = tab.entity.read(cx).exit_status();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .p_1()
            .child(div().flex_1().min_h_0().min_w_0().child(tab.view.clone()))
            .children(exited.map(|status| {
                div()
                    .flex_none()
                    .px_2()
                    .pb_1()
                    .font_family(cx.theme().mono_family.clone())
                    .text_size(Text::Base.px())
                    .text_color(colors.fg_muted)
                    .child(running::exited_line(status.code()))
            }))
            .into_any_element()
    }

    /// The workspace takes a dock tab dropped on it, moving the terminal
    /// to a tab; while one is over it, the side the file pane opens on is
    /// lit, as a pane's edge is.
    pub(super) fn terminal_tab_drop_target(
        &self,
        main: Stateful<gpui::Div>,
        cx: &Context<Self>,
    ) -> Stateful<gpui::Div> {
        let shown = self.terminals(cx).tab_drop && cx.has_active_drag();
        let left = self.file_pane_left;
        main.on_drag_move::<TerminalTabDrag>(cx.listener(
            |this, event: &DragMoveEvent<TerminalTabDrag>, _, cx| {
                let over = event.bounds.contains(&event.event.position);
                if this.terminals(cx).tab_drop != over {
                    this.terminals_state.as_mut(cx).tab_drop = over;
                    cx.notify();
                }
            },
        ))
        .on_drop(cx.listener(|this, drag: &TerminalTabDrag, _, cx| {
            this.terminals_state.as_mut(cx).tab_drop = false;
            this.move_terminal_to_tab(drag.id, cx);
        }))
        .when(shown, |main| main.child(tab_drop_hint(left, cx)))
    }
}

/// The wash over the half a moved terminal's tab opens in.
fn tab_drop_hint(left: bool, cx: &gpui::App) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let wash = div()
        .absolute()
        .top_0()
        .bottom_0()
        .w_1_2()
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .bg(colors.accent.opacity(0.18))
        .border_color(colors.accent)
        .text_size(Text::Base.px())
        .text_color(colors.fg)
        .child(
            Icon::new(IconName::Terminal)
                .size(IconSize::Sm)
                .color(colors.fg),
        )
        .child("Open in a tab");
    let wash = if left {
        wash.left_0().border_l_2()
    } else {
        wash.right_0().border_r_2()
    };
    div().absolute().inset_0().child(wash)
}
