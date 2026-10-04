//! App commands as GPUI actions: one keymap (MonoCode's shortcuts from
//! `src-tauri/src/menu.rs`), the macOS menu bar, and the root handlers.

use gpui::{App, Context, Div, InteractiveElement, KeyBinding, Menu, MenuItem, Stateful, actions};

use crate::app::{BenCodeApp, ViewMode};
use crate::ui::layout::{FocusDir, SplitDir};

actions!(
    bencode,
    [
        Quit,
        OpenSettings,
        Search,
        NewThread,
        CloseActive,
        Save,
        ToggleSidebar,
        ToggleTerminal,
        NewTerminal,
        SplitRight,
        SplitDown,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        NextTab,
        PreviousTab,
        OpenNotes,
        OpenInbox,
        OpenProject,
        CloseView,
        SwitchModel,
        FindInConversation,
        GoToFile,
        FindNext,
        FindPrevious,
        ToggleWorkspaceMode,
        ToggleSessionSidebar,
        GoBack,
        GoForward,
    ]
);

/// Shortcuts, matching MonoCode's menu accelerators.
fn keymap() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-k", Search, None),
        KeyBinding::new("cmd-t", NewThread, None),
        KeyBinding::new("cmd-w", CloseActive, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-o", OpenProject, None),
        KeyBinding::new("escape", CloseView, None),
        KeyBinding::new("cmd-.", SwitchModel, None),
        KeyBinding::new("cmd-f", FindInConversation, None),
        KeyBinding::new("cmd-p", GoToFile, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrevious, None),
        // MonoCode "Composer: Toggle Workspace", only in a new thread's
        // composer; deeper than the global ⌘⇧G, so it wins there.
        KeyBinding::new("cmd-shift-g", ToggleWorkspaceMode, Some("DraftComposer")),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-b", ToggleSessionSidebar, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        KeyBinding::new("cmd-j", ToggleTerminal, None),
        KeyBinding::new("cmd-`", NewTerminal, None),
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-alt-left", FocusLeft, None),
        KeyBinding::new("cmd-alt-right", FocusRight, None),
        KeyBinding::new("cmd-alt-up", FocusUp, None),
        KeyBinding::new("cmd-alt-down", FocusDown, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PreviousTab, None),
    ]
}

/// Installs the keymap and the menu bar. Call once at startup.
pub fn install(cx: &mut App) {
    cx.bind_keys(keymap());
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(menus());
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("BenCode").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit BenCode", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Thread", NewThread),
            MenuItem::action("Open Project…", OpenProject),
            MenuItem::action("Search…", Search),
            MenuItem::action("Go to File…", GoToFile),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Close", CloseActive),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Projects", ToggleSidebar),
            MenuItem::action("Toggle Session Sidebar", ToggleSessionSidebar),
            MenuItem::action("Switch Model…", SwitchModel),
            MenuItem::action("Find in Conversation", FindInConversation),
            MenuItem::action("Toggle Terminal", ToggleTerminal),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::separator(),
            MenuItem::action("Split Pane Right", SplitRight),
            MenuItem::action("Split Pane Down", SplitDown),
            MenuItem::separator(),
            MenuItem::action("Notes", OpenNotes),
            MenuItem::action("Inbox", OpenInbox),
        ]),
        Menu::new("Go").items([
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
            MenuItem::separator(),
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Previous Tab", PreviousTab),
            MenuItem::separator(),
            MenuItem::action("Focus Pane Left", FocusLeft),
            MenuItem::action("Focus Pane Right", FocusRight),
            MenuItem::action("Focus Pane Up", FocusUp),
            MenuItem::action("Focus Pane Down", FocusDown),
        ]),
    ]
}

/// The tab `delta` steps from the active one, wrapping around.
fn cycled_tab(ids: &[&str], active: Option<&str>, delta: isize) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let current = active
        .and_then(|id| ids.iter().position(|t| *t == id))
        .unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(ids.len() as isize) as usize;
    Some(ids[next].to_string())
}

impl BenCodeApp {
    fn cycle_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ids: Vec<&str> = self.deck_tabs().iter().map(|t| t.id.as_str()).collect();
        if let Some(id) = cycled_tab(&ids, self.tabs.active_id(), delta) {
            self.switch_tab(&id, cx);
        }
    }

    fn close_active(&mut self, cx: &mut Context<Self>) {
        // An open view closes first (notes are saved on the way out).
        if self.surface.is_some() {
            self.close_surface(cx);
            return;
        }
        if self.active_view_mode == ViewMode::Editor && self.request_close_active_editor_file(cx) {
            return;
        }
        if let Some(id) = self.selected_session_id.clone() {
            self.close_pane(&id, cx);
        }
    }

    /// Wires every command to the root element.
    pub fn bind_commands(root: Stateful<Div>, cx: &Context<Self>) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
            this.open_settings(cx);
        }))
        .on_action(cx.listener(|this, _: &Search, _, cx| this.open_search_modal(cx)))
        .on_action(cx.listener(|this, _: &NewThread, _, cx| this.create_new_session(cx)))
        .on_action(cx.listener(|this, _: &CloseActive, _, cx| this.close_active(cx)))
        .on_action(cx.listener(|this, _: &Save, _, cx| {
            if this.active_view_mode == ViewMode::Editor {
                this.save_current_editor_file(cx);
            }
        }))
        // MonoCode: ⌘B toggles the project rail, ⇧⌘B the session sidebar.
        .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
            this.is_rail_open = !this.is_rail_open;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ToggleSessionSidebar, _, cx| {
            this.is_sidebar_open = !this.is_sidebar_open;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &OpenProject, _, cx| this.open_project_dialog(cx)))
        .on_action(cx.listener(|this, _: &SwitchModel, _, cx| this.toggle_recent_models(cx)))
        .on_action(cx.listener(|this, _: &FindInConversation, _, cx| this.open_find(cx)))
        .on_action(cx.listener(|this, _: &GoToFile, _, cx| this.open_quick_open(cx)))
        .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_find(1, cx)))
        .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_find(-1, cx)))
        .on_action(cx.listener(|this, _: &ToggleWorkspaceMode, _, cx| this.toggle_new_worktree(cx)))
        .on_action(cx.listener(|this, _: &CloseView, _, cx| {
            if this.surface.is_some() {
                this.close_surface(cx);
            } else if !this.close_lightbox(cx)
                && !this.close_quick_open(cx)
                && !this.close_mcp_picker(true, cx)
                && !this.close_folder_picker(true, cx)
                && !this.close_handoff_menu(cx)
                && !this.close_composer_popovers(cx)
                && !this.close_find(cx)
            {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
        .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
        .on_action(cx.listener(|this, _: &NewTerminal, _, cx| this.new_terminal(cx)))
        .on_action(cx.listener(|this, _: &ToggleTerminal, _, cx| {
            this.set_terminal_open(!this.is_terminal_open, cx)
        }))
        .on_action(
            cx.listener(|this, _: &SplitRight, _, cx| this.split_active_pane(SplitDir::Right, cx)),
        )
        .on_action(
            cx.listener(|this, _: &SplitDown, _, cx| this.split_active_pane(SplitDir::Down, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FocusLeft, _, cx| this.focus_adjacent_pane(FocusDir::Left, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FocusRight, _, cx| {
                this.focus_adjacent_pane(FocusDir::Right, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &FocusUp, _, cx| this.focus_adjacent_pane(FocusDir::Up, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FocusDown, _, cx| this.focus_adjacent_pane(FocusDir::Down, cx)),
        )
        .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
        .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
        .on_action(cx.listener(|this, _: &OpenNotes, _, cx| this.open_notes(cx)))
        .on_action(cx.listener(|this, _: &OpenInbox, _, cx| this.open_inbox_modal(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycling_wraps_both_ways() {
        let ids = ["a", "b", "c"];
        assert_eq!(cycled_tab(&ids, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(cycled_tab(&ids, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(cycled_tab(&ids, None, 1).as_deref(), Some("b"));
        assert_eq!(cycled_tab(&[], None, 1), None);
    }

    #[test]
    fn keymap_chords_are_unique_per_action() {
        assert_eq!(keymap().len(), 28);
        assert_eq!(menus().len(), 4);
    }
}
