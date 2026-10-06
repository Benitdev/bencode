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
use crate::db::{MonoCodeDb, SessionRow};

/// MonoCode `recents.ts` `MAX`.
const RECENT_PROJECT_LIMIT: usize = 20;
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
        self.return_to_project(&cwd);
        self.set_current_project(cwd.clone());
        // Open panes already hold their threads, so the plan above does not
        // need the rest; they arrive from a background read.
        self.load_sessions_in_background(Some(cwd.clone()), cx);
        // A project new to the rail gets its stats.
        self.refresh_project_stats(cx);
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
                    let path = path.to_string_lossy().into_owned();
                    // MonoCode `rememberProject` takes it out of the archive.
                    app.update_rail_prefs(|prefs| prefs.with_restored(&path), cx);
                    app.switch_project(path, cx);
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
        if self.surface.is_some() {
            self.surface = None;
            self.settings_return = None;
        }
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
        self.selected_diff_path = None;
    }

    /// Adds the threads not in memory yet: `project`'s most recent ones
    /// and the current project's folder members. SQLite is read on the
    /// background executor through its own read-only connection (an
    /// in-memory database is read in place) and applied on the UI thread.
    pub(crate) fn load_sessions_in_background(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        let known: Vec<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
        let members: Vec<String> = self
            .project_folders()
            .iter()
            .flat_map(|f| f.session_ids.iter())
            .filter(|id| !known.contains(id))
            .cloned()
            .collect();
        if project.is_none() && members.is_empty() {
            return;
        }
        let request = SessionsRequest { project, known, members };
        let Some(path) = self.db.file_path() else {
            let loaded = read_sessions(&self.db, &request);
            self.apply_loaded_sessions(loaded, cx);
            return;
        };
        let task = cx.background_executor().spawn(async move {
            let db = MonoCodeDb::open_reader(&path)?;
            Ok(read_sessions(&db, &request))
        });
        cx.spawn(async move |this, cx| {
            let loaded: anyhow::Result<LoadedSessions> = task.await;
            match loaded {
                Ok(loaded) => {
                    if let Err(err) = this.update(cx, |app, cx| app.apply_loaded_sessions(loaded, cx)) {
                        log::debug!("sessions loaded after app drop: {err:#}");
                    }
                }
                Err(err) => log::error!("could not open the session reader: {err:#}"),
            }
        })
        .detach();
    }

    /// Rows already in memory win over the read: they may have changed
    /// since. Folder members the database no longer has leave their folders.
    fn apply_loaded_sessions(&mut self, loaded: LoadedSessions, cx: &mut Context<Self>) {
        let mut known: HashSet<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
        let fresh: Vec<SessionRow> = loaded
            .rows
            .into_iter()
            .filter(|row| known.insert(row.id.clone()))
            .collect();
        let gone: Vec<String> = loaded
            .gone
            .into_iter()
            .filter(|id| !known.contains(id))
            .collect();
        let changed = !fresh.is_empty();
        self.sessions.extend(fresh);
        if !gone.is_empty() {
            self.remove_sessions_from_folders(&gone, cx);
        }
        if changed {
            cx.notify();
        }
    }
}

/// What [`read_sessions`] looks up; owned so it can cross threads.
struct SessionsRequest {
    project: Option<String>,
    /// Threads already in memory, not read again.
    known: Vec<String>,
    /// Folder members not in memory.
    members: Vec<String>,
}

#[derive(Default)]
struct LoadedSessions {
    rows: Vec<SessionRow>,
    /// Folder members the database no longer has.
    gone: Vec<String>,
}

/// Failures are logged and leave that part out, as the UI-thread load did.
fn read_sessions(db: &MonoCodeDb, request: &SessionsRequest) -> LoadedSessions {
    let mut loaded = LoadedSessions::default();
    if let Some(cwd) = &request.project {
        match db.list_sessions_for_cwd(cwd, PROJECT_SESSION_LIMIT, &request.known) {
            Ok(rows) => loaded.rows = rows,
            Err(err) => log::error!("failed to load sessions for {cwd}: {err:#}"),
        }
    }
    for id in &request.members {
        if loaded.rows.iter().any(|row| &row.id == id) {
            continue;
        }
        match db.get_session(id) {
            Ok(Some(row)) => loaded.rows.push(row),
            Ok(None) => loaded.gone.push(id.clone()),
            Err(err) => log::error!("could not load folder member {id}: {err:#}"),
        }
    }
    loaded
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
