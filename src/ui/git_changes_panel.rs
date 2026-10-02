//! Git changes panel: staged and unstaged working tree changes, commit form,
//! sync status (ahead/behind), and recent commit history.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::Alert;
use ely_gpui_component::git::{ChangeAction, Changed, ChangesList, Commit, CommitItem, DiffStat};
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::lists::GitStatus;
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Severity};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::*,
    px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::git::{
    GitFileChange, GitFileStatus, commit, discard_all, discard_file, stage_all, stage_file,
    unstage_all, unstage_file,
};

pub fn to_ely_status(status: &GitFileStatus) -> GitStatus {
    match status {
        GitFileStatus::Modified => GitStatus::Modified,
        GitFileStatus::Added => GitStatus::Added,
        GitFileStatus::Deleted => GitStatus::Deleted,
        GitFileStatus::Untracked => GitStatus::Untracked,
        GitFileStatus::Renamed => GitStatus::Renamed,
    }
}

fn to_changed(files: &[GitFileChange]) -> Vec<Changed> {
    files
        .iter()
        .map(|f| Changed {
            path: f.path.clone().into(),
            status: to_ely_status(&f.status),
            added: f.additions,
            removed: f.deletions,
        })
        .collect()
}

impl BenCodeApp {
    pub fn render_git_changes_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let branch_name = self.git_status.branch.clone();
        let ahead = self.git_status.ahead;
        let behind = self.git_status.behind;
        let staged_count = self.git_status.staged.len();
        let history_collapsed = self.git_history_collapsed;

        let total_additions: usize = self
            .git_status
            .staged
            .iter()
            .map(|f| f.additions)
            .sum::<usize>()
            + self
                .git_status
                .unstaged
                .iter()
                .map(|f| f.additions)
                .sum::<usize>();
        let total_deletions: usize = self
            .git_status
            .staged
            .iter()
            .map(|f| f.deletions)
            .sum::<usize>()
            + self
                .git_status
                .unstaged
                .iter()
                .map(|f| f.deletions)
                .sum::<usize>();

        let staged_changed = to_changed(&self.git_status.staged);
        let unstaged_changed = to_changed(&self.git_status.unstaged);

        let (on_action, on_all) = (cx.entity().downgrade(), cx.entity().downgrade());
        let changes_list =
            ChangesList::new("git-worktree-changes", staged_changed, unstaged_changed)
                .on_action(move |path, action, _, cx| {
                    let path = path.to_string();
                    let _ =
                        on_action.update(cx, |this, cx| this.on_change_action(path, action, cx));
                })
                .on_all(move |stage, _, cx| {
                    let _ = on_all.update(cx, |this, cx| {
                        if stage {
                            this.run_git_action("Stage all", stage_all, cx);
                        } else {
                            this.run_git_action("Unstage all", unstage_all, cx);
                        }
                    });
                });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            // Last failed git action dismissible alert
            .when_some(self.workspace.git_error.clone(), |el, error| {
                el.child(div().p_2().child(
                    Alert::new("git-error-banner", Severity::Danger, "Git Error").body(error),
                ))
            })
            // 2. Branch & Sync Status Row
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.bg)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Xs)
                                    .color(colors.accent),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .text_color(colors.fg)
                                    .child(branch_name),
                            )
                            .when(ahead > 0 || behind > 0, |el| {
                                el.child(
                                    div()
                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                        .text_color(colors.fg_muted)
                                        .child(format!("↑{ahead} ↓{behind}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(total_additions > 0 || total_deletions > 0, |el| {
                                el.child(DiffStat::new(total_additions, total_deletions))
                            })
                            .child(
                                IconButton::new("git-sync-btn", IconName::RotateCw)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Refresh git status")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_git_status(cx);
                                    })),
                            ),
                    ),
            )
            // 3. Commit Box Area
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.bg)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .w_full()
                            .min_h(px(48.0))
                            .p_1p5()
                            .rounded(cx.theme().radius(Radius::Sm))
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.surface)
                            .text_size(px(12.0))
                            .child(self.git_commit_input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(
                                    "git-commit-btn",
                                    if staged_count > 0 {
                                        format!("Commit ({staged_count})")
                                    } else {
                                        "Commit".to_string()
                                    },
                                )
                                .variant(if staged_count > 0 {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .size(ControlSize::Sm)
                                .full_width()
                                .icon(IconName::Check)
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.commit_staged_changes(cx);
                                    },
                                )),
                            )
                            .child(
                                IconButton::new("git-discard-all-btn", IconName::Undo2)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Discard all unstaged changes")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.git_confirm = Some(GitConfirm::DiscardAll);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                IconButton::new("git-stage-all-btn", IconName::Plus)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Secondary)
                                    .tooltip("Stage all changes")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_git_action("Stage all", stage_all, cx);
                                    })),
                            ),
                    ),
            )
            // 4. Scrollable Changes Lists: Staged & Working Tree Changes via ChangesList + Commit History
            .child(
                on_axis(div().id("git-changes-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .py_1()
                    .child(changes_list)
                    // Section 3: Commit History
                    .child(self.render_history_section(history_collapsed, cx)),
            )
    }

    fn render_history_section(&self, collapsed: bool, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let commits = &self.git_commits;
        let count = commits.len();

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1p5()
                    .bg(colors.surface)
                    .hover(|s| s.bg(colors.hover))
                    .child(
                        div()
                            .id("toggle-history-collapsed-btn")
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .cursor_pointer()
                            .child(
                                Icon::new(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(IconSize::Xs)
                                .color(colors.fg_muted),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .text_color(colors.fg_muted)
                                    .child("RECENT COMMITS"),
                            )
                            .child(Badge::new(count.to_string()).tone(Tone::Neutral))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_history_collapsed = !this.git_history_collapsed;
                                cx.notify();
                            })),
                    ),
            )
            .when(!collapsed, |el| {
                if commits.is_empty() {
                    el.child(
                        div()
                            .px_6()
                            .py_2()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("No recent commits"),
                    )
                } else {
                    el.children(commits.iter().map(|c| {
                        CommitItem::new(
                            format!("commit-{}", c.hash),
                            Commit {
                                id: c.short_hash.clone().into(),
                                parents: vec![],
                                subject: c.message.clone().into(),
                                author: c.author.clone().into(),
                                when: c.relative_time.clone().into(),
                                refs: vec![],
                            },
                        )
                    }))
                }
            })
    }

    fn on_change_action(&mut self, path: String, action: ChangeAction, cx: &mut Context<Self>) {
        match action {
            ChangeAction::Open => {
                self.active_view_mode = ViewMode::Changes;
                self.select_diff_path(path, cx);
            }
            ChangeAction::Stage => {
                self.run_git_action("Stage", move |cwd| stage_file(cwd, &path), cx)
            }
            ChangeAction::Unstage => {
                self.run_git_action("Unstage", move |cwd| unstage_file(cwd, &path), cx)
            }
            ChangeAction::Discard => {
                self.git_confirm = Some(GitConfirm::DiscardFile(path));
                cx.notify();
            }
        }
    }

    pub fn commit_staged_changes(&mut self, cx: &mut Context<Self>) {
        let message = self.git_commit_input.read(cx).text().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.git_commit_input
            .update(cx, |input, cx| input.set_text("", cx));
        let input = self.git_commit_input.clone();
        let cwd = self.workspace_cwd();
        let restore = message.clone();
        let task = cx
            .background_executor()
            .spawn(async move { commit(&cwd, &message) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if let Err(err) = &result {
                    log::error!("git commit failed: {err:#}");
                    // Give the message back unless a new draft was started meanwhile.
                    input.update(cx, |input, cx| {
                        if input.text().trim().is_empty() {
                            input.set_text(restore, cx);
                        }
                    });
                }
                app.workspace.git_error = result.err().map(|err| format!("Commit failed: {err:#}"));
                app.refresh_workspace(cx);
            });
        })
        .detach();
    }

    /// The discard confirmation, when one is pending.
    pub fn render_git_confirm(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let pending = self.git_confirm.clone()?;
        let close = cx.listener(|this, _: &(), _, cx| {
            this.git_confirm = None;
            cx.notify();
        });
        let confirm = cx.listener(move |this, _: &(), _, cx| match &pending {
            GitConfirm::DiscardFile(path) => {
                let path = path.clone();
                this.run_git_action("Discard", move |cwd| discard_file(cwd, &path), cx);
            }
            GitConfirm::DiscardAll => this.run_git_action("Discard all", discard_all, cx),
        });
        let (title, message) = match self.git_confirm.as_ref()? {
            GitConfirm::DiscardFile(path) => (
                "Discard changes?",
                format!("Unstaged changes to {path} will be lost."),
            ),
            GitConfirm::DiscardAll => (
                "Discard all changes?",
                "Every unstaged change in the working tree will be lost.".to_string(),
            ),
        };
        Some(
            ConfirmDialog::new("git-discard", title, message, move |window, cx| {
                close(&(), window, cx)
            })
            .confirm("Discard")
            .destructive()
            .on_confirm(move |window, cx| confirm(&(), window, cx))
            .into_any_element(),
        )
    }
}

/// A destructive git action waiting for confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitConfirm {
    DiscardFile(String),
    DiscardAll,
}
