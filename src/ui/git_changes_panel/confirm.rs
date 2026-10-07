//! MonoCode `confirmNative` prompts for destructive git actions.

use super::*;

/// A destructive or risky git action waiting for confirmation (MonoCode
/// `confirmNative`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitConfirm {
    DiscardFile { path: String, untracked: bool },
    DiscardAll,
    /// Push (or open a PR) from the default branch.
    PushDefault(PendingCommit),
    /// Amend a commit that is already on a remote.
    AmendPushed(PendingCommit),
    CreatePrDefault,
}

impl BenCodeApp {
    /// The pending confirmation (MonoCode `confirmNative` messages).
    pub fn render_git_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let pending = self.git_confirm.clone()?;
        let (title, message, confirm, destructive) = match &pending {
            GitConfirm::DiscardFile { path, untracked: true } => (
                "Delete untracked file?",
                format!("Delete untracked file {}?", basename(path)),
                "Delete",
                true,
            ),
            GitConfirm::DiscardFile { path, .. } => (
                "Discard changes?",
                format!("Discard changes in {}? This cannot be undone.", basename(path)),
                "Discard",
                true,
            ),
            GitConfirm::DiscardAll => {
                let files = &self.git_status.unstaged;
                match files.as_slice() {
                    [only] if only.status == GitFileStatus::Untracked => (
                        "Delete untracked file?",
                        format!("Delete untracked file {}?", basename(&only.path)),
                        "Delete",
                        true,
                    ),
                    [only] => (
                        "Discard changes?",
                        format!(
                            "Discard changes in {}? This cannot be undone.",
                            basename(&only.path)
                        ),
                        "Discard",
                        true,
                    ),
                    files => (
                        "Discard all changes?",
                        format!(
                            "Discard all unstaged changes in {} files? This cannot be undone.",
                            files.len()
                        ),
                        "Discard",
                        true,
                    ),
                }
            }
            GitConfirm::PushDefault(p) => (
                "Push to the default branch?",
                format!(
                    "{} default branch \"{}\"?",
                    if p.pr {
                        "Create a pull request from"
                    } else {
                        "Push to"
                    },
                    self.git_sync.branch.clone().unwrap_or_default()
                ),
                if p.pr { "Create PR" } else { "Push" },
                false,
            ),
            GitConfirm::CreatePrDefault => (
                "Create a pull request?",
                format!(
                    "Create a pull request from default branch \"{}\"?",
                    self.git_sync.branch.clone().unwrap_or_default()
                ),
                "Create PR",
                false,
            ),
            GitConfirm::AmendPushed(_) => (
                "Amend a pushed commit?",
                "Amend a commit that is already pushed? BenCode cannot push the result. You will need a force push from the terminal.".to_string(),
                "Amend",
                true,
            ),
        };
        let close = app_callback(cx, |this, cx| {
            this.git_confirm = None;
            cx.notify();
        });
        let run = app_callback(cx, move |this, cx| {
            this.git_confirm = None;
            match &pending {
                GitConfirm::DiscardFile { path, .. } => {
                    let path = path.clone();
                    this.run_changes_action(
                        Busy::File(path.clone()),
                        None,
                        move |cwd| discard_file(cwd, &path).map_err(|e| format!("{e:#}")),
                        |_, _| {},
                        cx,
                    );
                }
                GitConfirm::DiscardAll => this.run_changes_action(
                    Busy::All,
                    None,
                    |cwd| discard_all(cwd).map_err(|e| format!("{e:#}")),
                    |_, _| {},
                    cx,
                ),
                GitConfirm::PushDefault(p) => this.commit_from_panel(*p, true, false, cx),
                GitConfirm::AmendPushed(p) => this.commit_from_panel(*p, true, true, cx),
                GitConfirm::CreatePrDefault => this.create_pr(true, cx),
            }
        });
        let dialog = ConfirmDialog::new("git-confirm", title, message, close)
            .confirm(confirm)
            .on_confirm(run);
        Some(if destructive { dialog.destructive() } else { dialog }.into_any_element())
    }
}
