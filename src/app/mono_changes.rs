//! The Overview's Changes tab on the app (MonoCode `MonoChangesView`,
//! `MonoProjectCommit`): one tab per project the Mono works on, each with
//! its working tree's changes, staged through the git index as its own
//! Changes panel stages them, and committed and pushed from there.
//!
//! MonoCode lists the files the Mono's session changed (its checkpoints);
//! a Mono's turns keep none in BenCode (they run in the home folder), so
//! the tab shows each project's whole working tree. The git work is
//! `crate::git`'s, run off the UI thread as `app/source_control.rs` runs it.

use std::collections::HashMap;

use ely_gpui_component::forms::TextInput;
use gpui::{AppContext as _, Context, Entity, Window};

use crate::app::BenCodeApp;
use crate::git::sync::SyncInfo;
use crate::git::{GitDetailedStatus, StatusPass};

/// A project's checkout, as last read.
#[derive(Clone, Debug, Default)]
pub struct ProjectChanges {
    /// The folder is a git repository.
    pub repo: bool,
    pub status: GitDetailedStatus,
    pub sync: SyncInfo,
}

impl ProjectChanges {
    /// MonoCode's tab count: each changed file once, staged or not.
    pub fn file_count(&self) -> usize {
        let mut paths: Vec<&str> = self
            .status
            .staged
            .iter()
            .chain(&self.status.unstaged)
            .map(|f| f.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
}

pub struct ChangesState {
    /// The project whose tab is open; its first while none is picked.
    pub root: Option<String>,
    pub projects: HashMap<String, ProjectChanges>,
    pub message: Entity<TextInput>,
    /// The project an action is running in, and what it is.
    pub busy: Option<(String, &'static str)>,
    pub error: Option<String>,
    pub status: Option<String>,
    generation: u64,
}

impl ChangesState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let message = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(2, 8)
                .placeholder("Message")
        });
        Self {
            root: None,
            projects: HashMap::new(),
            message,
            busy: None,
            error: None,
            status: None,
            generation: 0,
        }
    }
}

/// Whether the project can push: a remote, and not diverged.
pub fn can_push(sync: &SyncInfo) -> bool {
    sync.remote.is_some() && !(sync.ahead > 0 && sync.behind > 0)
}

/// MonoCode `canSync`: an upstream it is ahead of or behind.
pub fn can_sync(sync: &SyncInfo) -> bool {
    sync.remote.is_some() && sync.upstream.is_some() && (sync.ahead > 0 || sync.behind > 0)
}

/// MonoCode `canPublish`: a remote and no upstream yet.
pub fn can_publish(sync: &SyncInfo) -> bool {
    sync.remote.is_some() && sync.upstream.is_none() && sync.branch.is_some()
}

impl BenCodeApp {
    /// The project whose tab shows.
    pub fn mono_changes_root(&self) -> Option<String> {
        let mono = self.mono_in_view()?;
        let picked = self.monos.changes.root.as_ref().filter(|root| mono.works_on(root));
        picked.cloned().or_else(|| mono.projects.first().cloned())
    }

    /// The Changes tab's count: what is changed across its projects.
    pub fn mono_changes_count(&self) -> usize {
        let Some(mono) = self.mono_in_view() else {
            return 0;
        };
        mono.projects
            .iter()
            .filter_map(|root| self.monos.changes.projects.get(root))
            .map(ProjectChanges::file_count)
            .sum()
    }

    /// Reads every project's status and sync state off the UI thread.
    pub fn load_mono_changes(&mut self, cx: &mut Context<Self>) {
        let Some(projects) = self.mono_in_view().map(|mono| mono.projects.clone()) else {
            return;
        };
        self.monos.changes.generation += 1;
        let generation = self.monos.changes.generation;
        let task = cx.background_executor().spawn(async move {
            projects
                .into_iter()
                .map(|root| {
                    let changes = match StatusPass::read(&root) {
                        Some(pass) => ProjectChanges {
                            repo: true,
                            status: pass.detailed_status(&root),
                            sync: crate::git::sync::sync_info(&root),
                        },
                        None => ProjectChanges::default(),
                    };
                    (root, changes)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let read = task.await;
            let landed = this.update(cx, |this, cx| {
                if this.monos.changes.generation != generation {
                    return;
                }
                for (root, changes) in read {
                    this.monos.changes.projects.insert(root, changes);
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("Mono changes after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub fn select_mono_changes_project(&mut self, root: &str, cx: &mut Context<Self>) {
        self.monos.changes.root = Some(root.to_string());
        self.monos.changes.error = None;
        cx.notify();
    }

    /// Runs `work` in the open project off the UI thread, then reads the
    /// projects again; a failure shows under the tabs.
    fn run_mono_git(
        &mut self,
        label: &'static str,
        done: Option<&'static str>,
        work: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
        after: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.monos.changes.busy.is_some() {
            return;
        }
        let Some(root) = self.mono_changes_root() else {
            return;
        };
        self.monos.changes.busy = Some((root.clone(), label));
        self.monos.changes.error = None;
        self.monos.changes.status = None;
        let task = cx.background_executor().spawn({
            let root = root.clone();
            async move { work(&root) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                this.monos.changes.busy = None;
                match result {
                    Ok(()) => {
                        this.monos.changes.status = done.map(str::to_string);
                        after(this, cx);
                    }
                    Err(err) => {
                        log::warn!("Mono git action failed in {root}: {err}");
                        this.monos.changes.error = Some(err);
                    }
                }
                this.load_mono_changes(cx);
                // The project's own Changes panel shows the same index.
                this.refresh_workspace(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("Mono git action after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Stages (`stage`) or unstages one file.
    pub fn mono_stage_file(&mut self, path: String, stage: bool, cx: &mut Context<Self>) {
        self.run_mono_git(
            "file",
            None,
            move |root| {
                if stage {
                    crate::git::stage_file(root, &path)
                } else {
                    crate::git::unstage_file(root, &path)
                }
                .map_err(|err| format!("{err:#}"))
            },
            |_, _| {},
            cx,
        );
    }

    pub fn mono_stage_all(&mut self, stage: bool, cx: &mut Context<Self>) {
        self.run_mono_git(
            "all",
            None,
            move |root| {
                if stage {
                    crate::git::stage_all(root)
                } else {
                    crate::git::unstage_all(root)
                }
                .map_err(|err| format!("{err:#}"))
            },
            |_, _| {},
            cx,
        );
    }

    /// MonoCode `commit(push)`: the staged files committed with the
    /// message, then pushed when asked.
    pub fn mono_commit(&mut self, push: bool, cx: &mut Context<Self>) {
        let message = self.monos.changes.message.read(cx).text().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.run_mono_git(
            if push { "commit-push" } else { "commit" },
            Some(if push { "Committed and pushed" } else { "Committed" }),
            move |root| {
                crate::git::sync::commit(root, &message, false)
                    .map_err(|err| format!("Couldn't commit: {err}"))?;
                if push {
                    crate::git::sync::push(root)
                        .map_err(|err| format!("Committed, but couldn't push: {err}"))?;
                }
                Ok(())
            },
            |this, cx| {
                this.monos
                    .changes
                    .message
                    .update(cx, |input, cx| input.set_text("", cx));
            },
            cx,
        );
    }

    /// Push (or publish the branch), or sync with its upstream.
    pub fn mono_push(&mut self, sync: bool, cx: &mut Context<Self>) {
        self.run_mono_git(
            if sync { "sync" } else { "push" },
            Some(if sync { "Synced" } else { "Pushed" }),
            move |root| {
                if sync {
                    crate::git::sync::sync(root)
                } else {
                    crate::git::sync::push(root)
                }
            },
            |_, _| {},
            cx,
        );
    }

    /// A changed file's review, in the workspace (MonoCode opens it over
    /// the list; BenCode's review lives in the file pane).
    pub fn open_mono_change(
        &mut self,
        path: String,
        staged: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.mono_changes_root() else {
            return;
        };
        self.leave_mono_view(cx);
        self.sync_selection(cx);
        let side = if staged {
            crate::ui::git_changes_panel::Side::Staged
        } else {
            crate::ui::git_changes_panel::Side::Unstaged
        };
        self.open_pane_tab(
            crate::app::file_pane::PaneTab::Review {
                cwd: root,
                path,
                side,
            },
            true,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitFileChange, GitFileStatus};

    fn file(path: &str) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            status: GitFileStatus::Modified,
            additions: 1,
            deletions: 0,
        }
    }

    #[test]
    fn a_file_staged_in_part_counts_once() {
        let changes = ProjectChanges {
            repo: true,
            status: GitDetailedStatus {
                branch: "main".into(),
                staged: vec![file("a"), file("b")],
                unstaged: vec![file("a"), file("c")],
            },
            sync: SyncInfo::default(),
        };
        assert_eq!(changes.file_count(), 3);
    }

    #[test]
    fn push_and_sync_follow_the_branch() {
        let mut sync = SyncInfo {
            branch: Some("feat".into()),
            remote: Some("origin".into()),
            ..Default::default()
        };
        assert!(can_publish(&sync) && can_push(&sync) && !can_sync(&sync));
        sync.upstream = Some("origin/feat".into());
        sync.ahead = 1;
        assert!(can_sync(&sync) && !can_publish(&sync));
        sync.behind = 1;
        assert!(!can_push(&sync));
    }
}
