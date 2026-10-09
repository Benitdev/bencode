//! MonoCode Settings › Worktrees (`WorktreesPage.tsx`) with its
//! `CreateWorktreeDialog.tsx` and `DeleteWorktreeDialog.tsx`: a project's
//! linked worktrees with their sessions and status, Reveal, Create, and
//! Delete behind one confirmation. State and logic: `app/worktree_lifecycle.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::forms::{Choice, Select, Switch};
use ely_gpui_component::motion::Spinner;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Hsla, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, relative,
};

use crate::app::worktree_lifecycle::deletion_blocker;
use crate::app::{BenCodeApp, same_project_path};
use crate::git::Worktree;
use crate::git::worktrees::default_worktrees_dir;
use crate::ui::app_callback::{app_callback, app_callback_with, on_value};
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow, icon_tile};
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
    /// MonoCode `WorktreesPage`: the project and where its worktrees go,
    /// then its linked worktrees.
    pub(crate) fn render_settings_worktrees(&self, cx: &Context<Self>) -> SettingsPage {
        let page = &self.worktrees_page;
        let fg = cx.theme().colors.fg;
        let project_choices = self.worktree_project_choices().into_iter().map(|path| {
            Choice::new(path.clone(), self.rail_project_label(&path)).note(pretty_path(&path))
        });
        let picker = div().w(px(240.0)).child(
            Select::new("worktrees-project", project_choices)
                .placeholder("Choose a project…")
                .selected(page.project.clone())
                .on_change(on_value(cx, |this, path, cx| this.select_worktrees_project(path, cx))),
        );
        let project = SettingsGroup::new("Project").row(
            SettingsRow::new("Project")
                .description("Whose worktrees this page manages.")
                .control(picker),
        );
        let project = match page.main() {
            Some(main) => {
                let root = default_worktrees_dir(std::path::Path::new(&main.path));
                project.row(
                    SettingsRow::new("Location")
                        .description(format!("New worktrees are created in {}.", pretty_path(&root.to_string_lossy()))),
                )
            }
            None => project,
        };

        let refresh_tip = match &page.load_error {
            Some(error) => format!("Refresh failed: {error}. Click to retry."),
            None => "Refresh worktrees".into(),
        };
        let actions = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                IconButton::new("worktrees-refresh", IconName::RefreshCw)
                    .variant(if page.load_error.is_some() { ButtonVariant::Danger } else { ButtonVariant::Ghost })
                    .size(ControlSize::Sm)
                    .disabled(page.project.is_empty())
                    .tooltip(refresh_tip)
                    .on_click(cx.listener(|this, _, _, cx| this.load_worktrees_page(cx))),
            )
            .child(
                Button::new("worktrees-create", "Create worktree")
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .icon(IconName::Plus)
                    .disabled(page.main().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.open_worktree_creation(cx))),
            );
        let mut list = SettingsGroup::new("Worktrees")
            .description(
                "Sessions can share a worktree. Deleting one keeps its sessions by default and \
                 discards uncommitted changes. Its branch and commits are kept.",
            )
            .action(actions);
        if let Some(error) = &page.error {
            list = list.row(SettingsRow::new("Something went wrong").error(error.clone()));
        }
        let trees = match (&page.trees, &page.load_error) {
            _ if page.project.is_empty() => {
                return SettingsTab::Worktrees
                    .page()
                    .group(project)
                    .group(list.note("Add a project to manage its worktrees."));
            }
            (None, Some(error)) => {
                let list = list.row(SettingsRow::new("Could not list worktrees").error(error.clone()));
                return SettingsTab::Worktrees.page().group(project).group(list);
            }
            (None, None) => {
                let list = list.row(
                    SettingsRow::new("Loading worktrees…").control(Spinner::new("worktrees-loading")),
                );
                return SettingsTab::Worktrees.page().group(project).group(list);
            }
            (Some(_), _) => page.linked().collect::<Vec<_>>(),
        };
        if trees.is_empty() {
            list = list.note("No additional worktrees. Create one to work on another branch in a separate folder.");
        }
        let delete_locked = page.refreshing_after_failure || page.load_error.is_some();
        for (ix, tree) in trees.into_iter().enumerate() {
            let count = self.worktree_session_count(&tree.path);
            let branch = match &tree.branch {
                Some(branch) => format!("Current branch: {branch}"),
                None => format!("Detached at {}", tree.head.chars().take(7).collect::<String>()),
            };
            let mut facts = vec![format!("{} in this worktree", plural(count, "session", "sessions"))];
            if let Some(n) = tree.unpushed.filter(|n| *n > 0) {
                facts.push(plural(n as usize, "unpublished commit", "unpublished commits"));
            }
            if tree.locked {
                facts.push("Locked".into());
            }
            let title = if same_project_path(&tree.path, &page.project) {
                format!("{} · Selected project folder", folder_name(&tree.path))
            } else {
                folder_name(&tree.path)
            };
            let (status, tone) = status(tree);
            let blocker = deletion_blocker(tree);
            let (reveal, remove) = (tree.path.clone(), tree.path.clone());
            list = list.row(
                SettingsRow::new(title)
                    .leading(icon_tile(ExtraIcon::FolderTree.icon().size(IconSize::Sm).color(fg.opacity(0.6)), fg))
                    .description(format!("{}\n{branch}\n{}", pretty_path(&tree.path), facts.join(" · ")))
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Badge::new(status).tone(tone))
                            .child(
                                IconButton::new(SharedString::from(format!("worktree-reveal-{ix}")), IconName::FolderOpen)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ControlSize::Sm)
                                    .disabled(tree.missing)
                                    .tooltip("Reveal folder")
                                    .on_click(move |_, _, cx| crate::ui::rail::reveal_project(&reveal, cx)),
                            )
                            .child(
                                IconButton::new(SharedString::from(format!("worktree-delete-{ix}")), IconName::Trash2)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ControlSize::Sm)
                                    .disabled(blocker.is_some() || delete_locked)
                                    .tooltip(blocker.unwrap_or("Delete worktree"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.open_worktree_deletion(&remove, cx))),
                            ),
                    ),
            );
        }
        SettingsTab::Worktrees.page().group(project).group(list)
    }

    /// MonoCode `CreateWorktreeDialog`.
    pub fn render_worktree_creation(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let creation = self.worktree_creation.as_ref()?;
        let page = &self.worktrees_page;
        let fg = cx.theme().colors.fg;
        let busy = creation.busy;
        let label = |text: &'static str| {
            div()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.7))
                .child(text)
        };
        let field = |title: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(label(title))
                .child(control)
        };
        let kind = Select::new(
            "create-worktree-kind",
            [
                Choice::new("new", "Create a new branch"),
                Choice::new("existing", "Use an existing local branch"),
            ],
        )
        .selected(if creation.existing { "existing" } else { "new" })
        .disabled(busy)
        .on_change(on_value(cx, |this, value, cx| {
            this.set_worktree_creation_existing(value == "existing", cx)
        }));
        let name = if creation.existing {
            Select::new(
                "create-worktree-existing",
                page.branches
                    .iter()
                    .filter(|b| !b.remote)
                    .map(|b| Choice::new(b.name.clone(), b.name.clone())),
            )
            .placeholder("Choose a branch…")
            .when(!creation.existing_branch.is_empty(), |s| {
                s.selected(creation.existing_branch.clone())
            })
            .disabled(busy)
            .on_change(on_value(cx, |this, value, cx| {
                this.set_worktree_creation_branch(value, cx)
            }))
            .into_any_element()
        } else {
            div()
                .child(self.worktree_branch_input.clone())
                .into_any_element()
        };
        let current = page.branches.iter().find(|b| b.current && !b.remote);
        let bases = std::iter::once(Choice::new(
            "HEAD",
            match current {
                Some(branch) => format!("Current commit ({})", branch.name),
                None => "Current commit".into(),
            },
        ))
        .chain(
            page.branches
                .iter()
                .map(|b| Choice::new(b.name.clone(), b.name.clone())),
        );
        let base = Select::new("create-worktree-base", bases)
            .selected(creation.base.clone())
            .disabled(busy)
            .on_change(on_value(cx, |this, value, cx| {
                this.set_worktree_creation_base(value, cx)
            }));
        let root = page
            .main()
            .map(|main| default_worktrees_dir(std::path::Path::new(&main.path)));
        let can_create = !busy && !self.worktree_creation_name(cx).is_empty();
        let close = app_callback(cx, |this, cx| this.close_worktree_creation(cx));
        let confirm = app_callback(cx, |this, cx| this.confirm_worktree_creation(cx));
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_size(px(12.0)).text_color(fg.opacity(0.55)).child(format!(
                "An independent working copy of {}. Existing uncommitted changes stay in their current working copy.",
                pretty_path(&page.project)
            )))
            .child(field("Branch", kind.into_any_element()))
            .child(field(
                if creation.existing { "Existing branch" } else { "New branch name" },
                name,
            ))
            .when(!creation.existing, |el| {
                el.child(field("Start from", base.into_any_element()))
            })
            .children(root.map(|root| {
                div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.4))
                    .child(format!("Created in {}", pretty_path(&root.to_string_lossy())))
            }))
            .children(creation.error.clone().map(|error| {
                div()
                    .text_size(px(12.0))
                    .text_color(cx.theme().colors.danger)
                    .child(error)
            }));
        let dialog = Dialog::new("create-worktree", "Create worktree", close)
            .child(body)
            .action(move |close| {
                Button::new("create-worktree-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .disabled(busy)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new(
                    "create-worktree-confirm",
                    if busy {
                        "Creating…"
                    } else {
                        "Create worktree"
                    },
                )
                .variant(ButtonVariant::Primary)
                .disabled(!can_create)
                .on_click(move |_, window, cx| confirm(window, cx))
            });
        Some(dialog.into_any_element())
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
                        .icon()
                        .size(IconSize::Sm)
                        .color(amber)
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
