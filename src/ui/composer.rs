//! Bottom prompt composer: multi-line input, model/branch/permission selectors,
//! mention (@) & slash-skill (/) autocomplete popovers, context token meter, and send/stop button.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px,
};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::harness::catalog::{self, ModelOption};
use crate::ui::theme;
use crate::workspace::BUILTIN_SKILLS;

impl BenCodeApp {
    pub fn render_composer(
        &self,
        session: Option<&SessionRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let is_running = self.is_agent_running();
        let is_model_open = self.is_model_picker_open;
        let is_branch_open = self.is_branch_picker_open;
        let is_skill_open = self.is_skill_picker_open;
        let is_mention_open = self.is_mention_picker_open;
        let skill_q = self.skill_query.clone();
        let mention_q = self.mention_query.clone();

        let branches = self.workspace.branches.clone();

        let model_name = catalog::label_for(session.map_or(self.selected_model.as_str(), |s| s.model.as_str()));

        let branch_name = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
        let harness_dot_color = theme::harness_color(harness, &colors);

        let (context_label, pct) = match (session.and_then(|s| s.context_used), session.and_then(|s| s.context_window)) {
            (Some(used), Some(window)) if window > 0 => {
                let used_str = if used >= 1_000_000 {
                    format!("{:.1}M", used as f64 / 1_000_000.0)
                } else if used >= 1000 {
                    format!("{:.1}k", used as f64 / 1000.0)
                } else {
                    format!("{used}")
                };
                let win_str = if window >= 1_000_000 {
                    format!("{:.1}M", window as f64 / 1_000_000.0)
                } else if window >= 1000 {
                    format!("{:.1}k", window as f64 / 1000.0)
                } else {
                    format!("{window}")
                };
                let p = ((used as f64 / window as f64) * 100.0).round() as u64;
                (format!("{used_str} / {win_str} tokens ({p}%)"), p)
            }
            _ => ("0 / 200k tokens (0%)".to_string(), 0),
        };

        let perm_mode = self.permission_mode;
        let (perm_icon_name, perm_label) = match perm_mode {
            PermissionMode::Auto => (IconName::Zap, "Auto"),
            PermissionMode::Confirm => (IconName::Shield, "Confirm"),
            PermissionMode::ReadOnly => (IconName::Square, "Read-Only"),
        };

        let filtered_skills: Vec<_> = BUILTIN_SKILLS
            .iter()
            .filter(|s| {
                skill_q.is_empty()
                    || s.name.contains(&skill_q)
                    || s.description.to_lowercase().contains(&skill_q)
            })
            .collect();

        let filtered_files: Vec<_> = self
            .workspace
            .files
            .iter()
            .filter(|&f| mention_q.is_empty() || f.to_lowercase().contains(&mention_q))
            .take(8)
            .cloned()
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
            .bg(colors.bg)
            .child(
                // Floating Composer Card
                div()
                    .relative()
                    .max_w(px(840.0))
                    .mx_auto()
                    .rounded(cx.theme().radius(Radius::Lg))
                    .border_1()
                    .border_color(if is_running { colors.accent } else { colors.border })
                    .bg(colors.surface)
                    // Popover: Model Picker Menu
                    .when(is_model_open, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(110.0))
                                .w(px(260.0))
                                .rounded(cx.theme().radius(Radius::Md))
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.surface)
                                .p_2()
                                .gap_1()
                                .flex()
                                .flex_col()
                                .children(self.render_model_groups(cx))
                        )
                    })
                    // Popover: Branch Picker Menu
                    .when(is_branch_open, |el| {
                        let c_border = colors.border;
                        let c_surf = colors.surface;
                        let c_fg = colors.fg;
                        let c_muted = colors.fg_muted;
                        let c_subtle = colors.fg_subtle;
                        let c_active = colors.active;
                        let c_hover = colors.hover;
                        let c_accent = colors.accent;

                        el.child(
                            on_axis(div().id("composer-branch-scroll"))
                                .absolute()
                                .left(px(160.0))
                                .bottom(px(110.0))
                                .w(px(240.0))
                                .max_h(px(220.0))
                                .overflow_y_scroll()
                                .rounded(cx.theme().radius(Radius::Md))
                                .border_1()
                                .border_color(c_border)
                                .bg(c_surf)
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(c_subtle)
                                        .child("SWITCH GIT BRANCH"),
                                )
                                .children(branches.into_iter().map(|b| {
                                    let is_cur = b == branch_name;
                                    let b_clone = b.clone();
                                    div()
                                        .id(SharedString::from(format!("branch-opt-{b}")))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded(cx.theme().radius(Radius::Sm))
                                        .cursor_pointer()
                                        .when(is_cur, |el| el.bg(c_active))
                                        .when(!is_cur, |el| el.hover(|s| s.bg(c_hover)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_session_branch(b_clone.clone(), cx);
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                .font_weight(if is_cur { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                .text_color(if is_cur { c_fg } else { c_muted })
                                                .child(Icon::new(IconName::GitBranch).size(IconSize::Xs))
                                                .child(b),
                                        )
                                        .when(is_cur, |el| {
                                            el.child(
                                                div()
                                                    .text_color(c_accent)
                                                    .child(Icon::new(IconName::Check).size(IconSize::Xs)),
                                            )
                                        })
                                })),
                        )
                    })
                    // Popover: Slash Skills Autocomplete
                    .when(is_skill_open, |el| {
                        let c_border = colors.border;
                        let c_surf = colors.surface;
                        let c_fg = colors.fg;
                        let c_muted = colors.fg_muted;
                        let c_subtle = colors.fg_subtle;
                        let c_hover = colors.hover;
                        let c_accent = colors.accent;

                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(100.0))
                                .w(px(380.0))
                                .max_h(px(260.0))
                                .rounded(cx.theme().radius(Radius::Md))
                                .border_1()
                                .border_color(c_border)
                                .bg(c_surf)
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
                                        .border_color(c_border)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .child(
                                                    Icon::new(IconName::Zap)
                                                        .size(IconSize::Xs)
                                                        .color(c_accent),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(c_accent)
                                                        .child("Slash Skills"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                .text_color(c_subtle)
                                                .child(format!("{} available", filtered_skills.len())),
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
                                            let name = skill.name.to_string();
                                            div()
                                                .id(SharedString::from(format!("skill-opt-{}", skill.name)))
                                                .px_2()
                                                .py_1p5()
                                                .rounded(cx.theme().radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(c_hover))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                                .text_color(c_fg)
                                                                .child(format!("/{}", skill.name)),
                                                        )
                                                        .child(
                                                            Badge::new("skill").tone(Tone::Neutral),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                        .text_color(c_muted)
                                                        .truncate()
                                                        .child(skill.description),
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.insert_skill(&name, cx);
                                                }))
                                        })),
                                ),
                        )
                    })
                    // Popover: Mention Files & Notes Autocomplete
                    .when(is_mention_open, |el| {
                        let c_border = colors.border;
                        let c_surf = colors.surface;
                        let c_fg = colors.fg;
                        let c_subtle = colors.fg_subtle;
                        let c_hover = colors.hover;
                        let c_accent = colors.accent;

                        el.child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .bottom(px(100.0))
                                .w(px(380.0))
                                .max_h(px(260.0))
                                .rounded(cx.theme().radius(Radius::Md))
                                .border_1()
                                .border_color(c_border)
                                .bg(c_surf)
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
                                        .border_color(c_border)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .child(
                                                    Icon::new(IconName::AtSign)
                                                        .size(IconSize::Xs)
                                                        .color(c_accent),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(c_accent)
                                                        .child("Mention Context"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                .text_color(c_subtle)
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
                                                .rounded(cx.theme().radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(c_hover))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            Icon::new(IconName::FileText)
                                                                .size(IconSize::Xs)
                                                                .color(c_accent),
                                                        )
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                                .text_color(c_fg)
                                                                .truncate()
                                                                .child(file_path),
                                                        ),
                                                )
                                                .child(Badge::new("file").tone(Tone::Neutral))
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
                                                .rounded(cx.theme().radius(Radius::Sm))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(c_hover))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            Icon::new(IconName::FileText)
                                                                .size(IconSize::Xs)
                                                                .color(c_accent),
                                                        )
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                                .text_color(c_fg)
                                                                .truncate()
                                                                .child(format!("@note/{} ({title})", note.slug)),
                                                        ),
                                                )
                                                .child(Badge::new("note").tone(Tone::Info))
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
                            .border_color(colors.border)
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
                                            .rounded(cx.theme().radius(Radius::Sm))
                                            .bg(colors.bg)
                                            .border_1()
                                            .border_color(colors.border)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(colors.hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.is_model_picker_open = !this.is_model_picker_open;
                                                this.is_branch_picker_open = false;
                                                cx.notify();
                                            }))
                                            .child(div().size(px(6.0)).rounded_full().bg(harness_dot_color))
                                            .child(
                                                div()
                                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(colors.fg)
                                                    .child(model_name.to_string()),
                                            )
                                            .child(
                                                Icon::new(if is_model_open { IconName::ChevronUp } else { IconName::ChevronDown })
                                                    .size(IconSize::Xs)
                                                    .color(colors.fg_subtle),
                                            ),
                                    )
                                    // Permission Mode Chip (Auto / Confirm / Read-Only)
                                    .child(
                                        div()
                                            .id("composer-perm-chip")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(cx.theme().radius(Radius::Sm))
                                            .bg(colors.bg)
                                            .border_1()
                                            .border_color(colors.border)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(colors.hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.permission_mode = match this.permission_mode {
                                                    PermissionMode::Auto => PermissionMode::Confirm,
                                                    PermissionMode::Confirm => PermissionMode::ReadOnly,
                                                    PermissionMode::ReadOnly => PermissionMode::Auto,
                                                };
                                                cx.notify();
                                            }))
                                            .child(
                                                Icon::new(perm_icon_name)
                                                    .size(IconSize::Xs)
                                                    .color(colors.fg_muted),
                                            )
                                            .child(
                                                div()
                                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(colors.fg_muted)
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
                                            .rounded(cx.theme().radius(Radius::Sm))
                                            .bg(colors.bg)
                                            .border_1()
                                            .border_color(colors.border)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(colors.hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.is_branch_picker_open = !this.is_branch_picker_open;
                                                this.is_model_picker_open = false;
                                                cx.notify();
                                            }))
                                            .child(
                                                Icon::new(IconName::GitBranch)
                                                    .size(IconSize::Xs)
                                                    .color(colors.fg_muted),
                                            )
                                            .child(
                                                div()
                                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                                    .text_color(colors.fg_muted)
                                                    .child(branch_name.to_string()),
                                            ),
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
                                    .rounded(cx.theme().radius(Radius::Sm))
                                    .child(
                                        div()
                                            .w(px(32.0))
                                            .h(px(4.0))
                                            .rounded_full()
                                            .bg(colors.hover)
                                            .child(
                                                div()
                                                    .w(px(32.0 * (pct as f32 / 100.0).clamp(0.05, 1.0)))
                                                    .h_full()
                                                    .rounded_full()
                                                    .bg(if pct > 80 {
                                                        colors.danger
                                                    } else if pct > 50 {
                                                        colors.warning
                                                    } else {
                                                        colors.accent
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                            .text_color(colors.fg_subtle)
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
                                        IconButton::new("composer-attach-btn", IconName::Plus)
                                            .size(ControlSize::Md)
                                            .variant(ButtonVariant::Secondary)
                                            .tooltip("Attach file or image"),
                                    )
                                    // Send / Stop action button
                                    .child(
                                        IconButton::new(
                                            "composer-send-action-btn",
                                            if is_running { IconName::Square } else { IconName::ArrowUp },
                                        )
                                        .size(ControlSize::Md)
                                        .variant(if is_running { ButtonVariant::Danger } else { ButtonVariant::Primary })
                                        .tooltip(if is_running { "Stop agent (SIGINT)" } else { "Send prompt (⏎)" })
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
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child("⏎ to send · ⇧⏎ for new line"),
                    ),
            )
    }

    /// One header + option list per harness, in catalog order.
    fn render_model_groups(&self, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let colors = &cx.theme().colors;
        let text_xs = cx.theme().text_size(TextSize::Xs);
        [HarnessKind::Claude, HarnessKind::Antigravity, HarnessKind::Codex, HarnessKind::OpenCode]
            .into_iter()
            .flat_map(|kind| {
                let installed = self.harnesses.iter().any(|h| h.id == kind.id() && h.available);
                let header = div()
                    .pt_1()
                    .px_2()
                    .py_1()
                    .flex()
                    .justify_between()
                    .text_size(text_xs)
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.fg_subtle)
                    .child(kind.label().to_uppercase())
                    .when(!installed, |el| el.child("not installed"))
                    .into_any_element();
                std::iter::once(header).chain(
                    catalog::models_for(kind).map(move |option| self.render_model_option(option, installed, cx).into_any_element()),
                )
            })
            .collect()
    }

    fn render_model_option(&self, option: &'static ModelOption, installed: bool, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let is_sel = self.selected_model == option.key;
        let harness_dot = theme::harness_color(option.harness.id(), colors);

        div()
            .id(SharedString::from(format!("model-option-{}", option.key)))
            .flex()
            .items_center()
            .justify_between()
            .px_2p5()
            .py_1p5()
            .rounded(cx.theme().radius(Radius::Sm))
            .when(!installed, |el| el.opacity(0.45))
            .when(is_sel, |el| el.bg(colors.active))
            .when(installed, |el| {
                el.cursor_pointer()
                    .when(!is_sel, |el| el.hover(|s| s.bg(colors.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_session_model(option.key, cx)))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(6.0)).rounded_full().bg(harness_dot))
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .font_weight(if is_sel { FontWeight::BOLD } else { FontWeight::NORMAL })
                            .text_color(if is_sel { colors.fg } else { colors.fg_muted })
                            .child(option.label),
                    ),
            )
            .when(is_sel, |el| el.child(Icon::new(IconName::Check).size(IconSize::Xs).color(colors.accent)))
    }
}
