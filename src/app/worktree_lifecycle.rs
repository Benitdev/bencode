//! Settings › Worktrees, ported from MonoCode `WorktreesPage.tsx`,
//! `useProjectWorktrees.ts`, `CreateWorktreeDialog.tsx`, `worktrees.ts`,
//! `onRemoveWorktree` (`App.tsx`) and `worktrees.rs::remove_with_sessions`.
//!
//! The page lists one project's linked worktrees, chosen from the open,
//! recent and archived projects, and creates new ones. Deleting asks once
//! and is then always forced. Threads that used the worktree are deleted
//! (the dialog's switch) or detached through the database's removal
//! journal *before* git deletes the folder: a failed deletion restores
//! them, and a crash is settled on the next launch
//! (`db/worktree_removals.rs`). Detached threads wait for a new working
//! copy (`ui/composer/removed_worktree.rs`).

use std::collections::HashMap;

use anyhow::Result;
use gpui::Context;

use crate::app::file_pane::PaneTab;
use crate::app::{BenCodeApp, is_path_in_project, normalize_project_path, same_project_path};
use crate::db::{MonoCodeDb, SessionRow};
use crate::git::Worktree;
use crate::git::worktrees::{create_worktree, list_worktrees, remove_worktree};
use crate::git::{Branch, list_branches};

/// The page's project and what was last loaded for it. A failed reload
/// keeps the last list (MonoCode `useProjectWorktrees`).
#[derive(Default)]
pub struct WorktreesPage {
    pub project: String,
    /// `None` until the first load lands.
    pub trees: Option<Vec<Worktree>>,
    /// Threads stored in each worktree, by worktree path.
    pub stored_sessions: HashMap<String, Vec<String>>,
    pub branches: Vec<Branch>,
    pub load_error: Option<String>,
    /// The last failed deletion, shown above the list.
    pub error: Option<String>,
    /// A failed deletion invalidated the list; Delete waits for a reload.
    pub refreshing_after_failure: bool,
    generation: u64,
}

impl WorktreesPage {
    pub fn main(&self) -> Option<&Worktree> {
        self.trees.as_ref()?.iter().find(|t| t.is_main)
    }

    pub fn linked(&self) -> impl Iterator<Item = &Worktree> {
        self.trees.iter().flatten().filter(|t| !t.is_main)
    }

    fn stored(&self, path: &str) -> &[String] {
        self.stored_sessions.get(path).map_or(&[], Vec::as_slice)
    }
}

/// The open "Create worktree" dialog. The new branch's name lives in
/// `BenCodeApp::worktree_branch_input`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorktreeCreation {
    /// "Use an existing local branch" rather than "Create a new branch".
    pub existing: bool,
    pub existing_branch: String,
    /// "Start from": `HEAD` or a branch.
    pub base: String,
    pub busy: bool,
    pub error: Option<String>,
}

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

/// MonoCode `WorktreesPage` `projects`: the open project, then recent
/// ones, then archived ones, each once.
pub fn project_choices(current: &str, recents: &[String], archived: &[String]) -> Vec<String> {
    let mut choices: Vec<String> = Vec::new();
    for path in std::iter::once(current)
        .chain(recents.iter().map(String::as_str))
        .chain(archived.iter().map(String::as_str))
    {
        let path = normalize_project_path(path);
        if path.is_empty() || path == "~" || choices.iter().any(|c| same_project_path(c, &path)) {
            continue;
        }
        choices.push(path);
    }
    choices
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
/// `MonoCodeDb::prepare_worktree_removal`.
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

/// Puts back what `detach_session` changed.
fn restore_session(session: &mut SessionRow, before: &SessionRow) {
    session.cwd = before.cwd.clone();
    session.worktree_cwd = before.worktree_cwd.clone();
    session.worktree_removed = before.worktree_removed;
    session.branch = before.branch.clone();
    session.provider_session_id = before.provider_session_id.clone();
    session.context_used = before.context_used;
    session.context_window = before.context_window;
}

type PageLoad = (
    Result<(Vec<Worktree>, HashMap<String, Vec<String>>)>,
    Vec<Branch>,
);

/// The page's data, off the UI thread: the working copies, the threads
/// stored in each (through a read-only connection) and the branches.
fn load_page(project: &str, db_file: Option<std::path::PathBuf>) -> PageLoad {
    let trees = list_worktrees(project).and_then(|trees| {
        let mut stored = HashMap::new();
        if let Some(file) = db_file {
            let reader = MonoCodeDb::open_reader(&file)?;
            for tree in trees.iter().filter(|t| !t.is_main) {
                stored.insert(
                    tree.path.clone(),
                    reader.session_ids_in_worktree(&tree.path)?,
                );
            }
        }
        Ok((trees, stored))
    });
    (trees, list_branches(project))
}

impl BenCodeApp {
    /// The page's project picker: open, recent and archived projects.
    pub fn worktree_project_choices(&self) -> Vec<String> {
        let archived: Vec<String> = self
            .settings
            .rail
            .archived_projects
            .iter()
            .map(|p| p.path.clone())
            .collect();
        project_choices(&self.current_cwd, &self.recent_projects, &archived)
    }

    /// Settings › Worktrees opened: starts on the open project.
    pub fn open_worktrees_page(&mut self, cx: &mut Context<Self>) {
        let project = self
            .worktree_project_choices()
            .into_iter()
            .next()
            .unwrap_or_default();
        self.select_worktrees_project(&project, cx);
    }

    /// MonoCode `onSelectProject`.
    pub fn select_worktrees_project(&mut self, project: &str, cx: &mut Context<Self>) {
        let page = &mut self.worktrees_page;
        if !same_project_path(&page.project, project) {
            page.trees = None;
            page.stored_sessions.clear();
            page.branches.clear();
            page.load_error = None;
        }
        page.project = project.to_string();
        page.error = None;
        self.worktree_deletion = None;
        self.load_worktrees_page(cx);
    }

    /// MonoCode `refresh`: reloads the page's project; a superseded load
    /// is dropped.
    pub fn load_worktrees_page(&mut self, cx: &mut Context<Self>) {
        let page = &mut self.worktrees_page;
        page.generation += 1;
        let generation = page.generation;
        let project = page.project.clone();
        cx.notify();
        if project.is_empty() {
            return;
        }
        let db_file = self.db.file_path();
        let task = cx
            .background_executor()
            .spawn(async move { load_page(&project, db_file) });
        cx.spawn(async move |this, cx| {
            let (trees, branches) = task.await;
            let landed = this.update(cx, |this, cx| {
                let page = &mut this.worktrees_page;
                if page.generation != generation {
                    return;
                }
                page.refreshing_after_failure = false;
                match trees {
                    Ok((trees, stored)) => {
                        page.trees = Some(trees);
                        page.stored_sessions = stored;
                        page.branches = branches;
                        page.load_error = None;
                    }
                    Err(err) => {
                        log::warn!("worktrees of {}: {err:#}", page.project);
                        page.load_error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("worktrees page load after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Threads in the worktree at `path`, as the page counts them.
    pub fn worktree_session_count(&self, path: &str) -> usize {
        worktree_session_ids(path, self.worktrees_page.stored(path), &self.sessions).len()
    }

    // ---- Create ----

    pub fn open_worktree_creation(&mut self, cx: &mut Context<Self>) {
        if self.worktrees_page.main().is_none() {
            return;
        }
        self.worktree_branch_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.worktree_creation = Some(WorktreeCreation {
            base: "HEAD".into(),
            ..Default::default()
        });
        cx.notify();
    }

    pub fn close_worktree_creation(&mut self, cx: &mut Context<Self>) {
        if self.worktree_creation.as_ref().is_some_and(|c| c.busy) {
            return;
        }
        self.worktree_creation = None;
        cx.notify();
    }

    /// "Create a new branch" / "Use an existing local branch"; either
    /// clears the name.
    pub fn set_worktree_creation_existing(&mut self, existing: bool, cx: &mut Context<Self>) {
        let Some(creation) = self.worktree_creation.as_mut() else {
            return;
        };
        creation.existing = existing;
        creation.existing_branch.clear();
        self.worktree_branch_input
            .update(cx, |input, cx| input.set_text("", cx));
        cx.notify();
    }

    pub fn set_worktree_creation_branch(&mut self, branch: &str, cx: &mut Context<Self>) {
        if let Some(creation) = self.worktree_creation.as_mut() {
            creation.existing_branch = branch.to_string();
            cx.notify();
        }
    }

    pub fn set_worktree_creation_base(&mut self, base: &str, cx: &mut Context<Self>) {
        if let Some(creation) = self.worktree_creation.as_mut() {
            creation.base = base.to_string();
            cx.notify();
        }
    }

    /// The branch the dialog would create or check out.
    pub fn worktree_creation_name(&self, cx: &gpui::App) -> String {
        match &self.worktree_creation {
            Some(c) if c.existing => c.existing_branch.trim().to_string(),
            Some(_) => self
                .worktree_branch_input
                .read(cx)
                .text()
                .trim()
                .to_string(),
            None => String::new(),
        }
    }

    /// The dialog's Create worktree: `git worktree add` from the project.
    pub fn confirm_worktree_creation(&mut self, cx: &mut Context<Self>) {
        let name = self.worktree_creation_name(cx);
        let project = self.worktrees_page.project.clone();
        let Some(creation) = self.worktree_creation.as_mut().filter(|c| !c.busy) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        creation.busy = true;
        creation.error = None;
        let (base, existing) = (creation.base.clone(), creation.existing);
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { create_worktree(&project, &name, &base, existing) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.worktree_creation = None;
                        this.load_worktrees_page(cx);
                        this.refresh_workspace(cx);
                    }
                    Err(err) => {
                        log::warn!("create worktree: {err:#}");
                        if let Some(creation) = this.worktree_creation.as_mut() {
                            creation.busy = false;
                            creation.error = Some(format!("{err:#}"));
                        }
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("worktree creation after app drop: {err:#}");
            }
        })
        .detach();
    }

    // ---- Delete ----

    /// The page's trash button.
    pub fn open_worktree_deletion(&mut self, path: &str, cx: &mut Context<Self>) {
        let page = &self.worktrees_page;
        if page.refreshing_after_failure || page.load_error.is_some() {
            return;
        }
        let Some(tree) = page
            .linked()
            .find(|t| same_project_path(&t.path, path))
            .cloned()
        else {
            return;
        };
        if deletion_blocker(&tree).is_some() {
            return;
        }
        let session_ids = worktree_session_ids(&tree.path, page.stored(&tree.path), &self.sessions);
        self.worktrees_page.error = None;
        self.worktree_deletion = Some(WorktreeDeletion {
            session_ids,
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

    /// A deletion that did not finish: the dialog closes, the error shows
    /// on the page, and Delete waits for a fresh list (MonoCode
    /// `setRefreshingAfterFailure`).
    fn abandon_worktree_deletion(&mut self, message: String, cx: &mut Context<Self>) {
        self.worktree_deletion = None;
        self.worktrees_page.error = Some(message);
        self.worktrees_page.refreshing_after_failure = true;
        self.load_worktrees_page(cx);
        self.refresh_workspace(cx);
    }

    /// The dialog's Delete: the sessions (if asked), the journal entry
    /// that detaches the kept threads, then `git worktree remove --force`.
    pub fn confirm_worktree_deletion(&mut self, cx: &mut Context<Self>) {
        let Some(deletion) = self.worktree_deletion.clone().filter(|d| !d.busy) else {
            return;
        };
        let path = deletion.tree.path.clone();
        let project = self.worktrees_page.project.clone();
        let Some(main) = self.worktrees_page.main().map(|t| t.path.clone()) else {
            return;
        };
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
            if self.sessions.iter().any(|s| ids.contains(&s.id))
                || self
                    .db
                    .session_ids_in_worktree(&path)
                    .map_or(true, |left| !left.is_empty())
            {
                return self.abandon_worktree_deletion(
                    "Some sessions could not be deleted, so the worktree was kept.".into(),
                    cx,
                );
            }
        }
        let kept = if sessions_deleted { Vec::new() } else { ids };
        // Detach before git runs; the journal restores them on failure.
        let journal = match self.db.prepare_worktree_removal(&path, &main, &kept) {
            Ok(saved) => saved,
            Err(err) => return self.fail_worktree_deletion(format!("{err:#}"), cx),
        };
        let before: Vec<SessionRow> = self
            .sessions
            .iter()
            .filter(|s| kept.contains(&s.id))
            .cloned()
            .collect();
        for session in self.sessions.iter_mut().filter(|s| kept.contains(&s.id)) {
            detach_session(session, &path, &main);
        }
        if let Some(d) = self.worktree_deletion.as_mut() {
            d.busy = true;
            d.error = None;
        }
        cx.notify();
        let task = cx.background_executor().spawn({
            let (project, path) = (project.clone(), path.clone());
            async move { remove_worktree(&project, &path, true) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    // The threads are already durably detached; a leftover
                    // journal entry is settled on the next launch.
                    if let Err(err) = this.db.finish_worktree_removal(&path, &[]) {
                        log::error!("worktree {path} deleted; journal cleanup retries on restart: {err:#}");
                    }
                    this.worktree_deletion = None;
                    this.worktree_deleted(&path, &main, cx);
                }
                Err(err) => {
                    log::warn!("delete worktree {path}: {err:#}");
                    let mut message = format!("{err:#}");
                    match this.db.finish_worktree_removal(&path, &journal) {
                        Ok(()) => {
                            for before in &before {
                                if let Some(s) = this.sessions.iter_mut().find(|s| s.id == before.id) {
                                    restore_session(s, before);
                                }
                            }
                        }
                        Err(restore) => {
                            log::error!("could not restore the threads of {path}: {restore:#}");
                            message = format!(
                                "{message}. Sessions remain detached until recovery on restart: {restore:#}"
                            );
                        }
                    }
                    if sessions_deleted {
                        message = format!("The sessions were deleted, but the worktree was kept. {message}");
                    }
                    this.abandon_worktree_deletion(message, cx);
                    this.sync_prompt_placeholder(cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("worktree deletion after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Git removed the folder (MonoCode `onDeleted`): the page and the
    /// open project move to the main checkout if they were inside it.
    fn worktree_deleted(&mut self, path: &str, main: &str, cx: &mut Context<Self>) {
        if is_path_in_project(&self.worktrees_page.project, path) {
            self.worktrees_page.project = main.to_string();
        }
        if self
            .worktree_focus()
            .is_some_and(|f| is_path_in_project(&f.path, path))
        {
            self.select_workspace(None, cx);
        }
        if is_path_in_project(&self.current_cwd, path) {
            self.switch_project(main.to_string(), cx);
        }
        self.sync_prompt_placeholder(cx);
        self.load_worktrees_page(cx);
        self.refresh_workspace(cx);
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
    fn project_choices_put_the_open_project_first_once() {
        let choices = project_choices(
            "/a/",
            &["/a".into(), "/b".into(), "~".into()],
            &["/b".into(), "/c".into()],
        );
        assert_eq!(choices, ["/a", "/b", "/c"]);
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
    fn detach_and_restore_round_trip() {
        let mut linked = session("a", "/p", Some("/p-worktrees/mc-a"));
        linked.branch = Some("mc/a".into());
        linked.provider_session_id = Some("prov".into());
        linked.context_used = Some(10);
        let before = linked.clone();
        detach_session(&mut linked, "/p-worktrees/mc-a", "/p");
        assert!(linked.worktree_removed);
        assert_eq!(linked.cwd, "/p");
        assert_eq!(linked.worktree_cwd.as_deref(), Some("/p-worktrees/mc-a"));
        assert_eq!(
            (
                &linked.branch,
                &linked.provider_session_id,
                linked.context_used
            ),
            (&None, &None, None)
        );
        restore_session(&mut linked, &before);
        assert_eq!(linked.branch.as_deref(), Some("mc/a"));
        assert!(!linked.worktree_removed);

        let mut direct = session("b", "/p-worktrees/mc-a/src", None);
        detach_session(&mut direct, "/p-worktrees/mc-a", "/p");
        assert_eq!(direct.cwd, "/p");
        assert_eq!(
            direct.worktree_cwd.as_deref(),
            Some("/p-worktrees/mc-a/src")
        );
    }
}
