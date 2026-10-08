//! Workspace tabs and split panes: selection, focus, split / close, docking,
//! resizing and the session rows panes are created from.
//!
//! Tab and layout state lives in `ui::layout::TabSet`, which keeps the
//! invariants; these methods persist rows, sync `selected_session_id` from
//! the active tab and notify GPUI.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::Context;

use super::tab_scope::{TabClosePlan, deck_tabs, is_blank_session, plan_tab_close};
use super::{BenCodeApp, NEW_SESSION_TITLE, now_ms};
use crate::db::SessionRow;
use crate::harness::{HarnessKind, catalog};
use crate::ui::drag_drop::PaneDropTarget;
use crate::ui::layout::{
    FocusDir, LayoutNode, PaneEdge, SplitDir, WorkspaceTab, leaf, neighbor_leaf_id,
};

/// Suffix for session ids, so two sessions made in one millisecond differ.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

impl BenCodeApp {
    /// Makes `selected_session_id` (and the model picker) follow the active
    /// tab's focused pane.
    pub(crate) fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let focused = self.tabs.focused_session().map(str::to_string);
        if focused != self.selected_session_id {
            // MonoCode leaves edit mode when the thread changes.
            if self.editing_last_turn.is_some() {
                self.leave_edit_last_turn(cx);
            }
            if let Some(old_id) = self.selected_session_id.clone() {
                let current_prompt = self.prompt_input.read(cx).text().to_string();
                let tags = std::mem::take(&mut *self.mcp_tags.borrow_mut());
                if current_prompt.is_empty() && tags.is_empty() {
                    // Nothing to keep: no state is made for a thread just visited.
                    if let Some(thread) = self.threads.get_mut(&old_id) {
                        thread.draft = None;
                        thread.mcp_tags = None;
                    }
                } else {
                    let thread = self.thread_mut(&old_id);
                    thread.draft = (!current_prompt.is_empty()).then_some(current_prompt);
                    thread.mcp_tags = (!tags.is_empty()).then_some(tags);
                }
            }
            // MonoCode closes the composer's pickers when the thread changes.
            self.mcp_picker = None;
            self.folder_picker = None;
            self.skill_draft = None;
            self.token_picker = None;
            *self.mcp_tags.borrow_mut() = focused
                .as_ref()
                .and_then(|id| self.threads.get_mut(id)?.mcp_tags.take())
                .unwrap_or_default();
            let restored = focused
                .as_ref()
                .and_then(|id| self.thread(id)?.draft.as_ref())
                .cloned()
                .unwrap_or_default();
            self.prompt_input.update(cx, |input, cx| {
                input.set_text(restored, cx);
            });
            self.sync_prompt_placeholder(cx);
            // MonoCode's focus effect: a newly focused thread's prompt
            // takes the keyboard.
            if focused.is_some() {
                self.refocus_prompt(cx);
            }
        }
        let model = focused
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|s| s.model.as_str())
            .filter(|model| catalog::is_model_key(model))
            .map(str::to_string);
        if let Some(model) = model {
            self.selected_model = model;
        }
        self.selected_session_id = focused;
        self.sync_prompt_placeholder(cx);
        self.follow_focused_session_project();
        if self.is_terminal_open() {
            // Each project has its own dock; a new one starts with a shell.
            self.ensure_project_terminal(cx);
        }
        self.remember_focused_tab();
        self.record_tab_visit();
        self.refresh_workspace_if_moved(cx);
        cx.notify();
    }

    fn record_tab_visit(&mut self) {
        let open: HashSet<&str> = self.tabs.tabs().iter().map(|t| t.id.as_str()).collect();
        let active = self.tabs.active_id();
        self.tab_history.prune(&open, active);
        if let Some(active) = active
            && !self.navigating_history
        {
            self.tab_history.record(active);
        }
    }

    /// MonoCode's Back (⌘[): the previously visited tab.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        // MonoCode `onRailBack`: Back leaves an open view first.
        if self.surface.is_some() {
            self.close_surface(cx);
            return;
        }
        if let Some(id) = self.tab_history.back() {
            self.visit_from_history(&id, cx);
        }
    }

    /// MonoCode's Forward (⌘]).
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        // MonoCode `onRailForward` closes every view; Settings closes onto
        // the view it was opened from, hence the loop.
        while self.surface.is_some() {
            self.close_surface(cx);
        }
        if let Some(id) = self.tab_history.forward() {
            self.visit_from_history(&id, cx);
        }
    }

    fn visit_from_history(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        self.navigating_history = true;
        self.switch_tab(tab_id, cx);
        self.navigating_history = false;
    }

    pub fn switch_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if self.tabs.activate(tab_id) {
            self.sync_selection(cx);
        }
    }

    /// Closes a tab and moves to the nearest tab of the same project and
    /// workspace. The last such tab stays open, so closing never jumps to
    /// another project (MonoCode `planWorkspaceTabClose`).
    pub fn close_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        match plan_tab_close(self.tabs.tabs(), &self.sessions, tab_id) {
            TabClosePlan::Close { next_active } => {
                if self.tabs.close_tab(tab_id) {
                    self.tabs.activate(&next_active);
                    self.sync_selection(cx);
                }
            }
            TabClosePlan::Keep => self.keep_last_tab(tab_id, cx),
        }
    }

    /// The project's last tab stays on screen (MonoCode `onCloseTitleTab`):
    /// a split loses its focused pane (`onClosePane`); a single thread is
    /// swapped for a fresh one of the same kind, leaving the old one in the
    /// history (`onClearTabSession`). An empty tab is already as clear as it
    /// gets.
    fn keep_last_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.tabs().iter().find(|t| t.id == tab_id) else {
            return;
        };
        if tab.layout != leaf(&tab.focused) {
            let focused = tab.focused.clone();
            if self.tabs.close_pane(&focused) {
                self.sync_selection(cx);
            }
            return;
        }
        let Some(old) = self.sessions.iter().find(|s| s.id == tab.focused).cloned() else {
            return;
        };
        if is_blank_session(&old, self.is_agent_running_in(&old.id)) {
            return;
        }
        let fresh = self.create_session_row(&old.cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == fresh) {
            // The blank replacement keeps the thread's harness, model and
            // worktree, so the tab stays where it was.
            session.harness = old.harness.clone();
            session.model = old.model.clone();
            session.runtime_mode = old.runtime_mode.clone();
            session.model_settings = old.model_settings.clone();
            session.worktree_cwd = old.worktree_cwd.clone();
            session.branch = old.branch.clone();
        }
        self.persist_session(&fresh);
        if self.tabs.replace_pane(&old.id, &fresh) {
            self.sync_selection(cx);
        }
    }

    /// Tabs shown in the title bar: the current project's, in the focused
    /// workspace, plus the active one.
    pub fn deck_tabs(&self) -> Vec<&WorkspaceTab> {
        deck_tabs(
            self.tabs.tabs(),
            self.tabs.active_id(),
            &self.sessions,
            &self.current_cwd,
            self.workspace_path(),
        )
    }

    /// The active tab's layout; `None` when no tab is open.
    pub fn active_layout(&self) -> Option<&LayoutNode> {
        self.tabs.active().map(|tab| &tab.layout)
    }

    /// Focuses an open pane, switching to its tab if needed.
    pub fn focus_pane(&mut self, pane_id: String, cx: &mut Context<Self>) {
        if self.tabs.focus(&pane_id) {
            self.sync_selection(cx);
        }
    }

    /// Moves focus to the split pane next to the focused one.
    pub fn focus_adjacent_pane(&mut self, dir: FocusDir, cx: &mut Context<Self>) {
        let next = self
            .tabs
            .active()
            .and_then(|tab| neighbor_leaf_id(&tab.layout, &tab.focused, dir));
        if let Some(next) = next {
            self.focus_pane(next, cx);
        }
    }

    /// Splits pane `pane_id` of the active tab with a new session in the
    /// split pane's working directory.
    pub fn split_pane_from(&mut self, pane_id: &str, dir: SplitDir, cx: &mut Context<Self>) {
        if !self.tabs.active().is_some_and(|tab| tab.contains(pane_id)) {
            log::warn!("split ignored: pane {pane_id} is not in the active tab");
            return;
        }
        let cwd = self
            .sessions
            .iter()
            .find(|s| s.id == pane_id)
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
            .unwrap_or_else(|| self.workspace.cwd.clone());
        let new_id = self.create_session_row(&cwd);
        self.tabs.split(pane_id, dir, &new_id);
        self.sync_selection(cx);
    }

    pub fn split_active_pane(&mut self, dir: SplitDir, cx: &mut Context<Self>) {
        let Some(current) = self.tabs.focused_session().map(str::to_string) else {
            return;
        };
        self.split_pane_from(&current, dir, cx);
    }

    /// Closes a pane; the last pane of a tab closes the tab.
    pub fn close_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let last_pane_of = self
            .tabs
            .tab_of(pane_id)
            .filter(|tab| tab.layout == leaf(pane_id))
            .map(|tab| tab.id.clone());
        if let Some(tab_id) = last_pane_of {
            self.close_tab(&tab_id, cx);
            return;
        }
        if self.tabs.close_pane(pane_id) {
            self.sync_selection(cx);
        }
    }

    /// Stores the shares reported by a split's resize handle. No notify:
    /// the Ely `SplitPane` already redraws while dragging.
    pub fn resize_split(&mut self, split_id: &str, shares: &[f32]) {
        self.tabs.resize(split_id, shares);
    }

    pub fn set_active_pane_drop(
        &mut self,
        over_id: String,
        edge: PaneEdge,
        cx: &mut Context<Self>,
    ) {
        let target = PaneDropTarget { over_id, edge };
        if self.active_pane_drop.as_ref() != Some(&target) {
            self.active_pane_drop = Some(target);
            cx.notify();
        }
    }

    /// Clears the pane drop hint if it points at `over_id` (the drag left it).
    pub fn clear_pane_drop(&mut self, over_id: &str, cx: &mut Context<Self>) {
        if self
            .active_pane_drop
            .as_ref()
            .is_some_and(|target| target.over_id == over_id)
        {
            self.active_pane_drop = None;
            cx.notify();
        }
    }

    pub fn set_active_file_drop(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        if self.active_file_drop_target != target {
            self.active_file_drop_target = target;
            cx.notify();
        }
    }

    /// Drops both drag hints. Returns whether any was set.
    pub fn clear_drop_hints(&mut self) -> bool {
        let had_hint = self.active_pane_drop.is_some() || self.active_file_drop_target.is_some();
        self.active_pane_drop = None;
        self.active_file_drop_target = None;
        had_hint
    }

    /// Docks a dragged pane onto the hinted edge of another pane.
    pub fn handle_pane_drop(&mut self, from_id: &str, to_id: &str, cx: &mut Context<Self>) {
        let edge = self
            .active_pane_drop
            .take()
            .filter(|target| target.over_id == to_id)
            .map_or(PaneEdge::Right, |target| target.edge);
        self.clear_drop_hints();
        if self.tabs.dock(from_id, to_id, edge) {
            self.sync_selection(cx);
        } else {
            cx.notify();
        }
    }

    /// MonoCode `onPlaceTabOnPane`: tab `tab_id` joins the pane `to_id` on
    /// the hinted edge. A blank thread there is replaced and dropped.
    pub fn place_title_tab_on_pane(&mut self, tab_id: &str, to_id: &str, cx: &mut Context<Self>) {
        let edge = self
            .active_pane_drop
            .take()
            .filter(|target| target.over_id == to_id)
            .map_or(PaneEdge::Right, |target| target.edge);
        self.clear_drop_hints();
        let blank = self
            .sessions
            .iter()
            .find(|s| s.id == to_id)
            .is_some_and(|s| !s.blocks.iter().any(|b| b.role == "user"))
            && !self.is_agent_running_in(to_id);
        if !self.tabs.place_tab_on_pane(tab_id, to_id, edge, blank) {
            cx.notify();
            return;
        }
        if blank {
            self.discard_blank_session(to_id);
        }
        self.sync_selection(cx);
        if blank {
            self.forget_thread_state(to_id);
        }
    }

    /// Moves a split pane into a tab of its own.
    pub fn detach_pane_to_new_tab(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        self.clear_drop_hints();
        if self.tabs.detach(pane_id).is_some() {
            self.sync_selection(cx);
        } else {
            cx.notify();
        }
    }

    pub fn reorder_open_tabs(
        &mut self,
        from_index: usize,
        to_index: usize,
        cx: &mut Context<Self>,
    ) {
        // The indices come from the title bar, which shows only `deck_tabs`.
        let position = |ix: usize| {
            let id = self.deck_tabs().get(ix).map(|t| t.id.clone())?;
            self.tabs.tabs().iter().position(|t| t.id == id)
        };
        let (Some(from), Some(to)) = (position(from_index), position(to_index)) else {
            return;
        };
        if self.tabs.reorder(from, to) {
            cx.notify();
        }
    }

    /// Deletes a session row and every trace of it: panes in all tabs and
    /// its transcript state.
    pub fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_agent_running_in(id) {
            log::warn!("refusing to delete session {id} while its agent is running");
            return;
        }
        self.vacate_pane(id);
        // Out of the list first, so nothing saves the thread again; the
        // delete then runs behind its queued saves, which would otherwise
        // bring it back.
        let removed = self
            .sessions
            .iter()
            .position(|s| s.id == id)
            .map(|ix| self.sessions.remove(ix));
        self.transcripts.remove(id);
        self.sync_selection(cx);
        self.forget_thread_state(id);
        let row = id.to_string();
        let id = id.to_string();
        self.db_then(
            cx,
            move |db| db.delete_session(&row),
            move |this, deleted, cx| match deleted {
                Ok(()) => {
                    this.checkpoints.forget(&id);
                    this.forget_folder_session(&id, cx);
                }
                Err(err) => {
                    log::error!("failed to delete session {id}: {err:#}");
                    // Still in the database: back in the list it goes.
                    this.sessions.extend(removed);
                    cx.notify();
                }
            },
        );
    }

    /// Drops what the composer and the queue kept for a thread that no
    /// longer exists (MonoCode `sessionRemoval`). Runs after
    /// `sync_selection`, which saves the old selection's draft.
    fn forget_thread_state(&mut self, id: &str) {
        self.threads.remove(id);
        self.new_worktrees.remove(id);
        self.transcripts.remove(id);
        if self.queue_editing.as_ref().is_some_and(|(sid, _)| sid == id) {
            self.queue_editing = None;
        }
    }

    /// Takes `id` out of the tabs without leaving its project: a tab it fills
    /// alone moves focus like closing it would, and the project's last tab
    /// gets a fresh thread instead of closing.
    fn vacate_pane(&mut self, id: &str) {
        let alone_in = self
            .tabs
            .tab_of(id)
            .filter(|tab| tab.layout == leaf(id))
            .map(|tab| tab.id.clone());
        let Some(tab_id) = alone_in else {
            self.tabs.remove_session(id);
            return;
        };
        match plan_tab_close(self.tabs.tabs(), &self.sessions, &tab_id) {
            TabClosePlan::Close { next_active } => {
                self.tabs.close_tab(&tab_id);
                self.tabs.activate(&next_active);
            }
            TabClosePlan::Keep => {
                let cwd = self
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map_or_else(|| self.current_cwd.clone(), |s| s.cwd.clone());
                let fresh = self.create_session_row(&cwd);
                self.tabs.replace_pane(id, &fresh);
            }
        }
    }

    /// Opens a thread from history (sidebar, search): focuses it where it is
    /// open, else shows it in place of an empty pane of the active tab, else
    /// in a new tab. MonoCode `onSelectHistorySession`.
    pub fn open_session(&mut self, id: String, cx: &mut Context<Self>) {
        let mut discarded = None;
        if !self.tabs.focus(&id) {
            match self.blank_pane_in_active_tab().filter(|blank| *blank != id) {
                Some(blank) if self.tabs.replace_pane(&blank, &id) => {
                    self.discard_blank_session(&blank);
                    discarded = Some(blank);
                }
                _ => {
                    self.tabs.open(&id);
                }
            }
        }
        self.sync_selection(cx);
        if let Some(blank) = discarded {
            self.forget_thread_state(&blank);
        }
    }

    /// The active tab's focused pane if it is an empty thread, else its
    /// first empty one.
    fn blank_pane_in_active_tab(&self) -> Option<String> {
        let tab = self.tabs.active()?;
        let is_blank = |id: &str| {
            self.sessions
                .iter()
                .find(|s| s.id == id)
                .is_some_and(|s| is_blank_session(s, self.is_agent_running_in(id)))
        };
        if is_blank(&tab.focused) {
            return Some(tab.focused.clone());
        }
        tab.leaf_ids().into_iter().find(|id| is_blank(id))
    }

    /// Drops a replaced thread that never received a prompt; MonoCode does
    /// not keep those either.
    fn discard_blank_session(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
        self.transcripts.remove(id);
        // Behind the thread's queued saves, which would bring it back.
        let row = id.to_string();
        self.db_write("drop empty session", move |db| db.delete_session(&row));
    }

    /// Creates and persists a new session row with the welcome block.
    pub fn create_session_row(&mut self, cwd: &str) -> String {
        let now = now_ms();
        let seq = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let id = format!("bencode-{now}-{seq}");
        let harness = catalog::find(&self.selected_model)
            .map(|m| m.harness)
            .unwrap_or(HarnessKind::Claude);
        let branch = Some(self.git_status.branch.clone()).filter(|b| !b.is_empty());

        // MonoCode keeps `cwd` on the project and records the worktree apart.
        let worktree_cwd = self
            .worktree_focus()
            .filter(|_| crate::app::same_project_path(cwd, &self.current_cwd))
            .map(|focus| focus.path.clone());

        let session = SessionRow {
            id: id.clone(),
            title: NEW_SESSION_TITLE.to_string(),
            cwd: cwd.to_string(),
            harness: harness.id().to_string(),
            model: self.selected_model.clone(),
            created_at: now,
            updated_at: now,
            branch,
            // No ring until the harness reports usage (MonoCode).
            context_used: None,
            context_window: None,
            blocks: Vec::new(),
            runtime_mode: Some(self.permission_mode.id().to_string()),
            worktree_cwd,
            model_settings: Some(self.preferred_model_settings(&self.selected_model, None)),
            ..Default::default()
        };

        self.sessions.insert(0, session);
        self.persist_session(&id);
        id
    }

    /// Creates a session in the current project (and focused worktree) and
    /// opens it in a new tab.
    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        self.create_new_session_id(cx);
    }

    /// `create_new_session`, returning the new thread's id.
    pub fn create_new_session_id(&mut self, cx: &mut Context<Self>) -> String {
        let cwd = self.current_cwd.clone();
        let id = self.create_session_row(&cwd);
        self.tabs.open(&id);
        self.sync_selection(cx);
        id
    }
}
