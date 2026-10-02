//! Workspace tabs and split panes: selection, focus, split / close, docking,
//! resizing and the session rows panes are created from.
//!
//! Tab and layout state lives in `ui::layout::TabSet`, which keeps the
//! invariants; these methods persist rows, sync `selected_session_id` from
//! the active tab and notify GPUI.

use std::sync::atomic::{AtomicU64, Ordering};

use gpui::Context;

use super::tab_scope::{TabClosePlan, deck_tabs, plan_tab_close};
use super::{BenCodeApp, DEFAULT_CONTEXT_WINDOW, NEW_SESSION_TITLE, now_ms};
use crate::db::{Block, SessionRow};
use crate::harness::{HarnessKind, catalog};
use crate::ui::drag_drop::PaneDropTarget;
use crate::ui::layout::{
    FocusDir, LayoutNode, PaneEdge, SplitDir, WorkspaceTab, leaf, neighbor_leaf_id,
};

/// Suffix for session ids, so two sessions made in one millisecond differ.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

const WELCOME_TEXT: &str =
    "Ready for your instructions. I can edit files, run commands, and inspect git diffs.";

impl BenCodeApp {
    /// Makes `selected_session_id` (and the model picker) follow the active
    /// tab's focused pane.
    pub(super) fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let focused = self.tabs.focused_session().map(str::to_string);
        if focused != self.selected_session_id {
            self.selected_diff_path = None;
        }
        let model = focused
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|s| s.model.as_str())
            .filter(|model| catalog::find(model).is_some())
            .map(str::to_string);
        if let Some(model) = model {
            self.selected_model = model;
        }
        self.selected_session_id = focused;
        self.follow_focused_session_project();
        self.remember_focused_tab();
        self.refresh_workspace_if_moved(cx);
        cx.notify();
    }

    /// Shows a session: focuses its pane if it is already open (switching
    /// tab if needed), otherwise opens it in a new tab.
    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        self.tabs.select(&id);
        self.sync_selection(cx);
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
        let TabClosePlan::Close { next_active } =
            plan_tab_close(self.tabs.tabs(), &self.sessions, tab_id)
        else {
            return;
        };
        if self.tabs.close_tab(tab_id) {
            self.tabs.activate(&next_active);
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
        if let Err(err) = self.db.delete_session(id) {
            log::error!("failed to delete session {id}: {err:#}");
            return;
        }
        self.sessions.retain(|s| s.id != id);
        self.tabs.remove_session(id);
        self.transcripts.remove(id);
        self.sync_selection(cx);
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

        let mut welcome = Block::new("b1", "assistant", WELCOME_TEXT);
        welcome.started_at = Some(now);

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
            context_used: Some(0),
            context_window: Some(DEFAULT_CONTEXT_WINDOW),
            blocks: vec![welcome],
            worktree_cwd,
            ..Default::default()
        };

        self.sessions.insert(0, session);
        self.persist_session(&id);
        id
    }

    /// Creates a session in the current project (and focused worktree) and
    /// opens it in a new tab.
    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        let cwd = self.current_cwd.clone();
        let id = self.create_session_row(&cwd);
        self.tabs.open(&id);
        self.sync_selection(cx);
    }
}
