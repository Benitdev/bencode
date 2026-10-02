//! The project rail: which directory BenCode is working in and the list of
//! known projects.
//!
//! Like MonoCode's `syncProjectRailOrder`, the rail order is stable:
//! selecting a project never moves it, and new projects are appended.
//! Path helpers here are pure string operations because the rail and the
//! sidebar call them on every render.

use std::collections::HashSet;

use gpui::{Context, PathPromptOptions};

use crate::app::BenCodeApp;
use crate::app::tab_scope::{ProjectReturn, plan_project_return};
use crate::app::workspace_nav::WorkspaceRequest;
use crate::db::SessionRow;

const RECENT_PROJECT_LIMIT: usize = 8;
/// Threads loaded from SQLite when switching to a project.
const PROJECT_SESSION_LIMIT: usize = 50;

/// `path` without surrounding whitespace or trailing slashes; `/` stays `/`.
fn trim_path(path: &str) -> &str {
    let trimmed = path.trim();
    let stripped = trimmed.trim_end_matches('/');
    if stripped.is_empty() && !trimmed.is_empty() {
        "/"
    } else {
        stripped
    }
}

/// Normalised form used for storing and comparing project paths.
pub fn normalize_project_path(path: &str) -> String {
    trim_path(path).to_string()
}

pub fn same_project_path(a: &str, b: &str) -> bool {
    trim_path(a) == trim_path(b)
}

/// Whether `path` is `project` itself or one of its sub-directories.
pub fn is_path_in_project(path: &str, project: &str) -> bool {
    let path = trim_path(path);
    let project = trim_path(project);
    if project.is_empty() {
        return false;
    }
    if project == "/" {
        return path.starts_with('/');
    }
    path.strip_prefix(project)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Distinct session working directories, most recent first, current one on top.
pub(super) fn recent_projects(current_cwd: &str, sessions: &[SessionRow]) -> Vec<String> {
    let mut seen = HashSet::new();
    std::iter::once(normalize_project_path(current_cwd))
        .chain(sessions.iter().map(|s| normalize_project_path(&s.cwd)))
        .filter(|cwd| !cwd.is_empty() && cwd != "~" && seen.insert(cwd.clone()))
        .take(RECENT_PROJECT_LIMIT)
        .collect()
}

impl BenCodeApp {
    /// Selecting a project on the rail (MonoCode `openProjects` followed by
    /// `workspaceNavigation.selectProject`): land on a tab of that project,
    /// then on its focused worktree.
    pub fn switch_project(&mut self, new_cwd: String, cx: &mut Context<Self>) {
        let cwd = normalize_project_path(&new_cwd);
        if cwd.is_empty() || cwd == "~" {
            return;
        }
        self.close_overlay_views();
        if same_project_path(&self.current_cwd, &cwd) {
            cx.notify();
            return;
        }
        self.load_project_sessions(&cwd);
        self.return_to_project(&cwd);
        self.set_current_project(cwd.clone());
        let focus = self.worktree_focuses.get(&cwd).cloned();
        self.navigate_workspace(&cwd, focus, WorkspaceRequest::Project);
        // Refreshes git/files for the new directory and records the landing.
        self.sync_selection(cx);
    }

    /// "Open folder…" (⌘O): picks one or more folders and opens each as a
    /// project; the last one chosen ends up focused.
    pub fn open_project_dialog(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Open Project".into()),
        });
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return, // dismissed
                Ok(Err(err)) => {
                    log::error!("folder picker failed: {err:#}");
                    return;
                }
            };
            let opened = this.update(cx, |app, cx| {
                for path in paths {
                    app.switch_project(path.to_string_lossy().into_owned(), cx);
                }
            });
            if let Err(err) = opened {
                log::debug!("project picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Opening a project leaves the full-screen views, as in MonoCode.
    fn close_overlay_views(&mut self) {
        self.is_search_open = false;
        self.is_inbox_open = false;
        self.is_notes_open = false;
        self.is_automations_open = false;
    }

    fn return_to_project(&mut self, cwd: &str) {
        let focused = self
            .selected_session()
            .map(|s| (s, self.is_agent_running_in(&s.id)));
        let plan = plan_project_return(
            &self.project_return,
            self.tabs.tabs(),
            &self.sessions,
            focused,
            cwd,
        );
        match plan {
            ProjectReturn::Keep => {}
            ProjectReturn::Activate { pane_id } => {
                self.tabs.focus(&pane_id);
            }
            ProjectReturn::ReuseBlank { session_id } => self.move_session(&session_id, cwd, None),
            ProjectReturn::Create => {
                let id = self.create_session_row(cwd);
                self.tabs.open(&id);
            }
        }
    }

    /// Follows the focused thread into its project when it lives outside the
    /// current one (MonoCode sets the project from the opened session).
    pub(super) fn follow_focused_session_project(&mut self) {
        let Some(cwd) = self
            .selected_session()
            .map(|s| normalize_project_path(&s.cwd))
            .filter(|cwd| !cwd.is_empty() && cwd != "~")
        else {
            return;
        };
        if !same_project_path(&cwd, &self.current_cwd) {
            self.set_current_project(cwd);
        }
    }

    fn set_current_project(&mut self, cwd: String) {
        if !self
            .recent_projects
            .iter()
            .any(|p| same_project_path(p, &cwd))
        {
            self.recent_projects.push(cwd.clone());
        }
        self.current_cwd = cwd;
        self.file_tree.dir_cache.clear();
        self.file_tree.expanded_paths.clear();
        self.file_tree.selected_path = None;
        self.selected_diff_path = None;
    }

    /// Adds the project's threads that are not in memory yet.
    fn load_project_sessions(&mut self, cwd: &str) {
        let rows = match self.db.list_sessions_for_cwd(cwd, PROJECT_SESSION_LIMIT) {
            Ok(rows) => rows,
            Err(err) => {
                log::error!("failed to load sessions for {cwd}: {err:#}");
                return;
            }
        };
        let known: HashSet<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
        self.sessions
            .extend(rows.into_iter().filter(|row| !known.contains(&row.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_projects_are_unique_and_current_first() {
        let session = |cwd: &str| SessionRow {
            cwd: cwd.into(),
            ..Default::default()
        };
        let projects = recent_projects(
            "/here",
            &[session("/a"), session("/here/"), session("/a"), session("")],
        );
        assert_eq!(projects, ["/here", "/a"]);
    }

    #[test]
    fn project_paths_ignore_trailing_slashes() {
        assert!(same_project_path("/a/b/", "/a/b"));
        assert!(!same_project_path("/a/b", "/a/bc"));
        assert_eq!(normalize_project_path(" / "), "/");
    }

    #[test]
    fn sub_directories_belong_to_their_project() {
        assert!(is_path_in_project("/a/b", "/a/b/"));
        assert!(is_path_in_project("/a/b/c", "/a/b"));
        assert!(!is_path_in_project("/a/bc", "/a/b"));
        assert!(is_path_in_project("/anything", "/"));
        assert!(!is_path_in_project("/a", ""));
    }
}
