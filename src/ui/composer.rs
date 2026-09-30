use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::git::get_branches;
use crate::ui::theme::MonoTheme;
use crate::workspace::{list_workspace_files, BUILTIN_SKILLS};

impl BenCodeApp {
    pub fn render_composer(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_running = self.is_agent_running;
        let is_model_open = self.is_model_picker_open;
        let is_branch_open = self.is_branch_picker_open;
        let is_skill_open = self.is_skill_picker_open;
        let is_mention_open = self.is_mention_picker_open;
        let skill_q = self.skill_query.clone();
        let mention_q = self.mention_query.clone();

        let cwd = session.map(|s| s.cwd.as_str()).unwrap_or(".");
        let branches = get_branches(cwd);

        let model_name = session
            .map(|s| {
                if s.model.is_empty() {
                    "Claude 3.7 Sonnet"
                } else {
                    s.model.as_str()
                }
            })
            .unwrap_or(&self.selected_model);

        let branch_name = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
        let harness_dot_color = match harness {
            "claude" => MonoTheme::claude_orange(),
            "antigravity" => MonoTheme::antigravity_blue(),
            "codex" => MonoTheme::codex_green(),
            _ => MonoTheme::accent(),
        };

        let (context_label, pct) = match (session.and_then(|s| s.context_used), session.and_then(|s| s.context_window)) {
            (Some(used), Some(window)) if window > 0 => {
                let used_str = if used >= 1_000_000 {
                    format!("{:.1}M", used as f64 / 1_000_000.0)
                } else if used >= 1000 {
                    format!("{:.1}k", used as f64 / 1000.0)
                } else {
                    format!("{}", used)
                };
                let win_str = if window >= 1_000_000 {
                    format!("{:.1}M", window as f64 / 1_000_000.0)
                } else if window >= 1000 {
                    format!("{:.1}k", window as f64 / 1000.0)
                } else {
                    format!("{}", window)
                };
                let p = ((used as f64 / window as f64) * 100.0).round() as u64;
                (format!("{} / {} tokens ({}%)", used_str, win_str, p), p)
            }
            _ => ("0 / 200k tokens (0%)".to_string(), 0),
        };

        let perm_mode = self.permission_mode;
        let (perm_icon, perm_label) = match perm_mode {
            PermissionMode::Auto => ("⚡", "Auto"),
            PermissionMode::Confirm => ("🛡", "Confirm"),
            PermissionMode::ReadOnly => ("🔒", "Read-Only"),
        };

        let filtered_skills: Vec<_> = BUILTIN_SKILLS
            .iter()
            .filter(|s| {
                skill_q.is_empty()
                    || s.name.contains(&skill_q)
                    || s.description.to_lowercase().contains(&skill_q)
            })
            .collect();

        let workspace_path = std::path::Path::new(cwd);
        let workspace_files = list_workspace_files(workspace_path, 40);
        let filtered_files: Vec<_> = workspace_files
            .into_iter()
            .filter(|f| mention_q.is_empty() || f.to_lowercase().contains(&mention_q))
            .take(8)
            .collect();

        let filtered_notes: Vec<_> = self
            .notes
            .iter()
            .filter(|n| {
                mention_q.is_empty()
                    || n.title.to_lowercase().contains(&mention_q)
                    || n.slug.contains(&mention_q)
            })
            .take(5)
            .cloned()
            .collect();

        div()
            .relative()
            .px_6()
            .pb_5()
            .pt_2()
            .bg(MonoTheme::bg_base())
            .child(
                // Floating Composer Card
                div()
                    .relative()
                    .max_w(px(840.0))
                    .mx_auto()
                    .rounded(theme.radius(Radius::Lg))
                    .border_1()
                    .border_color(if is_running {
                        MonoTheme::accent()
                    } else {
                        MonoTheme::border_stroke()
                    })
                    .bg(MonoTheme::bg_surface())
                    // Popover: Model Picker Menu
                    .when(is_model_open, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(110.0))
                                .w(px(260.0))
                                .rounded(theme.radius(Radius::Md))
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .bg(MonoTheme::bg_surface())
                                .p_2()
                                .gap_1()
                                .flex()
                                .flex_col()
                                // Header: Anthropic
                                .child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(MonoTheme::fg_subtle())
                                        .child("ANTHROPIC"),
                                )
                                .child(self.render_model_option("Claude 3.7 Sonnet", "claude", MonoTheme::claude_orange(), cx))
                                .child(self.render_model_option("Claude 3.5 Sonnet", "claude", MonoTheme::claude_orange(), cx))
                                .child(self.render_model_option("Claude 3.5 Haiku", "claude", MonoTheme::claude_orange(), cx))
                                // Header: Google Antigravity
                                .child(
                                    div()
                                        .pt_2()
                                        .px_2()
                                        .py_1()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(MonoTheme::fg_subtle())
                                        .child("GOOGLE ANTIGRAVITY"),
                                )
                                .child(self.render_model_option("Gemini 3.8 Flash (High)", "antigravity", MonoTheme::antigravity_blue(), cx))
                                .child(self.render_model_option("Gemini 2.5 Flash", "antigravity", MonoTheme::antigravity_blue(), cx))
                                .child(self.render_model_option("Gemini 2.5 Pro", "antigravity", MonoTheme::antigravity_blue(), cx))
                                // Header: OpenAI
                                .child(
                                    div()
                                        .pt_2()
                                        .px_2()
                                        .py_1()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(MonoTheme::fg_subtle())
                                        .child("OPENAI"),
                                )
                                .child(self.render_model_option("GPT-4o", "codex", MonoTheme::codex_green(), cx))
                                .child(self.render_model_option("o3-mini", "codex", MonoTheme::codex_green(), cx))
                        )
                    })
                    // Popover: Branch Picker Menu
                    .when(is_branch_open, |el| {
                        el.child(
                            on_axis(div().id("composer-branch-scroll"))
                                .absolute()
                                .left(px(160.0))
                                .bottom(px(110.0))
                                .w(px(240.0))
                                .max_h(px(220.0))
                                .overflow_y_scroll()
                                .rounded(theme.radius(Radius::Md))
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .bg(MonoTheme::bg_surface())
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(MonoTheme::fg_subtle())
                                        .child("SWITCH GIT BRANCH"),
                                )
                                .children(branches.into_iter().map(|b| {
                                    let is_cur = b == branch_name;
                                    let b_clone = b.clone();
                                    div()
                                        .id(SharedString::from(format!("branch-opt-{}", b)))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded(theme.radius(Radius::Sm))
                                        .cursor_pointer()
                                        .when(is_cur, |el| el.bg(MonoTheme::bg_active()))
                                        .when(!is_cur, |el| el.hover(|s| s.bg(MonoTheme::bg_hover())))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_session_branch(b_clone.clone(), cx);
                                        }))
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .font_weight(if is_cur { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                .text_color(if is_cur { MonoTheme::fg_primary() } else { MonoTheme::fg_muted() })
                                                .child(format!("⎇ {}", b)),
                                        )
                                        .when(is_cur, |el| {
                                            el.child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::accent())
                                                    .child("✓"),
                                            )
                                        })
                                })),
                        )
                    })
                    // Popover: Slash Skills Autocomplete
                    .when(is_skill_open, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(100.0))
                                .w(px(380.0))
                                .max_h(px(260.0))
                                .rounded(theme.radius(Radius::Md))
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .bg(MonoTheme::bg_surface())
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_2()
                                        .py_1()
                                        .border_b_1()
                                        .border_color(MonoTheme::border_stroke())
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(MonoTheme::skill_gold())
                                                .child("⚡ Slash Skills"),
                                        )
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .text_color(MonoTheme::fg_subtle())
                                                .child("Click or ↵ to select"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("skill-popover-list")
                                        .flex_1()
                                        .overflow_y_scroll()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .children(filtered_skills.into_iter().map(|skill| {
                                            let skill_name = skill.name;
                                            div()
                                                .id(SharedString::from(format!("skill-item-{}", skill.name.replace('/', ""))))
                                                .p_2()
                                                .rounded(theme.radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(MonoTheme::bg_hover()))
                                                .flex()
                                                .flex_col()
                                                .gap_0p5()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_size(theme.text_size(TextSize::Sm))
                                                                .text_color(MonoTheme::skill_gold())
                                                                .child(skill.name),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::fg_subtle())
                                                                .child(skill.example),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .text_color(MonoTheme::fg_muted())
                                                        .child(skill.description),
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.insert_skill(skill_name, cx);
                                                }))
                                        })),
                                ),
                        )
                    })
                    // Popover: Mention Files & Notes Autocomplete
                    .when(is_mention_open, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(100.0))
                                .w(px(400.0))
                                .max_h(px(280.0))
                                .rounded(theme.radius(Radius::Md))
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .bg(MonoTheme::bg_surface())
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_2()
                                        .py_1()
                                        .border_b_1()
                                        .border_color(MonoTheme::border_stroke())
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(MonoTheme::mention_cyan())
                                                .child("@ Mention Context"),
                                        )
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .text_color(MonoTheme::fg_subtle())
                                                .child("Files & Notes"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("mention-popover-list")
                                        .flex_1()
                                        .overflow_y_scroll()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .children(filtered_files.into_iter().map(|file_path| {
                                            let fp = file_path.clone();
                                            div()
                                                .id(SharedString::from(format!("mention-file-{}", file_path.replace('/', "-").replace('.', "_"))))
                                                .px_2()
                                                .py_1p5()
                                                .rounded(theme.radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(MonoTheme::bg_hover()))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::mention_cyan())
                                                                .child("📄"),
                                                        )
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::fg_primary())
                                                                .truncate()
                                                                .child(file_path),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .text_color(MonoTheme::fg_subtle())
                                                        .child("file"),
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.insert_mention(&fp, cx);
                                                }))
                                        }))
                                        .children(filtered_notes.into_iter().map(|note| {
                                            let slug = format!("note/{}", note.slug);
                                            let title = note.title.clone();
                                            div()
                                                .id(SharedString::from(format!("mention-note-{}", note.id)))
                                                .px_2()
                                                .py_1p5()
                                                .rounded(theme.radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(MonoTheme::bg_hover()))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::accent())
                                                                .child("📝"),
                                                        )
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::fg_primary())
                                                                .truncate()
                                                                .child(format!("@note/{} ({})", note.slug, title)),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .text_color(MonoTheme::fg_subtle())
                                                        .child("note"),
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.insert_mention(&slug, cx);
                                                }))
                                        })),
                                ),
                        )
                    })
                    .child(
                        // 1. Controls Top Bar (Model Picker, Permission Mode, Branch, Context Meter)
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    // Model Picker Chip
                                    .child(
                                        div()
                                            .id("composer-model-picker")
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.is_model_picker_open = !this.is_model_picker_open;
                                                this.is_branch_picker_open = false;
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .size(px(6.0))
                                                    .rounded_full()
                                                    .bg(harness_dot_color),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(MonoTheme::fg_primary())
                                                    .child(model_name.to_string()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child(if is_model_open { "▴" } else { "▾" }),
                                            ),
                                    )
                                    // Permission Mode Chip (⚡ Auto / 🛡 Confirm / 🔒 Read-Only)
                                    .child(
                                        div()
                                            .id("composer-perm-chip")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.permission_mode = match this.permission_mode {
                                                    PermissionMode::Auto => PermissionMode::Confirm,
                                                    PermissionMode::Confirm => PermissionMode::ReadOnly,
                                                    PermissionMode::ReadOnly => PermissionMode::Auto,
                                                };
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .child(perm_icon),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(MonoTheme::fg_muted())
                                                    .child(perm_label),
                                            ),
                                    )
                                    // Branch Pill
                                    .child(
                                        div()
                                            .id("composer-branch-pill")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.is_branch_picker_open = !this.is_branch_picker_open;
                                                this.is_model_picker_open = false;
                                                cx.notify();
                                            }))
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .child(format!("⎇ {}", branch_name)),
                                    ),
                            )
                            // Right Side: Context Meter with Mini Progress Bar
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .child(
                                        div()
                                            .w(px(32.0))
                                            .h(px(4.0))
                                            .rounded_full()
                                            .bg(MonoTheme::bg_hover())
                                            .child(
                                                div()
                                                    .w(px(32.0 * (pct as f32 / 100.0).clamp(0.05, 1.0)))
                                                    .h_full()
                                                    .rounded_full()
                                                    .bg(if pct > 80 {
                                                        MonoTheme::danger()
                                                    } else if pct > 50 {
                                                        MonoTheme::warning()
                                                    } else {
                                                        MonoTheme::accent()
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child(context_label),
                                    ),
                            ),
                    )
                    .child(
                        // 2. Interactive Input Prompt Text Area + Send Action
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .p_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(46.0))
                                    .px_2()
                                    .py_1()
                                    .child(self.prompt_input.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    // Attachment button (+)
                                    .child(
                                        div()
                                            .id("composer-attach-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.0))
                                            .rounded(theme.radius(Radius::Md))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_base())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .text_color(MonoTheme::fg_muted())
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .child("+"),
                                    )
                                    // Send / Stop action button
                                    .child(
                                        div()
                                            .id("composer-send-action-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.0))
                                            .rounded(theme.radius(Radius::Md))
                                            .bg(if is_running {
                                                MonoTheme::danger()
                                            } else {
                                                MonoTheme::accent()
                                            })
                                            .text_color(MonoTheme::on_accent())
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.9))
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .font_weight(FontWeight::BOLD)
                                            .child(if is_running { "■" } else { "↑" })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.handle_send_or_stop(cx);
                                            })),
                                    ),
                            ),
                    )
                    // 3. Bottom Keyboard Hints
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .px_3()
                            .pb_2()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_subtle())
                            .child("↵ to send · ⇧↵ for new line"),
                    ),
            )
    }

    fn render_model_option(
        &self,
        name: &'static str,
        harness: &'static str,
        color: gpui::Rgba,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_sel = self.selected_model == name;

        div()
            .id(SharedString::from(format!("model-option-{}", name)))
            .flex()
            .items_center()
            .justify_between()
            .px_2p5()
            .py_1p5()
            .rounded(theme.radius(Radius::Sm))
            .cursor_pointer()
            .when(is_sel, |el| el.bg(MonoTheme::bg_active()))
            .when(!is_sel, |el| el.hover(|s| s.bg(MonoTheme::bg_hover())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_session_model(name.to_string(), harness.to_string(), cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(6.0))
                            .rounded_full()
                            .bg(color),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(if is_sel { FontWeight::BOLD } else { FontWeight::NORMAL })
                            .text_color(if is_sel { MonoTheme::fg_primary() } else { MonoTheme::fg_muted() })
                            .child(name),
                    ),
            )
            .when(is_sel, |el| {
                el.child(
                    div()
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(MonoTheme::accent())
                        .child("✓"),
                )
            })
    }
}
