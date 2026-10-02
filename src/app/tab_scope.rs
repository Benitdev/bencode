//! Which tabs belong to which project and worktree, and where focus goes when
//! the project, the worktree or the open tabs change.
//!
//! Pure ports of MonoCode's `workspaceTabGroups.ts` (`workspaceTabCwd`,
//! `workspaceTabWorktree`, `planWorkspaceTabClose`), `projectReturn.ts`
//! (`planProjectReturn`) and the `deckProjectTabs` filter in `App.tsx`.
//! A tab's project is its first chat pane's `cwd`; its workspace is that
//! pane's worktree (or the project folder).

use std::collections::HashMap;

use crate::app::same_project_path;
use crate::db::SessionRow;
use crate::ui::layout::WorkspaceTab;

/// The first chat pane of `tab` that belongs to a project.
fn first_project_session<'a>(
    tab: &WorkspaceTab,
    sessions: &'a [SessionRow],
) -> Option<&'a SessionRow> {
    tab.leaf_ids().iter().find_map(|id| {
        sessions
            .iter()
            .find(|s| &s.id == id)
            .filter(|s| !s.cwd.is_empty() && s.cwd != "~")
    })
}

/// Project folder of `tab`; `None` for a tab with no project.
pub fn tab_project<'a>(tab: &WorkspaceTab, sessions: &'a [SessionRow]) -> Option<&'a str> {
    first_project_session(tab, sessions).map(|s| s.cwd.as_str())
}

/// Working copy `tab` runs in: its first chat's worktree, else its project.
pub fn tab_workspace<'a>(tab: &WorkspaceTab, sessions: &'a [SessionRow]) -> Option<&'a str> {
    first_project_session(tab, sessions).map(SessionRow::work_dir)
}

fn in_project(tab: &WorkspaceTab, sessions: &[SessionRow], project: &str) -> bool {
    tab_project(tab, sessions).is_some_and(|cwd| same_project_path(cwd, project))
}

/// Tabs without a working copy show in every workspace.
fn in_workspace(tab: &WorkspaceTab, sessions: &[SessionRow], workspace: &str) -> bool {
    tab_workspace(tab, sessions).is_none_or(|path| same_project_path(path, workspace))
}

/// Tabs of `project` that run in `workspace`, in tab order.
pub fn scoped_tabs<'a>(
    tabs: &'a [WorkspaceTab],
    sessions: &[SessionRow],
    project: &str,
    workspace: &str,
) -> Vec<&'a WorkspaceTab> {
    tabs.iter()
        .filter(|t| in_project(t, sessions, project) && in_workspace(t, sessions, workspace))
        .collect()
}

/// The tabs the title bar shows: the current project's tabs in the focused
/// workspace, plus the active tab. A projectless active tab stands alone.
pub fn deck_tabs<'a>(
    tabs: &'a [WorkspaceTab],
    active_id: Option<&str>,
    sessions: &[SessionRow],
    project: &str,
    workspace: &str,
) -> Vec<&'a WorkspaceTab> {
    let active = tabs.iter().find(|t| Some(t.id.as_str()) == active_id);
    if let Some(active) = active
        && tab_project(active, sessions).is_none()
    {
        return vec![active];
    }
    tabs.iter()
        .filter(|t| {
            Some(t.id.as_str()) == active_id
                || (in_project(t, sessions, project) && in_workspace(t, sessions, workspace))
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum TabClosePlan {
    /// Closing would leave the project without a tab on screen: do nothing.
    Keep,
    Close {
        next_active: String,
    },
}

/// Where focus lands when `closing_id` closes: the nearest tab (left first)
/// in the same project and workspace. MonoCode's `planWorkspaceTabClose` with
/// the `project` scope.
pub fn plan_tab_close(
    tabs: &[WorkspaceTab],
    sessions: &[SessionRow],
    closing_id: &str,
) -> TabClosePlan {
    let Some(ix) = tabs.iter().position(|t| t.id == closing_id) else {
        return TabClosePlan::Keep;
    };
    if tabs.len() == 1 {
        return TabClosePlan::Keep;
    }
    let closing = &tabs[ix];
    let Some(project) = tab_project(closing, sessions) else {
        let neighbour = if ix > 0 { ix - 1 } else { ix + 1 };
        return TabClosePlan::Close {
            next_active: tabs[neighbour].id.clone(),
        };
    };
    let workspace = tab_workspace(closing, sessions);
    let same_scope = |tab: &WorkspaceTab| {
        in_project(tab, sessions, project)
            && workspace.is_none_or(|path| in_workspace(tab, sessions, path))
    };
    tabs[..ix]
        .iter()
        .rev()
        .chain(&tabs[ix + 1..])
        .find(|t| same_scope(t))
        .map_or(TabClosePlan::Keep, |t| TabClosePlan::Close {
            next_active: t.id.clone(),
        })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProjectReturn {
    /// The focused pane already belongs to the project.
    Keep,
    Activate {
        pane_id: String,
    },
    /// Move the focused, still empty thread into the project.
    ReuseBlank {
        session_id: String,
    },
    Create,
}

fn pane_in_project(sessions: &[SessionRow], pane_id: &str, project: &str) -> bool {
    sessions
        .iter()
        .any(|s| s.id == pane_id && same_project_path(&s.cwd, project))
}

/// Mirrors MonoCode's `isBlankSession`.
pub fn is_blank_session(session: &SessionRow, running: bool) -> bool {
    !running && session.blocks.iter().all(|b| b.role != "user")
}

/// What to show when `project` is selected on the rail: the focused pane if
/// it is already there, else the pane last focused in that project, else the
/// first open pane in it, else the empty focused thread, else a new thread.
/// `focused` is the focused thread and whether its agent is running.
pub fn plan_project_return(
    memory: &HashMap<String, String>,
    tabs: &[WorkspaceTab],
    sessions: &[SessionRow],
    focused: Option<(&SessionRow, bool)>,
    project: &str,
) -> ProjectReturn {
    if let Some((session, _)) = focused
        && same_project_path(&session.cwd, project)
    {
        return ProjectReturn::Keep;
    }
    let remembered = memory
        .get(project)
        .filter(|pane| tabs.iter().any(|t| t.contains(pane)))
        .filter(|pane| pane_in_project(sessions, pane, project));
    if let Some(pane) = remembered {
        return ProjectReturn::Activate {
            pane_id: pane.clone(),
        };
    }
    let open_pane = tabs
        .iter()
        .flat_map(WorkspaceTab::leaf_ids)
        .find(|pane| pane_in_project(sessions, pane, project));
    if let Some(pane_id) = open_pane {
        return ProjectReturn::Activate { pane_id };
    }
    match focused {
        Some((session, running)) if is_blank_session(session, running) => {
            ProjectReturn::ReuseBlank {
                session_id: session.id.clone(),
            }
        }
        _ => ProjectReturn::Create,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Block;
    use crate::ui::layout::leaf;

    fn session(id: &str, cwd: &str, worktree: Option<&str>) -> SessionRow {
        SessionRow {
            id: id.into(),
            cwd: cwd.into(),
            worktree_cwd: worktree.map(Into::into),
            blocks: vec![Block::new("b1", "user", "hi")],
            ..Default::default()
        }
    }

    fn tab(id: &str, pane: &str) -> WorkspaceTab {
        WorkspaceTab {
            id: id.into(),
            layout: leaf(pane),
            focused: pane.into(),
        }
    }

    fn ids(tabs: Vec<&WorkspaceTab>) -> Vec<&str> {
        tabs.into_iter().map(|t| t.id.as_str()).collect()
    }

    fn fixture() -> (Vec<WorkspaceTab>, Vec<SessionRow>) {
        let sessions = vec![
            session("a1", "/a", None),
            session("b1", "/b", None),
            session("a2", "/a", Some("/a-worktrees/x")),
            session("a3", "/a", None),
        ];
        let tabs = vec![
            tab("t1", "a1"),
            tab("t2", "b1"),
            tab("t3", "a2"),
            tab("t4", "a3"),
        ];
        (tabs, sessions)
    }

    #[test]
    fn deck_shows_only_the_current_project_and_workspace() {
        let (tabs, sessions) = fixture();
        let deck = deck_tabs(&tabs, Some("t1"), &sessions, "/a", "/a");
        assert_eq!(ids(deck), ["t1", "t4"]);

        let deck = deck_tabs(&tabs, Some("t3"), &sessions, "/a", "/a-worktrees/x");
        assert_eq!(ids(deck), ["t3"]);
    }

    #[test]
    fn deck_always_keeps_the_active_tab() {
        let (tabs, sessions) = fixture();
        let deck = deck_tabs(&tabs, Some("t2"), &sessions, "/a", "/a");
        assert_eq!(ids(deck), ["t1", "t2", "t4"]);
    }

    #[test]
    fn scoped_tabs_filter_by_project_and_workspace() {
        let (tabs, sessions) = fixture();
        assert_eq!(ids(scoped_tabs(&tabs, &sessions, "/a", "/a")), ["t1", "t4"]);
        assert_eq!(
            ids(scoped_tabs(&tabs, &sessions, "/a", "/a-worktrees/x")),
            ["t3"]
        );
    }

    #[test]
    fn closing_moves_to_a_tab_of_the_same_project_and_workspace() {
        let (tabs, sessions) = fixture();
        assert_eq!(
            plan_tab_close(&tabs, &sessions, "t4"),
            TabClosePlan::Close {
                next_active: "t1".into()
            },
            "skips t3 (other worktree) and t2 (other project)"
        );
        assert_eq!(
            plan_tab_close(&tabs, &sessions, "t2"),
            TabClosePlan::Keep,
            "last tab of /b never jumps to /a"
        );
        assert_eq!(plan_tab_close(&tabs, &sessions, "t3"), TabClosePlan::Keep);
    }

    #[test]
    fn project_return_prefers_remembered_then_first_open_pane() {
        let (tabs, sessions) = fixture();
        let focused = Some((&sessions[1], false));
        let mut memory = HashMap::new();
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, focused, "/a"),
            ProjectReturn::Activate {
                pane_id: "a1".into()
            }
        );
        memory.insert("/a".to_string(), "a3".to_string());
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, focused, "/a"),
            ProjectReturn::Activate {
                pane_id: "a3".into()
            }
        );
        let focused_in_a = Some((&sessions[0], false));
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, focused_in_a, "/a"),
            ProjectReturn::Keep
        );
    }

    #[test]
    fn project_return_reuses_a_blank_thread_or_creates_one() {
        let (tabs, sessions) = fixture();
        let mut blank = session("new", "/b", None);
        blank.blocks.clear();
        let memory = HashMap::new();
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, Some((&blank, false)), "/c"),
            ProjectReturn::ReuseBlank {
                session_id: "new".into()
            }
        );
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, Some((&blank, true)), "/c"),
            ProjectReturn::Create,
            "a running thread is never moved"
        );
        assert_eq!(
            plan_project_return(&memory, &tabs, &sessions, Some((&sessions[1], false)), "/c"),
            ProjectReturn::Create
        );
    }
}
