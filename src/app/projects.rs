//! The project rail: which directory BenCode is working in and the list of
//! known projects.
//!
//! Like MonoCode's `syncProjectRailOrder`, the rail order is stable:
//! selecting a project never moves it, and new projects are appended.
//! Path helpers here are pure string operations because the rail and the
//! sidebar call them on every render.

use std::collections::HashSet;

use gpui::Context;

use crate::app::BenCodeApp;
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
    /// Makes `cwd` the active project and shows a thread from it: an open
    /// pane, else its latest thread, else a fresh one.
    pub fn switch_project(&mut self, new_cwd: String, cx: &mut Context<Self>) {
        let cwd = normalize_project_path(&new_cwd);
        if cwd.is_empty() || same_project_path(&self.current_cwd, &cwd) {
            return;
        }
        self.set_current_project(cwd.clone());
        self.load_project_sessions(&cwd);
        match self.project_session_to_show(&cwd) {
            Some(id) => self.select_session(id, cx),
            None => self.start_session_in(&cwd, cx),
        }
        // `select_session` refreshes the workspace for the new directory.
        cx.notify();
    }

    /// Follows the focused thread into its project when it lives outside the
    /// current one (e.g. switching to a tab opened from another project).
    pub(super) fn follow_focused_session_project(&mut self) {
        let Some(cwd) = self
            .selected_session()
            .map(|s| normalize_project_path(&s.cwd))
            .filter(|cwd| !cwd.is_empty() && cwd != "~")
        else {
            return;
        };
        if is_path_in_project(&cwd, &self.current_cwd) {
            return;
        }
        let project = self
            .recent_projects
            .iter()
            .find(|p| is_path_in_project(&cwd, p))
            .cloned()
            .unwrap_or(cwd);
        self.set_current_project(project);
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
        self.worktree_focus = None;
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

    /// An open pane in `cwd` (active tab first), else its latest thread.
    fn project_session_to_show(&self, cwd: &str) -> Option<String> {
        let in_project = |id: &str| {
            self.sessions
                .iter()
                .any(|s| s.id == id && is_path_in_project(&s.cwd, cwd))
        };
        let active_id = self.tabs.active_id();
        let open_pane = self
            .tabs
            .active()
            .into_iter()
            .chain(
                self.tabs
                    .tabs()
                    .iter()
                    .filter(|t| Some(t.id.as_str()) != active_id),
            )
            .flat_map(|tab| tab.leaf_ids())
            .find(|id| in_project(id));
        open_pane.or_else(|| {
            self.sessions
                .iter()
                .filter(|s| is_path_in_project(&s.cwd, cwd))
                .max_by_key(|s| s.updated_at)
                .map(|s| s.id.clone())
        })
    }

    /// Moves the focused thread to `cwd` if nothing was sent in it yet,
    /// otherwise starts a new thread there.
    fn start_session_in(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let blank = self
            .selected_session_mut()
            .filter(|s| s.blocks.iter().all(|b| b.role != "user"));
        let id = match blank {
            Some(session) => {
                session.cwd = cwd.to_string();
                let id = session.id.clone();
                self.persist_session(&id);
                id
            }
            None => self.create_session_row(cwd),
        };
        self.select_session(id, cx);
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
