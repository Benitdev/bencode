//! MonoCode `onRemoveWorktree` (`App.tsx`), `worktrees.ts` and
//! `worktrees.rs::remove_with_sessions`: deleting a linked worktree from
//! Settings › Worktrees. The one confirmation covers the whole action, so
//! the removal is always forced. Threads that used the worktree are either
//! deleted (the dialog's switch) or kept and detached: they remember the
//! worktree, drop provider state and wait for a new working copy
//! (`ui/composer/removed_worktree.rs`).

use gpui::Context;

use crate::app::file_pane::PaneTab;
use crate::app::{BenCodeApp, is_path_in_project, same_project_path};
use crate::db::SessionRow;
use crate::git::Worktree;
use crate::git::worktrees::remove_worktree;

/// The open "Delete worktree?" dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct WorktreeDeletion {
    pub tree: Worktree,
    /// Threads using the worktree when the dialog opened.
    pub session_ids: Vec<String>,
    /// "Also delete associated sessions".
    pub delete_sessions: bool,
    pub busy: bool,
    pub error: Option<String>,
}

/// MonoCode `WorktreesPage`: why the delete button is disabled.
pub fn deletion_blocker(tree: &Worktree) -> Option<&'static str> {
    if tree.locked {
        Some("Unlock this worktree in Git first")
    } else if tree.branch.is_none() {
        Some("Create a branch before deleting this detached worktree")
    } else {
        None
    }
}

/// The thread's working directory: its worktree, else its folder.
fn work_dir(session: &SessionRow) -> &str {
    session
        .worktree_cwd
        .as_deref()
        .filter(|w| !w.is_empty())
        .unwrap_or(&session.cwd)
}

/// MonoCode `worktreeSessionIds`: the stored ids, with live threads taking
/// precedence over what was last saved for them.
pub fn worktree_session_ids(path: &str, stored: &[String], sessions: &[SessionRow]) -> Vec<String> {
    let mut ids: Vec<String> = stored
        .iter()
        .filter(|id| !sessions.iter().any(|s| &s.id == *id))
        .cloned()
        .collect();
    ids.extend(
        sessions
            .iter()
            .filter(|s| !s.worktree_removed && is_path_in_project(work_dir(s), path))
            .map(|s| s.id.clone()),
    );
    ids
}

/// MonoCode `detachSessionWorktree`, in memory; the database side is
/// `MonoCodeDb::detach_worktree_sessions`.
pub fn detach_session(session: &mut SessionRow, path: &str, project_cwd: &str) {
    if session.worktree_cwd.as_deref().is_none_or(str::is_empty) {
        session.worktree_cwd = Some(session.cwd.clone());
    }
    if is_path_in_project(&session.cwd, path) {
        session.cwd = project_cwd.to_string();
    }
    session.worktree_removed = true;
    session.branch = None;
    session.provider_session_id = None;
    session.context_used = None;
    session.context_window = None;
}

impl BenCodeApp {
    /// The page's trash button.
    pub fn open_worktree_deletion(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(tree) = self
            .workspace
            .worktrees
            .iter()
            .find(|t| !t.is_main && same_project_path(&t.path, path))
            .cloned()
        else {
            return;
        };
        if deletion_blocker(&tree).is_some() {
            return;
        }
        let stored = self
            .db
            .session_ids_in_worktree(&tree.path)
            .unwrap_or_else(|err| {
                log::error!(
                    "could not list the threads of worktree {}: {err:#}",
                    tree.path
                );
                Vec::new()
            });
        self.worktrees_page_error = None;
        self.worktree_deletion = Some(WorktreeDeletion {
            session_ids: worktree_session_ids(&tree.path, &stored, &self.sessions),
            tree,
            delete_sessions: false,
            busy: false,
            error: None,
        });
        cx.notify();
    }

    pub fn close_worktree_deletion(&mut self, cx: &mut Context<Self>) {
        if self.worktree_deletion.as_ref().is_some_and(|d| d.busy) {
            return;
        }
        self.worktree_deletion = None;
        cx.notify();
    }

    pub fn set_delete_worktree_sessions(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(deletion) = self.worktree_deletion.as_mut() {
            deletion.delete_sessions = on;
            cx.notify();
        }
    }

    /// MonoCode `assertWorktreeFilesClosed`: a file tab or a terminal still
    /// open inside the worktree.
    fn worktree_in_use(&self, path: &str) -> bool {
        let workspace = self.workspace_path();
        let files = self
            .file_pane
            .entries()
            .iter()
            .any(|entry| match &entry.tab {
                PaneTab::File { path: file } => {
                    let file = if file.starts_with('/') {
                        file.clone()
                    } else {
                        format!("{}/{file}", workspace.trim_end_matches('/'))
                    };
                    is_path_in_project(&file, path)
                }
                PaneTab::Review { cwd, .. }
                | PaneTab::Changes { cwd, .. }
                | PaneTab::SessionChanges { cwd, .. }
                | PaneTab::Commit { cwd, .. } => is_path_in_project(cwd, path),
            });
        files || self.terminals.any_in(path)
    }

    fn fail_worktree_deletion(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(deletion) = self.worktree_deletion.as_mut() {
            deletion.busy = false;
            deletion.error = Some(message);
        }
        cx.notify();
    }

    /// The dialog's Delete: the sessions (if asked), then
    /// `git worktree remove --force`, then the kept threads are detached.
    pub fn confirm_worktree_deletion(&mut self, cx: &mut Context<Self>) {
        let Some(deletion) = self.worktree_deletion.clone().filter(|d| !d.busy) else {
            return;
        };
        let path = deletion.tree.path.clone();
        if self.worktree_in_use(&path) {
            return self.fail_worktree_deletion(
                "Close the files and terminals open in this worktree first.".into(),
                cx,
            );
        }
        let stored = match self.db.session_ids_in_worktree(&path) {
            Ok(ids) => ids,
            Err(err) => return self.fail_worktree_deletion(format!("{err:#}"), cx),
        };
        let ids = worktree_session_ids(&path, &stored, &self.sessions);
        if ids.iter().any(|id| self.is_agent_running_in(id)) {
            return self.fail_worktree_deletion(
                "Close the terminals and agent processes using this worktree first.".into(),
                cx,
            );
        }
        let sessions_deleted = deletion.delete_sessions && !ids.is_empty();
        if sessions_deleted {
            for id in &ids {
                self.delete_session(id, cx);
            }
            if self
                .db
                .session_ids_in_worktree(&path)
                .is_ok_and(|left| !left.is_empty())
            {
                self.worktree_deletion = None;
                self.worktrees_page_error =
                    Some("Some sessions could not be deleted, so the worktree was kept.".into());
                self.refresh_workspace(cx);
                cx.notify();
                return;
            }
        }
        if let Some(d) = self.worktree_deletion.as_mut() {
            d.busy = true;
            d.error = None;
        }
        cx.notify();
        let project = self.current_cwd.clone();
        let kept = if sessions_deleted { Vec::new() } else { ids };
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move { remove_worktree(&project, &path, true) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                this.worktree_deletion = None;
                match result {
                    Ok(()) => this.worktree_deleted(&path, &kept, cx),
                    Err(err) => {
                        log::warn!("delete worktree {path}: {err:#}");
                        this.worktrees_page_error = Some(if sessions_deleted {
                            format!("The sessions were deleted, but the worktree was kept. {err:#}")
                        } else {
                            format!("{err:#}")
                        });
                    }
                }
                this.refresh_workspace(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("worktree deletion after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Git removed the folder: detach the kept threads and leave the
    /// worktree if the sidebar was focused on it.
    fn worktree_deleted(&mut self, path: &str, kept: &[String], cx: &mut Context<Self>) {
        let project = self
            .workspace
            .worktrees
            .iter()
            .find(|t| t.is_main)
            .map_or_else(|| self.current_cwd.clone(), |t| t.path.clone());
        if let Err(err) = self.db.detach_worktree_sessions(kept, path, &project) {
            log::error!("worktree {path} deleted but its threads were not detached: {err:#}");
            self.worktrees_page_error = Some(format!(
                "The worktree was deleted, but its sessions could not be updated: {err:#}"
            ));
        }
        for session in self.sessions.iter_mut().filter(|s| kept.contains(&s.id)) {
            detach_session(session, path, &project);
        }
        if self
            .worktree_focus()
            .is_some_and(|f| is_path_in_project(&f.path, path))
        {
            self.select_workspace(None, cx);
        }
        self.sync_prompt_placeholder(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Worktree {
        Worktree {
            path: "/p-worktrees/mc-a".into(),
            branch: Some("mc/a".into()),
            head: "0123456789abcdef".into(),
            dirty: Some(false),
            ..Default::default()
        }
    }

    fn session(id: &str, cwd: &str, worktree: Option<&str>) -> SessionRow {
        SessionRow {
            id: id.into(),
            cwd: cwd.into(),
            worktree_cwd: worktree.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn locked_and_detached_worktrees_cannot_be_deleted() {
        assert_eq!(deletion_blocker(&tree()), None);
        assert_eq!(
            deletion_blocker(&Worktree {
                locked: true,
                ..tree()
            }),
            Some("Unlock this worktree in Git first")
        );
        assert_eq!(
            deletion_blocker(&Worktree {
                branch: None,
                ..tree()
            }),
            Some("Create a branch before deleting this detached worktree")
        );
    }

    #[test]
    fn live_threads_override_stored_ids() {
        let path = "/p-worktrees/mc-a";
        let mut sessions = vec![
            session("moved", "/p", None),
            session("in", "/p", Some(path)),
            session("nested", "/p-worktrees/mc-a/src", None),
            session("gone", "/p", Some(path)),
        ];
        sessions[3].worktree_removed = true;
        let stored = ["moved".to_string(), "archived".to_string()];

        let mut ids = worktree_session_ids(path, &stored, &sessions);
        ids.sort();

        assert_eq!(ids, ["archived", "in", "nested"]);
    }

    #[test]
    fn detach_keeps_the_worktree_and_drops_provider_state() {
        let mut linked = session("a", "/p", Some("/p-worktrees/mc-a"));
        linked.branch = Some("mc/a".into());
        linked.provider_session_id = Some("prov".into());
        linked.context_used = Some(10);
        detach_session(&mut linked, "/p-worktrees/mc-a", "/p");
        assert!(linked.worktree_removed);
        assert_eq!(linked.cwd, "/p");
        assert_eq!(linked.worktree_cwd.as_deref(), Some("/p-worktrees/mc-a"));
        assert_eq!(
            (
                linked.branch,
                linked.provider_session_id,
                linked.context_used
            ),
            (None, None, None)
        );

        let mut direct = session("b", "/p-worktrees/mc-a/src", None);
        detach_session(&mut direct, "/p-worktrees/mc-a", "/p");
        assert_eq!(direct.cwd, "/p");
        assert_eq!(
            direct.worktree_cwd.as_deref(),
            Some("/p-worktrees/mc-a/src")
        );
    }
}
