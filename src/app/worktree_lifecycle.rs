//! MonoCode `worktree_lifecycle.rs`: removing a linked worktree and pruning
//! stale ones from the sidebar's worktree switcher. A removal asks first;
//! a worktree with uncommitted changes or unpushed commits needs a second,
//! explicit "Remove anyway". Threads that ran in a removed worktree are
//! marked `worktree_removed` and wait for a new working copy
//! (`ui/composer/removed_worktree.rs`).

use gpui::Context;

use crate::app::{BenCodeApp, same_project_path};
use crate::git::Worktree;
use crate::git::worktrees::{prune_worktrees, remove_worktree};

/// The open "Remove worktree?" confirmation.
#[derive(Clone, Debug, PartialEq)]
pub struct WorktreeRemoval {
    pub path: String,
    /// The branch, or "detached" with the short HEAD.
    pub label: String,
    /// Why a plain removal would lose work. Confirming then forces it.
    pub warning: Option<String>,
}

impl WorktreeRemoval {
    pub fn force(&self) -> bool {
        self.warning.is_some()
    }
}

/// What a plain `git worktree remove` would refuse for `tree`, from the
/// last snapshot. `remove_worktree` checks again before it runs.
pub fn removal_warning(tree: &Worktree) -> Option<String> {
    let mut reasons = Vec::new();
    if tree.dirty == Some(true) {
        reasons.push("uncommitted or untracked changes".to_string());
    }
    match tree.unpushed {
        Some(0) | None => {}
        Some(1) => reasons.push("1 unpushed commit".to_string()),
        Some(n) => reasons.push(format!("{n} unpushed commits")),
    }
    if tree.branch.is_none() {
        reasons.push("a detached HEAD".to_string());
    }
    (!reasons.is_empty()).then(|| format!("This worktree has {}.", reasons.join(" and ")))
}

/// The switcher's name for a worktree.
pub fn worktree_label(tree: &Worktree) -> String {
    tree.branch
        .clone()
        .unwrap_or_else(|| format!("Detached {}", tree.head.chars().take(7).collect::<String>()))
}

/// Linked worktrees git would drop on prune: their folder is gone.
pub fn has_stale_worktrees(trees: &[Worktree]) -> bool {
    trees
        .iter()
        .any(|t| !t.is_main && (t.prunable || t.missing))
}

impl BenCodeApp {
    /// The "…" on a switcher row: asks before removing that worktree.
    pub fn request_worktree_removal(&mut self, path: &str, cx: &mut Context<Self>) {
        self.close_sidebar_menu(cx);
        let Some(tree) = self
            .workspace
            .worktrees
            .iter()
            .find(|t| !t.is_main && same_project_path(&t.path, path))
        else {
            return;
        };
        self.worktree_removal = Some(WorktreeRemoval {
            path: tree.path.clone(),
            label: worktree_label(tree),
            warning: removal_warning(tree),
        });
        cx.notify();
    }

    /// Threads whose working copy is the worktree at `path`.
    fn sessions_in_worktree(&self, path: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|s| {
                s.worktree_cwd
                    .as_deref()
                    .is_some_and(|w| !w.is_empty() && same_project_path(w, path))
            })
            .map(|s| s.id.clone())
            .collect()
    }

    /// Confirmed: `git worktree remove` (forced when the dialog warned). A
    /// refusal reopens the dialog with git's reason and the force option.
    pub fn confirm_worktree_removal(&mut self, removal: WorktreeRemoval, cx: &mut Context<Self>) {
        self.worktree_removal = None;
        let affected = self.sessions_in_worktree(&removal.path);
        if affected.iter().any(|id| self.is_agent_running_in(id)) {
            self.workspace.git_error =
                Some("Stop the session running in this worktree before removing it.".into());
            cx.notify();
            return;
        }
        let project = self.current_cwd.clone();
        let (path, force) = (removal.path.clone(), removal.force());
        let task = cx
            .background_executor()
            .spawn(async move { remove_worktree(&project, &path, force) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| match result {
                Ok(()) => this.worktree_removed_at(&removal.path, &affected, cx),
                Err(err) => {
                    log::warn!("remove worktree {}: {err:#}", removal.path);
                    if removal.force() {
                        this.workspace.git_error =
                            Some(format!("Could not remove the worktree: {err:#}"));
                    } else {
                        this.worktree_removal = Some(WorktreeRemoval {
                            warning: Some(format!("{err:#}")),
                            ..removal
                        });
                    }
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("worktree removal after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The worktree is gone: its threads lose their working copy and the
    /// switcher falls back to the project folder if it was focused.
    fn worktree_removed_at(&mut self, path: &str, affected: &[String], cx: &mut Context<Self>) {
        if let Err(err) = self.db.mark_worktree_removed(affected) {
            log::error!("could not mark threads of removed worktree {path}: {err:#}");
        }
        for session in self
            .sessions
            .iter_mut()
            .filter(|s| affected.contains(&s.id))
        {
            session.worktree_removed = true;
        }
        let focused = self
            .worktree_focus()
            .is_some_and(|f| same_project_path(&f.path, path));
        if focused {
            self.select_workspace(None, cx);
        }
        self.sync_prompt_placeholder(cx);
        self.refresh_workspace(cx);
        cx.notify();
    }

    /// `git worktree prune`: drops worktrees whose folder was deleted.
    pub fn prune_stale_worktrees(&mut self, cx: &mut Context<Self>) {
        self.close_sidebar_menu(cx);
        let project = self.current_cwd.clone();
        let stale: Vec<String> = self
            .workspace
            .worktrees
            .iter()
            .filter(|t| !t.is_main && (t.prunable || t.missing))
            .map(|t| t.path.clone())
            .collect();
        let task = cx
            .background_executor()
            .spawn(async move { prune_worktrees(&project) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        for path in &stale {
                            let affected = this.sessions_in_worktree(path);
                            this.worktree_removed_at(path, &affected, cx);
                        }
                    }
                    Err(err) => {
                        log::warn!("prune worktrees: {err:#}");
                        this.workspace.git_error =
                            Some(format!("Could not prune worktrees: {err:#}"));
                    }
                }
                this.refresh_workspace(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("worktree prune after app drop: {err:#}");
            }
        })
        .detach();
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
            is_main: false,
            locked: false,
            prunable: false,
            missing: false,
            dirty: Some(false),
            unpushed: None,
            status_error: None,
        }
    }

    #[test]
    fn clean_worktree_has_no_warning() {
        assert_eq!(removal_warning(&tree()), None);
        assert_eq!(
            removal_warning(&Worktree {
                unpushed: Some(0),
                ..tree()
            }),
            None
        );
    }

    #[test]
    fn warning_names_every_reason() {
        let dirty = Worktree {
            dirty: Some(true),
            unpushed: Some(2),
            ..tree()
        };
        assert_eq!(
            removal_warning(&dirty).as_deref(),
            Some("This worktree has uncommitted or untracked changes and 2 unpushed commits.")
        );
        let one = Worktree {
            unpushed: Some(1),
            ..tree()
        };
        assert_eq!(
            removal_warning(&one).as_deref(),
            Some("This worktree has 1 unpushed commit.")
        );
        let detached = Worktree {
            branch: None,
            ..tree()
        };
        assert_eq!(
            removal_warning(&detached).as_deref(),
            Some("This worktree has a detached HEAD.")
        );
    }

    #[test]
    fn label_falls_back_to_short_head() {
        assert_eq!(worktree_label(&tree()), "mc/a");
        assert_eq!(
            worktree_label(&Worktree {
                branch: None,
                ..tree()
            }),
            "Detached 0123456"
        );
    }

    #[test]
    fn stale_ignores_the_main_checkout() {
        let main = Worktree {
            is_main: true,
            missing: true,
            ..tree()
        };
        assert!(!has_stale_worktrees(&[main.clone(), tree()]));
        assert!(has_stale_worktrees(&[
            main,
            Worktree {
                prunable: true,
                ..tree()
            }
        ]));
    }
}
