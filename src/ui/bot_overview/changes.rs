//! MonoCode `MonoChangesView` and `MonoProjectCommit`: a row of project
//! tabs (each with its mascot, name and how many files changed), then the
//! open project's commit box, its Staged Changes and Changes. Stage and
//! unstage go through the git index (`app/bot_changes.rs`), as the
//! project's own Changes panel does; a file opens its review in the
//! workspace. No discard, folders, tree view or generated messages here:
//! those stay in the project's Changes panel.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*,
};

use super::{OverviewTab, STROKE, empty_state};
use crate::app::BenCodeApp;
use crate::app::bot_changes::{can_publish, can_push, can_sync};
use crate::bots::Bot;
use crate::git::GitFileChange;
use crate::ui::git_changes_panel::{status_color, status_letter};
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    pub(super) fn render_bot_changes(&self, bot: &Bot, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let name = bot.display_name();
        let Some(root) = self.bot_changes_root() else {
            return empty_state(
                OverviewTab::Changes,
                format!(
                    "Give {name} a project to work on, and its changes show up here to commit."
                ),
                fg,
            );
        };
        let changes = &self.bots.changes;
        let tabs = div()
            .id("bot-changes-projects")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(1.0))
            .h(px(36.0))
            .px_2()
            .overflow_x_scroll()
            .border_b_1()
            .border_color(fg.opacity(STROKE))
            .children(bot.projects.iter().enumerate().map(|(ix, path)| {
                let active = *path == root;
                let count = changes
                    .projects
                    .get(path)
                    .map_or(0, crate::app::bot_changes::ProjectChanges::file_count);
                let target = path.clone();
                div()
                    .id(("bot-changes-project", ix))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .h(px(24.0))
                    .px_2()
                    .rounded(px(6.0))
                    .text_size(Text::Base.px())
                    .cursor_pointer()
                    .tooltip(Tooltip::text(path.clone()))
                    .map(|el| {
                        if active {
                            el.bg(fg.fill(Fill::Selected)).text_color(fg)
                        } else {
                            el.text_color(fg.ink(Ink::Secondary))
                                .hover(move |s| s.text_color(fg.ink(Ink::Strong)))
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_bot_changes_project(&target, cx)
                    }))
                    .child(crate::ui::mascot::pixel_sprite(
                        &self.project_mascot(path).rest,
                        px(12.0),
                        self.project_color(path),
                        false,
                    ))
                    .child(
                        div()
                            .max_w(px(160.0))
                            .truncate()
                            .child(self.rail_project_label(path)),
                    )
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .text_color(fg.ink(Ink::Muted))
                                .child(count.to_string()),
                        )
                    })
            }));
        let project = changes.projects.get(&root);
        let body: AnyElement = match project {
            None => div()
                .p_4()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Muted))
                .child("Loading…")
                .into_any_element(),
            Some(project) if !project.repo => empty_state(
                OverviewTab::Changes,
                "This project isn’t a Git repository, so there’s nothing to commit.".to_string(),
                fg,
            ),
            Some(project) => self.render_bot_project_commit(project, cx),
        };
        let notice = changes
            .error
            .clone()
            .map(|err| (err, colors.danger))
            .or_else(|| {
                changes
                    .status
                    .clone()
                    .map(|text| (text, fg.ink(Ink::Secondary)))
            })
            .map(|(text, color)| {
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(fg.opacity(STROKE))
                    .px_4()
                    .py_2()
                    .text_size(Text::Base.px())
                    .text_color(color)
                    .child(text)
            });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(tabs)
            .child(body)
            .children(notice)
            .into_any_element()
    }

    /// MonoCode `MonoProjectCommit`: the message, Commit (and Commit &
    /// Push), the branch's push or sync, then the index's two sections.
    fn render_bot_project_commit(
        &self,
        project: &crate::app::bot_changes::ProjectChanges,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let changes = &self.bots.changes;
        let busy = changes.busy.is_some();
        let staged = &project.status.staged;
        let unstaged = &project.status.unstaged;
        let has_message = !changes.message.read(cx).text().trim().is_empty();
        let can_commit = !staged.is_empty() && has_message && !busy;
        let sync = &project.sync;
        let branch = sync
            .branch
            .clone()
            .unwrap_or_else(|| project.status.branch.clone());
        let mut actions = div().flex().items_center().gap_2().mt_2().child(
            Button::new("bot-commit", "Commit")
                .size(ControlSize::Sm)
                .disabled(!can_commit)
                .on_click(cx.listener(|this, _, _, cx| this.bot_commit(false, cx))),
        );
        if can_push(sync) {
            actions = actions.child(
                Button::new("bot-commit-push", "Commit & Push")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .disabled(!can_commit)
                    .on_click(cx.listener(|this, _, _, cx| this.bot_commit(true, cx))),
            );
        }
        let push_label = if can_sync(sync) {
            Some((format!("Sync ↓{} ↑{}", sync.behind, sync.ahead), true))
        } else if can_publish(sync) {
            Some(("Publish Branch".to_string(), false))
        } else {
            None
        };
        if let Some((label, sync)) = push_label {
            actions = actions.child(
                Button::new("bot-push", label)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .icon(if sync {
                        IconName::RefreshCw
                    } else {
                        IconName::Upload
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.bot_push(sync, cx))),
            );
        }
        let commit_box = div()
            .flex_none()
            .px_3()
            .py_3()
            .border_b_1()
            .border_color(fg.opacity(STROKE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .mb_2()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(IconSize::Xs)
                            .color(fg.ink(Ink::Muted)),
                    )
                    .child(div().min_w_0().truncate().child(branch)),
            )
            .child(
                div()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(fg.fill(Fill::Strong))
                    .bg(fg.fill(Fill::Faint))
                    .px_2()
                    .py_1p5()
                    .text_size(Text::Base.px())
                    .child(changes.message.clone()),
            )
            .child(actions);
        let list = if staged.is_empty() && unstaged.is_empty() {
            div()
                .py_6()
                .flex()
                .justify_center()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Muted))
                .child("No changes")
                .into_any_element()
        } else {
            div()
                .when(!staged.is_empty(), |el| {
                    el.child(self.render_bot_change_section("Staged Changes", staged, true, cx))
                })
                .when(!unstaged.is_empty(), |el| {
                    el.child(self.render_bot_change_section("Changes", unstaged, false, cx))
                })
                .into_any_element()
        };
        let scroll = div()
            .id("bot-changes-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1()
            .child(list);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(commit_box)
            .child(crate::ui::scrollbar::Scrolled::new(
                "bot-changes-scrollbar",
                scroll,
            ))
            .into_any_element()
    }

    /// MonoCode `FileSection`: its title and count with Stage / Unstage
    /// all, then a row per file (status letter, name, folder; + or − on
    /// hover).
    fn render_bot_change_section(
        &self,
        title: &'static str,
        files: &[GitFileChange],
        staged: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let diff = crate::ui::appearance::diff_colors(cx);
        let busy = self.bots.changes.busy.is_some();
        let section = SharedString::from(format!("bot-changes-{title}"));
        let all_icon = if staged {
            IconName::Minus
        } else {
            IconName::Plus
        };
        let header = div()
            .id(section.clone())
            .group(section.clone())
            .flex()
            .items_center()
            .h(px(28.0))
            .px_3()
            .text_size(Text::Sm.px())
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg.ink(Ink::Secondary))
            .child(
                div()
                    .flex_1()
                    .child(format!("{} {}", title.to_uppercase(), files.len())),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{section}-all")))
                    .flex()
                    .size(px(20.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .invisible()
                    .group_hover(section.clone(), |s| s.visible())
                    .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                    .tooltip(Tooltip::text(if staged {
                        "Unstage All Changes"
                    } else {
                        "Stage All Changes"
                    }))
                    .when(!busy, |el| {
                        el.on_click(
                            cx.listener(move |this, _, _, cx| this.bot_stage_all(!staged, cx)),
                        )
                    })
                    .child(
                        Icon::new(all_icon)
                            .size(IconSize::Xs)
                            .color(fg.ink(Ink::Secondary)),
                    ),
            );
        div()
            .child(header)
            .children(files.iter().enumerate().map(|(ix, file)| {
                let row = SharedString::from(format!("{section}-{ix}"));
                let (name, folder) = match file.path.rsplit_once('/') {
                    Some((folder, name)) => (name.to_string(), folder.to_string()),
                    None => (file.path.clone(), String::new()),
                };
                let (open_path, stage_path) = (file.path.clone(), file.path.clone());
                div()
                    .id(row.clone())
                    .group(row.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(26.0))
                    .px_3()
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.fill(Fill::Hover)))
                    .tooltip(Tooltip::text(file.path.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_bot_change(open_path.clone(), staged, cx)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(Text::Base.px())
                                    .text_color(fg.ink(Ink::Strong))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(Text::Sm.px())
                                    .text_color(fg.ink(Ink::Muted))
                                    .child(folder),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{row}-stage")))
                            .flex()
                            .flex_none()
                            .size(px(20.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .invisible()
                            .group_hover(row.clone(), |s| s.visible())
                            .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                            .tooltip(Tooltip::text(if staged { "Unstage" } else { "Stage" }))
                            .when(!busy, |el| {
                                el.on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.bot_stage_file(stage_path.clone(), !staged, cx)
                                }))
                            })
                            .child(
                                Icon::new(all_icon)
                                    .size(IconSize::Xs)
                                    .color(fg.ink(Ink::Secondary)),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(12.0))
                            .text_size(Text::Sm.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(status_letter_color(file, diff))
                            .child(status_letter(&file.status)),
                    )
            }))
            .into_any_element()
    }
}

fn status_letter_color(file: &GitFileChange, diff: crate::ui::appearance::DiffColors) -> Hsla {
    status_color(&file.status, diff)
}
