//! MonoCode's "New worktree" workspace mode (`WorkspacePicker`,
//! `worktrees.ts`): before its first message a thread can choose to run in
//! a fresh worktree branched from a base (⌘⇧G toggles it). Nothing is
//! created until the first send; then `git worktree add` makes
//! `<repo>-worktrees/mc-<token>` on branch `mc/<token>`, the thread moves
//! there, and the message is sent from it.

use std::hash::{BuildHasher, RandomState};

use gpui::Context;

use crate::app::{BenCodeApp, TurnInput};
use crate::ui::composer::ComposerPopover;
use crate::git::worktrees::create_worktree;

/// MonoCode `temporaryWorktreeBranchName`: `mc/` and eight lowercase
/// letters or digits.
pub fn temporary_branch_name(seed: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rest = seed;
    let token: String = (0..8)
        .map(|_| {
            let digit = DIGITS[(rest % 36) as usize] as char;
            rest /= 36;
            digit
        })
        .collect();
    format!("mc/{token}")
}

fn random_seed() -> u64 {
    RandomState::new().hash_one(crate::app::now_ms())
}

impl BenCodeApp {
    /// The thread the focused composer would send to; `""` before one exists.
    fn draft_key(&self) -> String {
        self.selected_session_id.clone().unwrap_or_default()
    }

    /// The base a new worktree would branch from, while that mode is chosen.
    pub fn new_worktree_base(&self) -> Option<&str> {
        self.new_worktrees
            .get(&self.draft_key())
            .map(String::as_str)
    }

    /// The branch new worktrees start from by default (MonoCode: the
    /// current branch, else `HEAD`).
    fn default_worktree_base(&self) -> String {
        Some(self.git_status.branch.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "HEAD".to_string())
    }

    /// The focused thread has not started, so its workspace can change.
    pub fn is_draft_workspace(&self) -> bool {
        let session = self.selected_session();
        let fresh = session.is_none_or(|s| {
            !s.blocks.iter().any(|b| b.role == "user")
                && s.worktree_cwd.as_deref().is_none_or(str::is_empty)
        });
        fresh
            && session.is_none_or(|s| !self.is_agent_running_in(&s.id))
            && !self.workspace.worktrees.is_empty()
    }

    /// MonoCode `onWorkspaceModeChange`: "New worktree" on or off.
    pub fn set_new_worktree(&mut self, on: bool, cx: &mut Context<Self>) {
        if !self.is_draft_workspace() {
            return;
        }
        let key = self.draft_key();
        if on {
            let base = self.default_worktree_base();
            self.new_worktrees.insert(key, base);
            // A new worktree branches off the project, not another tree.
            if self.worktree_focus().is_some() {
                self.select_workspace(None, cx);
            }
        } else {
            self.new_worktrees.remove(&key);
        }
        cx.notify();
    }

    /// ⌘⇧G in a new thread's composer (MonoCode "Composer: Toggle Workspace").
    pub fn toggle_new_worktree(&mut self, cx: &mut Context<Self>) {
        let on = self.new_worktree_base().is_none();
        self.set_new_worktree(on, cx);
        self.refocus_prompt(cx);
    }

    pub(super) fn set_worktree_base(&mut self, base: &str, cx: &mut Context<Self>) {
        let key = self.draft_key();
        if let Some(chosen) = self.new_worktrees.get_mut(&key) {
            *chosen = base.to_string();
        }
        self.close_popover(ComposerPopover::Base);
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// The choice made before the thread existed goes with the thread
    /// created for the first send.
    pub fn adopt_draft_workspace(&mut self, session_id: &str) {
        if let Some(base) = self.new_worktrees.remove("") {
            self.new_worktrees.insert(session_id.to_string(), base);
        }
    }

    /// The first send of a "New worktree" thread: the worktree is made off
    /// the UI thread, the thread moves into it, then `input` is sent. If git
    /// refuses, the message returns to the composer with git's reason and
    /// the mode stays chosen.
    pub fn send_in_new_worktree(
        &mut self,
        session_id: &str,
        input: TurnInput,
        typed: String,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = self.new_worktrees.remove(session_id) else {
            return;
        };
        let Some(project) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.cwd.clone())
        else {
            return;
        };
        let session_id = session_id.to_string();
        self.thread_mut(&session_id).preparing_worktree = true;
        let branch = temporary_branch_name(random_seed());
        let create = cx.background_executor().spawn({
            let base = base.clone();
            async move { create_worktree(&project, &branch, &base, false) }
        });
        cx.spawn(async move |this, cx| {
            let created = create.await;
            let _ = this.update(cx, |app, cx| {
                if let Some(thread) = app.threads.get_mut(&session_id) {
                    thread.preparing_worktree = false;
                }
                match created {
                    Ok(tree) => {
                        if let Some(session) = app.sessions.iter_mut().find(|s| s.id == session_id)
                        {
                            session.worktree_cwd = Some(tree.path.clone());
                            session.branch = tree.branch.clone();
                        }
                        app.persist_session(&session_id);
                        app.refresh_workspace(cx);
                        app.send_turn(&session_id, input, cx);
                    }
                    Err(err) => {
                        log::warn!("new worktree: {err:#}");
                        app.new_worktrees.insert(session_id.clone(), base);
                        if app.selected_session_id.as_deref() == Some(session_id.as_str())
                            && app.prompt_input.read(cx).text().is_empty()
                        {
                            app.prompt_input
                                .update(cx, |field, cx| field.set_text(typed, cx));
                            app.thread_mut(&session_id).attachments = input.attachments;
                        }
                        app.composer_error =
                            Some(format!("Could not create the worktree: {err:#}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_branches_are_mc_and_eight_characters() {
        assert_eq!(temporary_branch_name(0), "mc/00000000");
        let name = temporary_branch_name(u64::MAX);
        assert_eq!(name.len(), 11);
        assert!(name[3..].chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(temporary_branch_name(1), temporary_branch_name(2));
    }
}
