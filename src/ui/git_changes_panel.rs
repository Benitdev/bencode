use ely_gpui_component::{
    layout::on_axis,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, SidebarMode, ViewMode};
use crate::git::{
    GitCommitInfo, GitDetailedStatus, GitFileChange, GitFileStatus, commit, discard_all,
    discard_file, get_detailed_status, get_recent_commits, stage_all, stage_file,
    unstage_all, unstage_file,
};
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let cwd = self.current_cwd.clone();
        self.git_status = get_detailed_status(&cwd);
        self.git_commits = get_recent_commits(&cwd, 8);
        cx.notify();
    }

    pub fn render_git_changes_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let branch_name = self.git_status.branch.clone();
        let ahead = self.git_status.ahead;
        let behind = self.git_status.behind;
        let staged_count = self.git_status.staged.len();
        let unstaged_count = self.git_status.unstaged.len();
        let staged_collapsed = self.git_staged_collapsed;
        let unstaged_collapsed = self.git_unstaged_collapsed;
        let history_collapsed = self.git_history_collapsed;

        let total_additions: usize = self.git_status.staged.iter().map(|f| f.additions).sum::<usize>()
            + self.git_status.unstaged.iter().map(|f| f.additions).sum::<usize>();
        let total_deletions: usize = self.git_status.staged.iter().map(|f| f.deletions).sum::<usize>()
            + self.git_status.unstaged.iter().map(|f| f.deletions).sum::<usize>();

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.0))
            .h_full()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // 1. Sidebar Top Segmented Switcher (Sessions | Files | Changes)
            .child(self.render_sidebar_mode_tabs(cx))
            // 2. Branch & Sync Status Row
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_base())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Xs)
                                    .color(MonoTheme::accent()),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_primary())
                                    .child(branch_name),
                            )
                            .when(ahead > 0 || behind > 0, |el| {
                                el.child(
                                    div()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .text_color(MonoTheme::fg_muted())
                                        .child(format!("↑{} ↓{}", ahead, behind)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("git-refresh-status-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.0))
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                    .child(
                                        Icon::new(IconName::RefreshCw)
                                            .size(IconSize::Xs)
                                            .color(MonoTheme::fg_muted()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_git_status(cx);
                                    })),
                            )
                            .when(total_additions > 0 || total_deletions > 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(
                                            div()
                                                .text_color(MonoTheme::success())
                                                .child(format!("+{}", total_additions)),
                                        )
                                        .child(
                                            div()
                                                .text_color(MonoTheme::status_error())
                                                .child(format!("-{}", total_deletions)),
                                        ),
                                )
                            }),
                    ),
            )
            // 3. Commit Composer Box
            .child(
                div()
                    .p_2p5()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_surface())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .rounded(theme.radius(Radius::Sm))
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .p_2()
                            .child(self.git_commit_input.clone())
                            .child(
                                div()
                                    .id("git-ai-wand-btn")
                                    .absolute()
                                    .top(px(4.0))
                                    .right(px(4.0))
                                    .size(px(20.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                    .child(
                                        Icon::new(IconName::WandSparkles)
                                            .size(IconSize::Xs)
                                            .color(MonoTheme::accent()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.generate_ai_commit_message(cx);
                                    })),
                            ),
                    )
                    // Commit Button Row
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("git-commit-btn")
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1p5()
                                    .h(px(26.0))
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(if staged_count > 0 {
                                        MonoTheme::accent()
                                    } else {
                                        MonoTheme::bg_hover()
                                    })
                                    .text_color(if staged_count > 0 {
                                        MonoTheme::on_accent()
                                    } else {
                                        MonoTheme::fg_subtle()
                                    })
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.9))
                                    .child(Icon::new(IconName::Check).size(IconSize::Xs))
                                    .child("Commit")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.commit_staged_changes(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("git-stage-all-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(26.0))
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .hover(|s| s.bg(MonoTheme::bg_active()))
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(IconSize::Xs)
                                            .color(MonoTheme::fg_primary()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.stage_all_workspace_changes(cx);
                                    })),
                            ),
                    ),
            )
            // 4. Scrollable Changes Lists: Staged Changes + Working Tree Changes + Commits
            .child(
                on_axis(div().id("git-changes-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .py_1()
                    // Section 1: Staged Changes
                    .child(
                        self.render_staged_section(staged_count, staged_collapsed, cx),
                    )
                    // Section 2: Changes (Unstaged)
                    .child(
                        self.render_unstaged_section(unstaged_count, unstaged_collapsed, cx),
                    )
                    // Section 3: Commit History
                    .child(
                        self.render_history_section(history_collapsed, cx),
                    ),
            )
    }

    fn render_staged_section(
        &self,
        count: usize,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

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
                    .bg(MonoTheme::bg_surface())
                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                    .child(
                        div()
                            .id("toggle-staged-collapsed-btn")
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
                                .color(MonoTheme::fg_muted()),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_muted())
                                    .child("STAGED CHANGES"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(if count > 0 {
                                        MonoTheme::accent()
                                    } else {
                                        MonoTheme::bg_hover()
                                    })
                                    .text_color(if count > 0 {
                                        MonoTheme::on_accent()
                                    } else {
                                        MonoTheme::fg_subtle()
                                    })
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .child(count.to_string()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_staged_collapsed = !this.git_staged_collapsed;
                                cx.notify();
                            })),
                    )
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .id("unstage-all-btn")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(18.0))
                                .rounded(theme.radius(Radius::Sm))
                                .cursor_pointer()
                                .hover(|s| s.bg(MonoTheme::bg_active()))
                                .child(
                                    Icon::new(IconName::Minus)
                                        .size(IconSize::Xs)
                                        .color(MonoTheme::fg_muted()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.unstage_all_workspace_changes(cx);
                                })),
                        )
                    }),
            )
            .when(!collapsed, |el| {
                if count == 0 {
                    el.child(
                        div()
                            .px_6()
                            .py_2()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_subtle())
                            .child("No staged changes"),
                    )
                } else {
                    let mut list = div().flex().flex_col();
                    for file in &self.git_status.staged {
                        list = list.child(self.render_git_file_row(file, true, cx));
                    }
                    el.child(list)
                }
            })
    }

    fn render_unstaged_section(
        &self,
        count: usize,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .mt_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1p5()
                    .bg(MonoTheme::bg_surface())
                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                    .child(
                        div()
                            .id("toggle-unstaged-collapsed-btn")
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
                                .color(MonoTheme::fg_muted()),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_muted())
                                    .child("CHANGES"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_color(MonoTheme::fg_muted())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .child(count.to_string()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_unstaged_collapsed = !this.git_unstaged_collapsed;
                                cx.notify();
                            })),
                    )
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .id("discard-all-unstaged-btn")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(18.0))
                                        .rounded(theme.radius(Radius::Sm))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(MonoTheme::bg_active()))
                                        .child(
                                            Icon::new(IconName::Undo2)
                                                .size(IconSize::Xs)
                                                .color(MonoTheme::status_error()),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.discard_all_workspace_changes(cx);
                                        })),
                                )
                                .child(
                                    div()
                                        .id("stage-all-unstaged-btn")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(18.0))
                                        .rounded(theme.radius(Radius::Sm))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(MonoTheme::bg_active()))
                                        .child(
                                            Icon::new(IconName::Plus)
                                                .size(IconSize::Xs)
                                                .color(MonoTheme::fg_muted()),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.stage_all_workspace_changes(cx);
                                        })),
                                ),
                        )
                    }),
            )
            .when(!collapsed, |el| {
                if count == 0 {
                    el.child(
                        div()
                            .px_6()
                            .py_2()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_subtle())
                            .child("Working tree clean"),
                    )
                } else {
                    let mut list = div().flex().flex_col();
                    for file in &self.git_status.unstaged {
                        list = list.child(self.render_git_file_row(file, false, cx));
                    }
                    el.child(list)
                }
            })
    }

    fn render_history_section(&self, collapsed: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let commits_count = self.git_commits.len();

        div()
            .flex()
            .flex_col()
            .mt_2()
            .border_t_1()
            .border_color(MonoTheme::border_stroke())
            .child(
                div()
                    .id("toggle-history-collapsed-btn")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_3()
                    .py_2()
                    .bg(MonoTheme::bg_surface())
                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                    .cursor_pointer()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(IconSize::Xs)
                        .color(MonoTheme::fg_muted()),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_muted())
                            .child("COMMIT HISTORY"),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_hover())
                            .text_color(MonoTheme::fg_muted())
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(commits_count.to_string()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_history_collapsed = !this.git_history_collapsed;
                        cx.notify();
                    })),
            )
            .when(!collapsed, |el| {
                let mut list = div().flex().flex_col();
                for commit in &self.git_commits {
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_1p5()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_primary())
                                            .overflow_hidden()
                                            .child(commit.message.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_1()
                                            .py_0p5()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_hover())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::accent())
                                            .child(commit.short_hash.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(commit.author.clone())
                                    .child("•")
                                    .child(commit.relative_time.clone()),
                            ),
                    );
                }
                el.child(list)
            })
    }

    fn render_git_file_row(
        &self,
        file: &GitFileChange,
        is_staged: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let file_path = file.path.clone();
        let file_name = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file_path)
            .to_string();
        let parent_dir = std::path::Path::new(&file_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();

        let is_selected = self.selected_diff_path.as_deref() == Some(&file_path);

        let (badge_text, badge_color, badge_bg) = match file.status {
            GitFileStatus::Modified => ("M", MonoTheme::warning(), MonoTheme::warning_bg()),
            GitFileStatus::Added => ("A", MonoTheme::success(), MonoTheme::success_bg()),
            GitFileStatus::Deleted => ("D", MonoTheme::status_error(), MonoTheme::status_error_bg()),
            GitFileStatus::Untracked => ("U", MonoTheme::fg_muted(), MonoTheme::bg_hover()),
            GitFileStatus::Renamed => ("R", MonoTheme::accent(), MonoTheme::bg_hover()),
        };

        let file_path_clone = file_path.clone();
        let file_path_stage = file_path.clone();
        let file_path_discard = file_path.clone();

        div()
            .id(SharedString::from(format!("git-file-row-{}", file_path)))
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .py_1p5()
            .bg(if is_selected {
                MonoTheme::bg_active()
            } else {
                gpui::rgba(0x00000000)
            })
            .hover(|s| s.bg(MonoTheme::bg_hover()))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Icon::new(IconName::FileText)
                            .size(IconSize::Xs)
                            .color(MonoTheme::fg_muted()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if is_selected {
                                        MonoTheme::accent()
                                    } else {
                                        MonoTheme::fg_primary()
                                    })
                                    .child(file_name),
                            )
                            .when(!parent_dir.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .text_color(MonoTheme::fg_subtle())
                                        .overflow_hidden()
                                        .child(parent_dir),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    // Additions / Deletions
                    .when(file.additions > 0 || file.deletions > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(theme.text_size(TextSize::Xs))
                                .child(
                                    div()
                                        .text_color(MonoTheme::success())
                                        .child(format!("+{}", file.additions)),
                                )
                                .child(
                                    div()
                                        .text_color(MonoTheme::status_error())
                                        .child(format!("-{}", file.deletions)),
                                ),
                        )
                    })
                    // Status Badge (M, A, D, U)
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(badge_bg)
                            .text_color(badge_color)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme.text_size(TextSize::Xs))
                            .child(badge_text),
                    )
                    // Stage / Unstage Action Button
                    .child(
                        div()
                            .id(SharedString::from(format!("git-action-btn-{}", file_path)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(18.0))
                            .rounded(theme.radius(Radius::Sm))
                            .hover(|s| s.bg(MonoTheme::bg_active()))
                            .child(
                                Icon::new(if is_staged {
                                    IconName::Minus
                                } else {
                                    IconName::Plus
                                })
                                .size(IconSize::Xs)
                                .color(MonoTheme::fg_muted()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_staged {
                                    this.unstage_workspace_file(&file_path_stage, cx);
                                } else {
                                    this.stage_workspace_file(&file_path_stage, cx);
                                }
                            })),
                    )
                    // Discard button for unstaged
                    .when(!is_staged, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("git-discard-btn-{}", file_path_discard)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(18.0))
                                .rounded(theme.radius(Radius::Sm))
                                .hover(|s| s.bg(MonoTheme::bg_active()))
                                .child(
                                    Icon::new(IconName::Undo2)
                                        .size(IconSize::Xs)
                                        .color(MonoTheme::status_error()),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.discard_workspace_file(&file_path_discard, cx);
                                })),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_diff_path = Some(file_path_clone.clone());
                this.active_view_mode = ViewMode::Changes;
                cx.notify();
            }))
    }

    // Git Action Handlers
    pub fn stage_workspace_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let _ = stage_file(&self.current_cwd, file);
        self.refresh_git_status(cx);
    }

    pub fn unstage_workspace_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let _ = unstage_file(&self.current_cwd, file);
        self.refresh_git_status(cx);
    }

    pub fn discard_workspace_file(&mut self, file: &str, cx: &mut Context<Self>) {
        let _ = discard_file(&self.current_cwd, file);
        self.refresh_git_status(cx);
    }

    pub fn stage_all_workspace_changes(&mut self, cx: &mut Context<Self>) {
        let _ = stage_all(&self.current_cwd);
        self.refresh_git_status(cx);
    }

    pub fn unstage_all_workspace_changes(&mut self, cx: &mut Context<Self>) {
        let _ = unstage_all(&self.current_cwd);
        self.refresh_git_status(cx);
    }

    pub fn discard_all_workspace_changes(&mut self, cx: &mut Context<Self>) {
        let _ = discard_all(&self.current_cwd);
        self.refresh_git_status(cx);
    }

    pub fn commit_staged_changes(&mut self, cx: &mut Context<Self>) {
        let message = self.git_commit_input.read(cx).text().trim().to_string();
        if message.is_empty() {
            return;
        }

        let _ = commit(&self.current_cwd, &message);
        self.git_commit_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });
        self.refresh_git_status(cx);
    }

    pub fn generate_ai_commit_message(&mut self, cx: &mut Context<Self>) {
        // Auto-synthesize conventional commit message from changed file list
        let changed_files = if !self.git_status.staged.is_empty() {
            &self.git_status.staged
        } else {
            &self.git_status.unstaged
        };

        if changed_files.is_empty() {
            return;
        }

        let first = &changed_files[0].path;
        let summary = if first.ends_with(".rs") {
            format!("refactor(core): update {} logic", first)
        } else if first.contains("ui") {
            format!("feat(ui): update components in {}", first)
        } else if first.ends_with(".toml") || first.ends_with(".json") {
            format!("chore: update configuration in {}", first)
        } else {
            format!("feat: update {}", first)
        };

        self.git_commit_input.update(cx, |input, cx| {
            input.set_text(&summary, cx);
        });
        cx.notify();
    }
}
