//! MonoCode `worktreeRemoved` (`WorktreePicker`): a thread whose worktree
//! was deleted shows "No branch" in place of its checkout and branch, its
//! prompt asks for a working copy, and nothing is sent until the thread is
//! moved to the project folder or another worktree.

use ely_gpui_component::buttons::ButtonVariant;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::IconName;
use gpui::{Context, IntoElement};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::app_callback::app_callback;

/// MonoCode's placeholder while the thread has no working copy.
pub const REMOVED_PLACEHOLDER: &str = "Select a branch or worktree to continue…";

impl BenCodeApp {
    /// The thread lost its worktree and waits for a new working copy.
    pub fn worktree_removed(&self, session_id: &str) -> bool {
        self.sessions
            .iter()
            .any(|s| s.id == session_id && s.worktree_removed)
    }

    /// Moves the thread to `worktree` (a worktree path) or, with `None`,
    /// to its project folder, and lets it run again.
    pub fn reattach_session(
        &mut self,
        session_id: &str,
        worktree: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (id, row, target) = (session_id.to_string(), session_id.to_string(), worktree.clone());
        self.db_then(
            cx,
            move |db| db.reattach_session(&row, target.as_deref()),
            move |this, moved, cx| {
                if let Err(err) = moved {
                    log::error!("could not move thread {id} to a working copy: {err:#}");
                    this.composer_error = Some(format!("Could not continue in that working copy: {err}"));
                    cx.notify();
                    return;
                }
                if let Some(session) = this.sessions.iter_mut().find(|s| s.id == id) {
                    session.worktree_removed = false;
                    session.worktree_cwd = worktree;
                }
                // A save queued while the move ran still named the old worktree.
                this.persist_session(&id);
                this.sync_prompt_placeholder(cx);
                this.refresh_workspace(cx);
                this.refocus_prompt(cx);
                cx.notify();
            },
        );
    }

    /// MonoCode `WorktreePicker`: "No branch", opening the project folder
    /// and the project's other worktrees.
    pub(super) fn removed_worktree_picker(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let main = {
            let id = session.id.clone();
            MenuItem::new("Project folder")
                .icon(IconName::Folder)
                .on_click(app_callback(cx, move |this, cx| {
                    this.reattach_session(&id, None, cx)
                }))
        };
        let menu =
            self.workspace
                .worktrees
                .iter()
                .filter(|tree| !tree.is_main && !tree.missing && !tree.prunable)
                .fold(Menu::new().item(main), |menu, tree| {
                    let label = tree.branch.clone().unwrap_or_else(|| {
                        std::path::Path::new(&tree.path)
                            .file_name()
                            .map_or_else(|| tree.path.clone(), |n| n.to_string_lossy().into_owned())
                    });
                    let (id, path) = (session.id.clone(), tree.path.clone());
                    menu.item(MenuItem::new(label).icon(IconName::GitBranch).on_click(
                        app_callback(cx, move |this, cx| {
                            this.reattach_session(&id, Some(path.clone()), cx)
                        }),
                    ))
                });
        DropdownMenu::new("composer-removed-worktree", "No branch", menu)
            .variant(ButtonVariant::Ghost)
            .icon(IconName::GitBranch)
    }
}
