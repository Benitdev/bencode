//! App commands as GPUI actions: one keymap (MonoCode's shortcuts from
//! `src-tauri/src/menu.rs`), the macOS menu bar, and the root handlers.

use gpui::{
    App, Context, Div, InteractiveElement, KeyBinding, Menu, MenuItem, OsAction, Stateful, Window,
    actions,
};

use bencode_ui::forms as input;

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::app::git_actions::GitAction;
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
        ToggleMono,
        SwapFilePaneSide,
        NewTerminal,
        OpenBrowser,
        ReloadIgnoringCache,
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
        FindInProject,
        GoToFile,
        ShowFileHistory,
        GoToDefinition,
        RenameSelectedSession,
        DeleteSelectedSessions,
        ClearSessionSelection,
        TreeCopyPath,
        TreeCopy,
        TranscriptCopy,
        TranscriptSelectAll,
        TranscriptClearSelection,
        DiffCopy,
        DiffSelectAll,
        DiffClearSelection,
        InboxCopy,
        InboxSelectAll,
        InboxClearSelection,
        TreeCut,
        TreePaste,
        TreeRename,
        TreeDelete,
        TreeClearCut,
        FindNext,
        FindPrevious,
        ToggleWorkspaceMode,
        ToggleInspector,
        ShowExplorer,
        ShowChanges,
        GoBack,
        GoForward,
        InboxNext,
        InboxPrevious,
        InboxSendToAgent,
        InboxOpenInTracker,
        InboxOpenBeside,
        InboxFocusFilter,
        InboxFocusList,
        InboxRefresh,
        InboxToggleRead,
        InboxAssignToMe,
        InboxFocusComment,
        InboxToggleTranslation,
        InboxCopyTicket,
        InboxToggleMine,
        InboxToggleUnread,
        InboxTogglePick,
        InboxClearPicks,
        OutlineNext,
        OutlinePrevious,
        OutlineJump,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        CloseOtherTabs,
        CloseAllTabs,
        ArchiveSession,
        StageSessionChanges,
        CommitSessionChanges,
        CommitAndPushSessionChanges,
        PushSessionCommits,
        PreviousSession,
        NextSession,
        PreviousSessionInTab,
        NextSessionInTab,
        PreviousProject,
        NextProject,
        NextAgent,
        ShowLogs,
        CheckForUpdates,
    ]
);

/// ⌘1 to ⌘8 show that tab of the strip (counted from 0); `None` (⌘9)
/// shows the last one.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = bencode, no_json)]
pub struct ActivateTab(pub Option<usize>);

/// A dev server's address, opened in the in-app browser (a reply's
/// `localhost` link).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = bencode, no_json)]
pub struct OpenInBrowser(pub String);

/// Shortcuts, matching MonoCode's menu accelerators and its workspace keys
/// (`workspace/model/tabKeys.ts`).
pub(crate) fn keymap() -> Vec<KeyBinding> {
    let mut keys = vec![
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
        KeyBinding::new("cmd-shift-f", FindInProject, None),
        KeyBinding::new("cmd-p", GoToFile, None),
        KeyBinding::new("cmd-alt-h", ShowFileHistory, None),
        KeyBinding::new("f12", GoToDefinition, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrevious, None),
        // MonoCode "Composer: Toggle Workspace", only in a new thread's
        // composer; deeper than the global ⌘⇧G, so it wins there.
        KeyBinding::new("cmd-shift-g", ToggleWorkspaceMode, Some("DraftComposer")),
        // The Inbox list, while it has focus.
        // MonoCode `PromptOutline`: the rail is one tab stop; arrows walk it.
        KeyBinding::new("down", OutlineNext, Some("PromptOutline")),
        KeyBinding::new("up", OutlinePrevious, Some("PromptOutline")),
        KeyBinding::new("enter", OutlineJump, Some("PromptOutline")),
        KeyBinding::new("space", OutlineJump, Some("PromptOutline")),
        KeyBinding::new("down", InboxNext, Some("InboxList")),
        KeyBinding::new("up", InboxPrevious, Some("InboxList")),
        KeyBinding::new("j", InboxNext, Some("InboxList")),
        KeyBinding::new("k", InboxPrevious, Some("InboxList")),
        KeyBinding::new("cmd-enter", InboxSendToAgent, Some("InboxList")),
        KeyBinding::new("o", InboxOpenInTracker, Some("InboxList")),
        KeyBinding::new("b", InboxOpenBeside, Some("InboxList")),
        KeyBinding::new("/", InboxFocusFilter, Some("InboxList")),
        KeyBinding::new("r", InboxRefresh, Some("InboxList")),
        KeyBinding::new("u", InboxToggleRead, Some("InboxList")),
        KeyBinding::new("a", InboxAssignToMe, Some("InboxList")),
        KeyBinding::new("c", InboxFocusComment, Some("InboxList")),
        KeyBinding::new("t", InboxToggleTranslation, Some("InboxList")),
        KeyBinding::new("y", InboxCopyTicket, Some("InboxList")),
        KeyBinding::new("m", InboxToggleMine, Some("InboxList")),
        KeyBinding::new("shift-u", InboxToggleUnread, Some("InboxList")),
        // Picks for a batch send; Esc lets them go before it closes the
        // Inbox.
        KeyBinding::new("x", InboxTogglePick, Some("InboxList")),
        KeyBinding::new("escape", InboxClearPicks, Some("InboxList")),
        // Deeper than the global Esc: the filter field hands the keyboard
        // back to the list instead of closing the Inbox.
        KeyBinding::new("escape", InboxFocusList, Some("InboxFilter")),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-b", ToggleInspector, None),
        KeyBinding::new("cmd-shift-e", ShowExplorer, None),
        KeyBinding::new("ctrl-shift-g", ShowChanges, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        // MonoCode "View: Zoom In" / "Zoom Out" / "Reset Zoom".
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        KeyBinding::new("cmd-j", ToggleTerminal, None),
        KeyBinding::new("cmd-i", ToggleMono, None),
        KeyBinding::new("cmd-shift-o", OpenBrowser, None),
        // Chrome's hard reload. A menu item, so it also reaches a page that
        // has the keyboard.
        KeyBinding::new("cmd-shift-r", ReloadIgnoringCache, None),
        KeyBinding::new("cmd-`", NewTerminal, None),
        // MonoCode "Terminal: New Tab"; every BenCode terminal is a dock tab.
        KeyBinding::new("cmd-shift-`", NewTerminal, None),
        KeyBinding::new("cmd-~", NewTerminal, None),
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-alt-left", FocusLeft, None),
        KeyBinding::new("cmd-alt-right", FocusRight, None),
        KeyBinding::new("cmd-alt-up", FocusUp, None),
        KeyBinding::new("cmd-alt-down", FocusDown, None),
        KeyBinding::new("cmd-alt-s", SwapFilePaneSide, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PreviousTab, None),
        // The same chords as the layout reports them with Shift held.
        KeyBinding::new("cmd-}", NextTab, None),
        KeyBinding::new("cmd-{", PreviousTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new("cmd-alt-t", CloseOtherTabs, None),
        KeyBinding::new("cmd-shift-w", CloseAllTabs, None),
        KeyBinding::new("cmd-shift-a", ArchiveSession, None),
        // Stepping through the sidebar's threads and the rail's projects.
        // A focused text field keeps these chords for its own caret moves.
        KeyBinding::new("cmd-shift-up", PreviousSession, None),
        KeyBinding::new("cmd-shift-down", NextSession, None),
        KeyBinding::new("cmd-up", PreviousSessionInTab, None),
        KeyBinding::new("cmd-down", NextSessionInTab, None),
        KeyBinding::new("cmd-shift-left", PreviousProject, None),
        KeyBinding::new("cmd-shift-right", NextProject, None),
        // The Agents section's threads, those waiting on you first.
        KeyBinding::new("cmd-alt-a", NextAgent, None),
        // MonoCode session cards: F2 renames, ⌫ deletes, Esc drops the picks.
        KeyBinding::new("f2", RenameSelectedSession, Some("SessionList")),
        KeyBinding::new("backspace", DeleteSelectedSessions, Some("SessionList")),
        KeyBinding::new("delete", DeleteSelectedSessions, Some("SessionList")),
        KeyBinding::new("escape", ClearSessionSelection, Some("SessionList")),
        // MonoCode Explorer keys on the selection (else the root).
        KeyBinding::new("cmd-shift-c", TreeCopyPath, Some("FileTree")),
        KeyBinding::new("cmd-c", TreeCopy, Some("FileTree")),
        KeyBinding::new("cmd-x", TreeCut, Some("FileTree")),
        KeyBinding::new("cmd-v", TreePaste, Some("FileTree")),
        KeyBinding::new("f2", TreeRename, Some("FileTree")),
        KeyBinding::new("backspace", TreeDelete, Some("FileTree")),
        KeyBinding::new("delete", TreeDelete, Some("FileTree")),
        KeyBinding::new("escape", TreeClearCut, Some("FileTree")),
        // Transcript text the pointer selected (MonoCode's native ⌘C / ⌘A).
        KeyBinding::new("cmd-c", TranscriptCopy, Some("Transcript")),
        KeyBinding::new("cmd-a", TranscriptSelectAll, Some("Transcript")),
        KeyBinding::new("escape", TranscriptClearSelection, Some("Transcript")),
        // A review's lines the pointer selected.
        KeyBinding::new("cmd-c", DiffCopy, Some("DiffText")),
        KeyBinding::new("cmd-a", DiffSelectAll, Some("DiffText")),
        KeyBinding::new("escape", DiffClearSelection, Some("DiffText")),
        // An Inbox item's text the pointer selected.
        KeyBinding::new("cmd-c", InboxCopy, Some("InboxText")),
        KeyBinding::new("cmd-a", InboxSelectAll, Some("InboxText")),
        KeyBinding::new("escape", InboxClearSelection, Some("InboxText")),
    ];
    // Spelled out: `KeyBinding::new` takes only static keystrokes.
    let slots = [
        "cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-6", "cmd-7", "cmd-8",
    ];
    keys.extend(
        slots
            .into_iter()
            .enumerate()
            .map(|(slot, chord)| KeyBinding::new(chord, ActivateTab(Some(slot)), None)),
    );
    keys.push(KeyBinding::new("cmd-9", ActivateTab(None), None));
    keys
}

/// The first chord `keys` binds to `action` in `context` (`None`: bound
/// everywhere), in keymap syntax ("cmd-shift-o"). Settings › Shortcuts reads
/// its keys here, so the page shows what is bound.
pub(crate) fn chord_for(
    keys: &[KeyBinding],
    action: &dyn gpui::Action,
    context: Option<&str>,
) -> Option<String> {
    keys.iter()
        .find(|binding| {
            binding.action().partial_eq(action)
                && binding.predicate().map(|p| p.to_string()).as_deref() == context
        })
        .map(|binding| {
            let os = bencode_ui::theme::Platform::current().os();
            let strokes: Vec<String> = binding.keystrokes().iter().map(|k| k.unparse(os)).collect();
            strokes.join(" ")
        })
}

/// Installs the keymap and the menu bar. Call once at startup.
pub fn install(cx: &mut App) {
    cx.bind_keys(keymap());
    // With no window open the app's own handler is not on the dispatch
    // path; quitting still saves through `on_app_quit`.
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &ShowLogs, cx| crate::logging::reveal(cx));
}

/// The menu bar. Building it looks every item's shortcut up in the keymap
/// and takes AppKit a while (24ms), so it waits for the first frame; the
/// shortcuts work before it, from the bindings `install` made.
pub fn install_menus(cx: &mut App) {
    cx.set_menus(menus());
}

/// Edit (BenCode's own; MonoCode's is the webview's). Its items carry the
/// system's selectors, so a browser page, a native view GPUI's key bindings
/// do not reach, takes ⌘X ⌘C ⌘V ⌘A from the menu while it has the keyboard.
/// GPUI's inputs answer the same keys from their bindings first.
fn edit_menu() -> Menu {
    Menu::new("Edit").items([
        MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
        MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
        MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
        MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
    ])
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("BenCode").items([
            MenuItem::action("Settings…", OpenSettings),
            // MonoCode `check_for_updates`, under Settings.
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Quit BenCode", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Thread", NewThread),
            MenuItem::action("Open Project…", OpenProject),
            MenuItem::action("Search…", Search),
            MenuItem::action("Go to File…", GoToFile),
            MenuItem::action("File History", ShowFileHistory),
            MenuItem::action("Go to Definition", GoToDefinition),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Close", CloseActive),
            MenuItem::action("Close Other Tabs", CloseOtherTabs),
            MenuItem::action("Close All Tabs", CloseAllTabs),
            MenuItem::separator(),
            MenuItem::action("Archive Session", ArchiveSession),
            MenuItem::separator(),
            // MonoCode's git pill: the thread's agent stages, commits or
            // pushes only what it changed.
            MenuItem::action("Stage Session Changes", StageSessionChanges),
            MenuItem::action("Commit Session Changes", CommitSessionChanges),
            MenuItem::action(
                "Commit and Push Session Changes",
                CommitAndPushSessionChanges,
            ),
            MenuItem::action("Push Session Commits", PushSessionCommits),
        ]),
        edit_menu(),
        Menu::new("View").items([
            MenuItem::action("Toggle Navigator", ToggleSidebar),
            MenuItem::action("Toggle Inspector", ToggleInspector),
            MenuItem::action("Explorer", ShowExplorer),
            MenuItem::action("Changes", ShowChanges),
            MenuItem::action("Switch Model…", SwitchModel),
            MenuItem::action("Find in Conversation", FindInConversation),
            MenuItem::action("Find in Files…", FindInProject),
            MenuItem::action("Toggle Terminal", ToggleTerminal),
            MenuItem::action("Toggle Mono", ToggleMono),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::action("Open Browser", OpenBrowser),
            MenuItem::action("Reload Page Ignoring Cache", ReloadIgnoringCache),
            MenuItem::separator(),
            MenuItem::action("Split Pane Right", SplitRight),
            MenuItem::action("Split Pane Down", SplitDown),
            MenuItem::action("Swap Chat and File Pane", SwapFilePaneSide),
            MenuItem::separator(),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Reset Zoom", ZoomReset),
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
            MenuItem::action("Previous Session", PreviousSession),
            MenuItem::action("Next Session", NextSession),
            MenuItem::action("Previous Project", PreviousProject),
            MenuItem::action("Next Project", NextProject),
            MenuItem::action("Next Agent", NextAgent),
            MenuItem::separator(),
            MenuItem::action("Focus Pane Left", FocusLeft),
            MenuItem::action("Focus Pane Right", FocusRight),
            MenuItem::action("Focus Pane Up", FocusUp),
            MenuItem::action("Focus Pane Down", FocusDown),
        ]),
        Menu::new("Help").items([MenuItem::action("Show Logs", ShowLogs)]),
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

/// MonoCode `adjacentItemId`: the id `delta` steps from `current`, wrapping;
/// the first (or, going back, the last) when `current` is not listed.
fn adjacent_id(ids: &[String], current: Option<&str>, delta: isize) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let next = match current.and_then(|id| ids.iter().position(|other| other == id)) {
        Some(ix) => (ix as isize + delta.signum()).rem_euclid(ids.len() as isize) as usize,
        None if delta < 0 => ids.len() - 1,
        None => 0,
    };
    ids.get(next).cloned()
}

impl BenCodeApp {
    /// `ActivateTab`: tab `slot` of the strip, or the last one.
    fn activate_tab_slot(&mut self, slot: Option<usize>, cx: &mut Context<Self>) {
        let tabs = self.deck_tabs();
        let tab = match slot {
            Some(slot) => tabs.get(slot),
            None => tabs.last(),
        };
        if let Some(id) = tab.map(|tab| tab.id.clone()) {
            self.switch_tab(&id, cx);
        }
    }

    /// The thread the keys act on: the focused pane's, unless a view covers
    /// the workspace or the file pane or a terminal has the keyboard
    /// (MonoCode `focusedBusyAgentSessionId` and its `surfaceOpen` guard).
    fn keyboard_session(&self, window: &Window, cx: &App) -> Option<String> {
        if self.surface.is_some() || self.file_pane_focused || self.terminal_focused(window, cx) {
            return None;
        }
        self.selected_session_id.clone()
    }

    /// MonoCode `onArchiveFocusedSession` (⇧⌘A).
    fn archive_focused_session(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(id) = self.keyboard_session(window, cx) {
            self.set_session_archived(&id, true, cx);
        }
    }

    /// MonoCode `onNavigateSessionList`: the thread above or below in the
    /// sidebar's order, in a tab of its own or (`in_tab`) in place of the
    /// focused pane.
    fn step_session(
        &mut self,
        delta: isize,
        in_tab: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.keyboard_session(window, cx) else {
            return;
        };
        let next = adjacent_id(&self.sessions_ui.order, Some(&current), delta);
        let Some(next) = next.filter(|next| *next != current) else {
            return;
        };
        self.sessions_ui.reveal = Some(next.clone());
        if in_tab && self.tabs.replace_pane(&current, &next) {
            self.sync_selection(cx);
        } else {
            self.open_session(next, cx);
        }
    }

    /// MonoCode `onNavigateProjectList`: the rail's next or previous project.
    fn step_project(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.surface.is_some() {
            return;
        }
        let order = self.rail_order();
        let current = order
            .iter()
            .find(|path| crate::app::same_project_path(path, &self.current_cwd));
        let next = adjacent_id(&order, current.map(String::as_str), delta);
        if let Some(next) =
            next.filter(|next| !crate::app::same_project_path(next, &self.current_cwd))
        {
            self.switch_project(next, cx);
        }
    }

    /// MonoCode's Escape on a working thread: stops its turn. Only reached
    /// when nothing else on screen wanted the key.
    fn stop_focused_turn(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        match self
            .keyboard_session(window, cx)
            .filter(|id| self.is_agent_running_in(id))
        {
            Some(id) => {
                self.stop_agent(&id, cx);
                true
            }
            None => false,
        }
    }

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
        if self.close_mono_view(cx) {
            return;
        }
        if self.file_pane_focused && self.close_active_pane_tab(cx) {
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
        .on_action(cx.listener(|this, _: &Quit, _, cx| this.request_quit(cx)))
        .on_action(
            cx.listener(|this, _: &CheckForUpdates, window, cx| this.check_for_updates(window, cx)),
        )
        .on_action(cx.listener(|this, _: &Search, _, cx| this.toggle_search(cx)))
        // A browser page with the keyboard is what these zoom, as in a
        // browser; otherwise the interface.
        .on_action(
            cx.listener(|this, _: &ZoomIn, _, cx| match this.browser_with_keys() {
                Some(id) => this.step_browser_zoom(id, true, cx),
                None => this.step_ui_scale(1.0, cx),
            }),
        )
        .on_action(
            cx.listener(|this, _: &ZoomOut, _, cx| match this.browser_with_keys() {
                Some(id) => this.step_browser_zoom(id, false, cx),
                None => this.step_ui_scale(-1.0, cx),
            }),
        )
        .on_action(cx.listener(
            |this, _: &ZoomReset, _, cx| match this.browser_with_keys() {
                Some(id) => this.set_browser_zoom(id, crate::app::browser::ZOOM_DEFAULT, cx),
                None => this.set_ui_scale(crate::ui::appearance::UI_SCALE_DEFAULT, cx),
            },
        ))
        .on_action(cx.listener(|this, _: &NewThread, _, cx| this.create_new_session(cx)))
        .on_action(cx.listener(|this, _: &CloseActive, _, cx| this.close_active(cx)))
        .on_action(cx.listener(|this, _: &Save, _, cx| {
            if matches!(this.file_pane.active(), Some(PaneTab::File { .. })) {
                this.save_current_editor_file(cx);
            }
        }))
        // MonoCode: ⌘B toggles the project rail, ⇧⌘B the session sidebar.
        .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
            this.is_rail_open = !this.is_rail_open;
            // MonoCode dismisses the rail's menus when it hides.
            if !this.is_rail_open {
                this.close_rail_menu(cx);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
            this.set_inspector_open(!this.inspector_open, cx)
        }))
        .on_action(cx.listener(|this, _: &ShowExplorer, _, cx| {
            this.show_inspector(crate::app::InspectorMode::Files, cx)
        }))
        .on_action(cx.listener(|this, _: &ShowChanges, _, cx| {
            this.show_inspector(crate::app::InspectorMode::Changes, cx)
        }))
        .on_action(cx.listener(|this, _: &OpenProject, _, cx| this.open_project_dialog(cx)))
        .on_action(cx.listener(|this, _: &SwitchModel, _, cx| this.toggle_recent_models(cx)))
        .on_action(cx.listener(|this, _: &FindInConversation, _, cx| this.open_find(cx)))
        .on_action(cx.listener(|this, _: &FindInProject, _, cx| this.open_project_search(cx)))
        .on_action(cx.listener(|this, _: &GoToFile, _, cx| this.open_quick_open(cx)))
        .on_action(
            cx.listener(|this, _: &ShowFileHistory, _, cx| this.show_active_file_history(cx)),
        )
        // The file pane's editor only: F12 in the chat is the chat's.
        .on_action(cx.listener(|this, _: &GoToDefinition, window, cx| {
            if this.file_pane_focused
                && matches!(this.file_pane.active(), Some(PaneTab::File { .. }))
            {
                this.go_to_definition_at_caret(window, cx);
            }
        }))
        .on_action(
            cx.listener(|this, _: &RenameSelectedSession, _, cx| this.rename_selected_session(cx)),
        )
        .on_action(
            cx.listener(|this, _: &DeleteSelectedSessions, _, cx| {
                this.delete_selected_sessions(cx)
            }),
        )
        .on_action(cx.listener(|this, _: &TreeCopyPath, _, cx| this.tree_copy_path(cx)))
        .on_action(cx.listener(|this, _: &TreeCopy, _, cx| this.tree_clip(false, cx)))
        .on_action(
            cx.listener(|this, _: &TranscriptCopy, _, cx| this.copy_transcript_selection(cx)),
        )
        .on_action(
            cx.listener(|this, _: &TranscriptSelectAll, _, cx| this.select_all_transcript(cx)),
        )
        .on_action(cx.listener(|this, _: &TranscriptClearSelection, _, cx| {
            // Nothing selected: Esc does what it does elsewhere.
            if !this.clear_transcript_selection(cx) {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &DiffCopy, _, cx| this.copy_diff_selection(cx)))
        .on_action(cx.listener(|this, _: &DiffSelectAll, _, cx| this.select_all_diff(cx)))
        .on_action(cx.listener(|this, _: &DiffClearSelection, _, cx| {
            if !this.clear_diff_selection(cx) {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &InboxCopy, _, cx| this.copy_inbox_selection(cx)))
        .on_action(cx.listener(|this, _: &InboxSelectAll, _, cx| this.select_all_inbox(cx)))
        .on_action(cx.listener(|this, _: &InboxClearSelection, _, cx| {
            if !this.clear_inbox_selection(cx) {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &TreeCut, _, cx| this.tree_clip(true, cx)))
        .on_action(cx.listener(|this, _: &TreePaste, _, cx| this.tree_paste_selected(cx)))
        .on_action(cx.listener(|this, _: &TreeRename, _, cx| this.tree_rename_selected(cx)))
        .on_action(cx.listener(|this, _: &TreeDelete, _, cx| this.tree_delete_selected(cx)))
        .on_action(cx.listener(|this, _: &TreeClearCut, _, cx| this.tree_clear_cut(cx)))
        .on_action(cx.listener(|this, _: &ClearSessionSelection, _, cx| {
            this.sessions_ui.selection.clear();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_find(1, cx)))
        .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_find(-1, cx)))
        .on_action(cx.listener(|this, _: &ToggleWorkspaceMode, _, cx| this.toggle_new_worktree(cx)))
        .on_action(cx.listener(|this, _: &InboxNext, _, cx| this.step_inbox_selection(1, cx)))
        .on_action(
            cx.listener(|this, _: &OutlineNext, window, cx| this.step_outline(1, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &OutlinePrevious, window, cx| this.step_outline(-1, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &OutlineJump, window, cx| this.open_outline_cursor(window, cx)),
        )
        .on_action(cx.listener(|this, _: &InboxPrevious, _, cx| this.step_inbox_selection(-1, cx)))
        .on_action(
            cx.listener(|this, _: &InboxSendToAgent, _, cx| this.send_selected_inbox_item(cx)),
        )
        .on_action(
            cx.listener(|this, _: &InboxOpenInTracker, _, cx| this.open_selected_inbox_item(cx)),
        )
        .on_action(
            cx.listener(|this, _: &InboxOpenBeside, _, cx| {
                this.open_selected_inbox_item_beside(cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &InboxFocusFilter, window, cx| {
                this.focus_inbox_filter(window, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &InboxFocusList, window, cx| {
                this.return_to_inbox_list(window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &InboxRefresh, _, cx| this.refresh_inbox(cx)))
        .on_action(
            cx.listener(|this, _: &InboxToggleRead, _, cx| this.toggle_selected_inbox_read(cx)),
        )
        .on_action(
            cx.listener(|this, _: &InboxAssignToMe, _, cx| this.assign_selected_inbox_item(cx)),
        )
        .on_action(cx.listener(|this, _: &InboxFocusComment, window, cx| {
            this.focus_inbox_comment(window, cx)
        }))
        .on_action(cx.listener(|this, _: &InboxToggleTranslation, _, cx| {
            this.toggle_selected_inbox_translation(cx)
        }))
        .on_action(
            cx.listener(|this, _: &InboxCopyTicket, _, cx| this.copy_selected_inbox_ticket(cx)),
        )
        .on_action(cx.listener(|this, _: &InboxToggleMine, _, cx| this.toggle_inbox_mine(cx)))
        .on_action(cx.listener(|this, _: &InboxToggleUnread, _, cx| this.toggle_inbox_unread(cx)))
        .on_action(cx.listener(|this, _: &InboxTogglePick, _, cx| this.toggle_open_inbox_pick(cx)))
        .on_action(cx.listener(|this, _: &InboxClearPicks, _, cx| {
            if !this.clear_inbox_picks(cx) {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &CloseView, window, cx| {
            if this.search.open {
                this.close_search_modal(cx);
            } else if this.surface.is_some() {
                this.close_surface(cx);
            } else if !this.close_lightbox(cx)
                && !this.close_quick_open(cx)
                && !this.close_mcp_picker(true, cx)
                && !this.close_folder_picker(true, cx)
                && !this.close_handoff_menu(cx)
                && !this.close_composer_popovers(cx)
                && !this.close_mono_settings_layer(cx)
                && !this.close_usage_popover(cx)
                && !this.close_find(cx)
                && !this.close_project_search(cx)
                && !this.close_browser_picker(cx)
                && !this.stop_focused_turn(window, cx)
            {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
        .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
        .on_action(cx.listener(|this, _: &NewTerminal, _, cx| this.new_terminal(cx)))
        .on_action(cx.listener(|this, _: &OpenBrowser, window, cx| {
            this.open_browser(None, window, cx);
        }))
        .on_action(cx.listener(|this, action: &OpenInBrowser, window, cx| {
            this.open_url_in_browser(&action.0, window, cx);
        }))
        .on_action(cx.listener(|this, _: &ReloadIgnoringCache, _, cx| {
            if let Some(id) = this.active_browser_id() {
                this.browser_reload(id, true, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
        .on_action(cx.listener(|this, _: &SwapFilePaneSide, _, cx| this.swap_file_pane_side(cx)))
        .on_action(cx.listener(|this, _: &ToggleTerminal, _, cx| {
            this.set_terminal_open(!this.is_terminal_open(cx), cx)
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
        .on_action(cx.listener(|this, _: &CloseOtherTabs, _, cx| this.close_other_tabs(cx)))
        .on_action(cx.listener(|this, _: &CloseAllTabs, _, cx| this.close_all_tabs(cx)))
        .on_action(cx.listener(|this, tab: &ActivateTab, _, cx| this.activate_tab_slot(tab.0, cx)))
        .on_action(cx.listener(|this, _: &ArchiveSession, window, cx| {
            this.archive_focused_session(window, cx)
        }))
        .on_action(cx.listener(|this, _: &StageSessionChanges, _, cx| {
            this.git_action_command(GitAction::Stage, cx)
        }))
        .on_action(cx.listener(|this, _: &CommitSessionChanges, _, cx| {
            this.git_action_command(GitAction::Commit, cx)
        }))
        .on_action(cx.listener(|this, _: &CommitAndPushSessionChanges, _, cx| {
            this.git_action_command(GitAction::CommitPush, cx)
        }))
        .on_action(cx.listener(|this, _: &PushSessionCommits, _, cx| {
            this.git_action_command(GitAction::Push, cx)
        }))
        .on_action(cx.listener(|this, _: &PreviousSession, window, cx| {
            this.step_session(-1, false, window, cx)
        }))
        .on_action(
            cx.listener(|this, _: &NextSession, window, cx| {
                this.step_session(1, false, window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &PreviousSessionInTab, window, cx| {
            this.step_session(-1, true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &NextSessionInTab, window, cx| {
            this.step_session(1, true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &PreviousProject, _, cx| this.step_project(-1, cx)))
        .on_action(cx.listener(|this, _: &NextProject, _, cx| this.step_project(1, cx)))
        .on_action(cx.listener(|this, _: &NextAgent, _, cx| this.select_next_live_agent(cx)))
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
    fn stepping_wraps_and_starts_from_an_end() {
        let ids = ["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(adjacent_id(&ids, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(adjacent_id(&ids, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(adjacent_id(&ids, Some("gone"), 1).as_deref(), Some("a"));
        assert_eq!(adjacent_id(&ids, None, -1).as_deref(), Some("c"));
        assert_eq!(adjacent_id(&[], None, 1), None);
    }

    #[test]
    fn chords_are_found_by_action_and_context() {
        let keys = keymap();
        assert_eq!(chord_for(&keys, &NewThread, None).as_deref(), Some("cmd-t"));
        assert_eq!(
            chord_for(&keys, &ToggleWorkspaceMode, Some("DraftComposer")).as_deref(),
            Some("cmd-shift-g")
        );
        // Bound only in the session list, so not found anywhere else.
        assert_eq!(chord_for(&keys, &RenameSelectedSession, None), None);
        assert_eq!(
            chord_for(&keys, &ActivateTab(Some(1)), None).as_deref(),
            Some("cmd-2")
        );
    }

    #[test]
    fn keymap_chords_are_unique_per_context() {
        let keys = keymap();
        // No chord is bound twice in one context: the later one would win
        // silently.
        let mut seen = std::collections::HashSet::new();
        let os = bencode_ui::theme::Platform::current().os();
        for binding in &keys {
            let chord: Vec<String> = binding.keystrokes().iter().map(|k| k.unparse(os)).collect();
            let context = binding.predicate().map(|p| format!("{p:?}"));
            assert!(
                seen.insert((chord.clone(), context)),
                "{chord:?} bound twice"
            );
        }
    }
}
