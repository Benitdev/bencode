//! Worktree focus and the tab that goes with it. Port of MonoCode's
//! `useWorkspaceNavigation`: each project remembers the worktree it is
//! narrowed to, and each (project, worktree) pair remembers its last tab.

use gpui::Context;

use crate::app::tab_scope::{is_blank_session, scoped_tabs, tab_project, tab_workspace};
use crate::app::{BenCodeApp, WorktreeFocus, normalize_project_path, same_project_path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspaceRequest {
    /// Selecting a project: never creates a tab, keeps the landing tab.
    Project,
    /// Picking a worktree in the switcher: opens a tab there if none exists.
    Workspace,
}

fn workspace_key(project: &str, path: &str) -> String {
    format!(
        "{}\0{}",
        normalize_project_path(project),
        normalize_project_path(path)
    )
}

impl BenCodeApp {
    /// The worktree the current project is narrowed to, if any.
    pub fn worktree_focus(&self) -> Option<&WorktreeFocus> {
        self.worktree_focuses.get(&self.current_cwd)
    }

    /// The focused workspace of the current project: its worktree, else the
    /// project folder.
    pub fn workspace_path(&self) -> &str {
        self.worktree_focus()
            .map_or(self.current_cwd.as_str(), |focus| focus.path.as_str())
    }

    /// The sidebar worktree switcher (MonoCode `onSelectWorkspace`).
    pub fn select_workspace(&mut self, focus: Option<WorktreeFocus>, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        self.navigate_workspace(&project, focus, WorkspaceRequest::Workspace);
        // Refreshes git/files when the workspace directory changed.
        self.sync_selection(cx);
    }

    /// Lands on a tab of `project` running in `focus` (or the project folder):
    /// the active tab if it already does, else the last tab used there, else
    /// the right-most one; an empty focused thread moves there instead.
    pub(super) fn navigate_workspace(
        &mut self,
        project: &str,
        focus: Option<WorktreeFocus>,
        request: WorkspaceRequest,
    ) {
        let path = focus
            .as_ref()
            .map_or(project, |f| f.path.as_str())
            .to_string();
        let Some(active) = self.tabs.active() else {
            self.set_worktree_focus(project, focus);
            return;
        };
        let active_id = active.id.clone();
        if !tab_project(active, &self.sessions).is_some_and(|p| same_project_path(p, project)) {
            self.set_worktree_focus(project, focus);
            return;
        }
        let key = workspace_key(project, &path);
        let target = if tab_workspace(active, &self.sessions)
            .is_none_or(|workspace| same_project_path(workspace, &path))
        {
            Some(active_id.clone())
        } else {
            let scoped = scoped_tabs(self.tabs.tabs(), &self.sessions, project, &path);
            let remembered = self.workspace_return.get(&key);
            scoped
                .iter()
                .find(|t| Some(&t.id) == remembered)
                .or(scoped.last())
                .map(|t| t.id.clone())
        };

        // Set first: a thread created below starts in the focused worktree.
        self.set_worktree_focus(project, focus.clone());
        let tab_id = match target {
            Some(id) => id,
            None => match self.blank_focused_session_in(project) {
                Some(session_id) => {
                    let worktree = focus.map(|f| f.path);
                    self.move_session(&session_id, project, worktree);
                    active_id
                }
                None if request == WorkspaceRequest::Project => active_id,
                None => {
                    let id = self.create_session_row(project);
                    self.tabs.open(&id)
                }
            },
        };
        self.workspace_return.insert(key, tab_id.clone());
        self.tabs.activate(&tab_id);
    }

    fn blank_focused_session_in(&self, project: &str) -> Option<String> {
        self.selected_session()
            .filter(|s| same_project_path(&s.cwd, project))
            .filter(|s| is_blank_session(s, self.is_agent_running_in(&s.id)))
            .map(|s| s.id.clone())
    }

    fn set_worktree_focus(&mut self, project: &str, focus: Option<WorktreeFocus>) {
        let key = normalize_project_path(project);
        match focus {
            Some(focus) => self.worktree_focuses.insert(key, focus),
            None => self.worktree_focuses.remove(&key),
        };
    }

    /// Re-homes a thread (MonoCode `onCwdChange` / `onWorktreeChange`).
    pub(super) fn move_session(&mut self, id: &str, project: &str, worktree: Option<String>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        session.cwd = project.to_string();
        session.worktree_cwd = worktree;
        self.persist_session(id);
    }

    /// Records where the user is, so the rail and the worktree switcher can
    /// return to it later (MonoCode `reconcileProjectReturn` and the
    /// workspace memory effect).
    pub(super) fn remember_focused_tab(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        if session.cwd.is_empty() || session.cwd == "~" {
            return;
        }
        let project = normalize_project_path(&session.cwd);
        let pane = session.id.clone();
        if let Some(tab_id) = self.tabs.active_id().map(str::to_string)
            && same_project_path(&project, &self.current_cwd)
        {
            let key = workspace_key(&project, self.workspace_path());
            self.workspace_return.insert(key, tab_id);
        }
        self.project_return.insert(project, pane);
    }
}
