//! MonoCode Settings › Worktrees (`WorktreesPage.tsx`) and its
//! `DeleteWorktreeDialog.tsx`: the current project's linked worktrees with
//! their sessions and status, Reveal, and Delete behind one confirmation.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::Switch;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::settings::{SettingsRow, SettingsSection};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Hsla, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
    px, relative,
};

use crate::app::BenCodeApp;
use crate::app::worktree_lifecycle::{deletion_blocker, worktree_session_ids};
use crate::git::Worktree;
use crate::git::worktrees::default_worktrees_dir;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::icons::ExtraIcon;
use crate::ui::sidebar_popovers::pretty_path;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// MonoCode's status line: missing folder, unknown, dirty or clean.
fn status(tree: &Worktree) -> (&'static str, Tone) {
    if tree.missing {
        ("Missing folder", Tone::Neutral)
    } else {
        match tree.dirty {
            None => ("Status unavailable", Tone::Neutral),
            Some(true) => ("Uncommitted changes", Tone::Warning),
            Some(false) => ("Clean", Tone::Neutral),
        }
    }
}

impl BenCodeApp {
    pub(crate) fn render_settings_worktrees(&self, cx: &Context<Self>) -> impl IntoElement {
        let trees: Vec<&Worktree> = self
            .workspace
            .worktrees
            .iter()
            .filter(|t| !t.is_main)
            .collect();
        let main = self.workspace.worktrees.iter().find(|t| t.is_main);
        let mut section = SettingsSection::new("Worktrees")
            .description("Manage additional worktrees for each project.")
            .row(
                SettingsRow::new(self.rail_project_label(&self.current_cwd))
                    .description(
                        "Sessions can share a worktree. Deleting one keeps its sessions by \
                         default and discards uncommitted changes. Its branch and commits are kept.",
                    )
                    .control(
                        IconButton::new("worktrees-refresh", IconName::RefreshCw)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Refresh worktrees")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_workspace(cx))),
                    ),
            );
        if let Some(error) = &self.worktrees_page_error {
            section = section.row(
                SettingsRow::new("Could not delete the worktree")
                    .description(error.clone())
                    .control(Badge::new("Error").tone(Tone::Danger)),
            );
        }
        if main.is_none() {
            return section
                .row(SettingsRow::new(
                    "Open a git project to manage its worktrees.",
                ))
                .into_any_element();
        }
        if trees.is_empty() {
            section = section.row(
                EmptyState::new(
                    "worktrees-empty",
                    IconName::GitBranch,
                    "No additional worktrees",
                )
                .body("Create a worktree to work on another branch in a separate folder."),
            );
        }
        for (ix, tree) in trees.into_iter().enumerate() {
            let count = worktree_session_ids(&tree.path, &[], &self.sessions).len();
            let branch = match &tree.branch {
                Some(branch) => format!("Current branch: {branch}"),
                None => format!(
                    "Detached at {}",
                    tree.head.chars().take(7).collect::<String>()
                ),
            };
            let mut facts = vec![format!(
                "{} in this worktree",
                plural(count, "session", "sessions")
            )];
            if let Some(n) = tree.unpushed.filter(|n| *n > 0) {
                facts.push(plural(
                    n as usize,
                    "unpublished commit",
                    "unpublished commits",
                ));
            }
            if tree.locked {
                facts.push("Locked".into());
            }
            let (status, tone) = status(tree);
            let blocker = deletion_blocker(tree);
            let (reveal, remove) = (tree.path.clone(), tree.path.clone());
            section = section.row(
                SettingsRow::new(folder_name(&tree.path))
                    .description(format!(
                        "{}\n{branch}\n{}",
                        pretty_path(&tree.path),
                        facts.join(" · ")
                    ))
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Badge::new(status).tone(tone))
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("worktree-reveal-{ix}")),
                                    IconName::FolderOpen,
                                )
                                .variant(ButtonVariant::Ghost)
                                .disabled(tree.missing)
                                .tooltip("Reveal folder")
                                .on_click(move |_, _, _| crate::ui::rail::reveal_project(&reveal)),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("worktree-delete-{ix}")),
                                    IconName::Trash2,
                                )
                                .variant(ButtonVariant::Ghost)
                                .disabled(blocker.is_some())
                                .tooltip(blocker.unwrap_or("Delete worktree"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.open_worktree_deletion(&remove, cx),
                                )),
                            ),
                    ),
            );
        }
        if let Some(main) = main {
            let root = default_worktrees_dir(std::path::Path::new(&main.path));
            section = section.row(SettingsRow::new("New worktrees").description(format!(
                "New worktrees are created in {}.",
                pretty_path(&root.to_string_lossy())
            )));
        }
        section.into_any_element()
    }

    /// MonoCode `DeleteWorktreeDialog`.
    pub fn render_worktree_deletion(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let deletion = self.worktree_deletion.as_ref()?;
        let tree = &deletion.tree;
        let fg = cx.theme().colors.fg;
        let (red, amber, muted) = (
            cx.theme().colors.danger,
            cx.theme().colors.warning,
            fg.opacity(0.35),
        );
        let consequence = |icon: AnyElement, text: String| {
            div()
                .flex()
                .items_start()
                .gap(px(10.0))
                .child(div().mt(px(1.0)).child(icon))
                .child(div().flex_1().min_w_0().child(text))
        };
        let icon = |name: IconName, tone: Hsla| {
            Icon::new(name)
                .size(IconSize::Sm)
                .color(tone)
                .into_any_element()
        };
        let count = deletion.session_ids.len();
        let mut list = div()
            .flex()
            .flex_col()
            .gap_2()
            .mt(px(10.0))
            .pt(px(10.0))
            .border_t_1()
            .border_color(fg.opacity(0.08))
            .text_size(px(12.5))
            .text_color(fg.opacity(0.75));
        if count > 0 {
            let (verb, fate) = match (count == 1, deletion.delete_sessions) {
                (true, true) => ("is", "permanently deleted."),
                (false, true) => ("are", "permanently deleted."),
                (true, false) => ("is", "kept. Select a branch or worktree to continue them."),
                (false, false) => ("are", "kept. Select a branch or worktree to continue them."),
            };
            list = list.child(consequence(
                icon(
                    IconName::MessageSquare,
                    if deletion.delete_sessions { red } else { muted },
                ),
                format!(
                    "{} using this worktree {verb} {fate}",
                    plural(count, "session", "sessions")
                ),
            ));
        }
        match tree.dirty {
            Some(true) => {
                list = list.child(consequence(
                    ExtraIcon::FileDiff
                        .render(px(14.0), amber)
                        .into_any_element(),
                    "All uncommitted and untracked changes here are discarded.".into(),
                ))
            }
            None => {
                list = list.child(consequence(
                    icon(IconName::CircleAlert, amber),
                    "Changes could not be checked. Anything uncommitted here is discarded.".into(),
                ))
            }
            Some(false) => {}
        }
        list = list.child(consequence(
            icon(IconName::GitBranch, muted),
            match &tree.branch {
                Some(branch) => format!("The {branch} branch and its commits are kept."),
                None => "The branch is kept.".into(),
            },
        ));
        if let Some(n) = tree.unpushed.filter(|n| *n > 0) {
            list = list.child(consequence(
                icon(IconName::CloudUpload, muted),
                format!(
                    "{} not on a remote. They stay on the branch.",
                    if n == 1 {
                        "1 commit is".to_string()
                    } else {
                        format!("{n} commits are")
                    }
                ),
            ));
        }
        let card = div()
            .rounded(px(8.0))
            .border_1()
            .border_color(fg.opacity(0.10))
            .bg(fg.opacity(0.05))
            .p_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.55))
                    .child(Icon::new(IconName::Folder).size(IconSize::Sm).color(muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family("monospace")
                            .child(pretty_path(&tree.path)),
                    ),
            )
            .child(list);
        let busy = deletion.busy;
        let label = match (count, deletion.delete_sessions) {
            (0, _) | (_, false) => "Delete worktree".to_string(),
            (1, true) => "Delete worktree and session".into(),
            (_, true) => "Delete worktree and sessions".into(),
        };
        let close = app_callback(cx, |this, cx| this.close_worktree_deletion(cx));
        let confirm = app_callback(cx, |this, cx| this.confirm_worktree_deletion(cx));
        let toggle = app_callback_with(cx, |this, on: bool, cx| {
            this.set_delete_worktree_sessions(on, cx)
        });
        let mut dialog = Dialog::new("delete-worktree", "Delete worktree?", close)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .child(div().text_color(fg.opacity(0.75)).child(
                        "This permanently deletes the working copy and everything inside it.",
                    ))
                    .child(card)
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(fg.opacity(0.10))
                                .p_3()
                                .child(
                                    Switch::new(
                                        "delete-worktree-sessions",
                                        deletion.delete_sessions,
                                    )
                                    .label("Also delete associated sessions")
                                    .disabled(busy)
                                    .on_change(move |on, window, cx| toggle(on, window, cx)),
                                ),
                        )
                    })
                    .children(
                        deletion
                            .error
                            .clone()
                            .map(|error| div().text_size(px(12.5)).text_color(red).child(error)),
                    ),
            )
            .action(move |close| {
                Button::new("delete-worktree-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .disabled(busy)
                    .on_click(move |_, window, cx| close(window, cx))
            });
        dialog = dialog.action(move |_| {
            Button::new(
                "delete-worktree-confirm",
                if busy {
                    "Deleting…".to_string()
                } else {
                    label
                },
            )
            .variant(ButtonVariant::Danger)
            .disabled(busy)
            .on_click(move |_, window, cx| confirm(window, cx))
        });
        Some(dialog.into_any_element())
    }
}
