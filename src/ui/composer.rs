use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_composer(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_running = self.is_agent_running;
        let model_name = session
            .map(|s| {
                if s.model.is_empty() {
                    self.selected_model.as_str()
                } else {
                    s.model.as_str()
                }
            })
            .unwrap_or(self.selected_model.as_str());
        let branch_name = session
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("main");

        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
        let harness_dot_color = match harness {
            "claude" => MonoTheme::claude_orange(),
            "antigravity" => MonoTheme::antigravity_blue(),
            "codex" => MonoTheme::codex_green(),
            "cursor" => MonoTheme::accent(),
            _ => MonoTheme::accent(),
        };

        let context_label = match (session.and_then(|s| s.context_used), session.and_then(|s| s.context_window)) {
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
                let pct = (used as f64 / window as f64 * 100.0).round() as u64;
                format!("{} / {} tokens ({}%)", used_str, win_str, pct)
            }
            _ => "0 / 200k tokens".to_string(),
        };

        let perm_mode = self.permission_mode;
        let (perm_icon, perm_label) = match perm_mode {
            PermissionMode::Auto => ("⚡", "Auto"),
            PermissionMode::Confirm => ("🛡", "Confirm"),
            PermissionMode::ReadOnly => ("🔒", "Read-Only"),
        };

        div()
            .px_6()
            .pb_5()
            .pt_2()
            .bg(MonoTheme::bg_base())
            .child(
                // Floating Composer Card
                div()
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
                                                    .child("▾"),
                                            ),
                                    )
                                    // Permission Mode Chip (⚡ Auto / 🛡 Confirm / 🔒 Read-Only)
                                    .child(
                                        div()
                                            .id("composer-perm-mode-picker")
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
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .child(format!("{} {}", perm_icon, perm_label))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.permission_mode = match this.permission_mode {
                                                    PermissionMode::Auto => PermissionMode::Confirm,
                                                    PermissionMode::Confirm => PermissionMode::ReadOnly,
                                                    PermissionMode::ReadOnly => PermissionMode::Auto,
                                                };
                                                cx.notify();
                                            })),
                                    )
                                    // Branch Pill
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .child(format!("⎇ {}", branch_name)),
                                    ),
                            )
                            // Right Side: Context Meter
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(context_label),
                            ),
                    )
                    .child(
                        // 2. Interactive Input Prompt Text Area
                        div()
                            .p_3()
                            .child(
                                div()
                                    .min_h(px(46.0))
                                    .px_1()
                                    .child(self.prompt_input.clone()),
                            ),
                    )
                    .child(
                        // 3. Bottom Action Bar: Skills / Files / Attachments / Send
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .pb_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    // Skill Picker Pill
                                    .child(
                                        div()
                                            .id("composer-skills-chip")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_base())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::skill_gold())
                                            .child("/ Skills")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.prompt_input.update(cx, |input, cx| {
                                                    let current = input.text().to_string();
                                                    input.set_text(format!("{}/", current), cx);
                                                });
                                            })),
                                    )
                                    // File Mention Pill
                                    .child(
                                        div()
                                            .id("composer-mention-chip")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_base())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::mention_cyan())
                                            .child("@ Files")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.prompt_input.update(cx, |input, cx| {
                                                    let current = input.text().to_string();
                                                    input.set_text(format!("{}@", current), cx);
                                                });
                                            })),
                                    )
                                    // Attachment button (+)
                                    .child(
                                        div()
                                            .id("composer-attach-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(26.0))
                                            .rounded(theme.radius(Radius::Sm))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_base())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .text_color(MonoTheme::fg_muted())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .child("+"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    // Keyboard hints
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child("↵ to send · ⇧↵ for newline"),
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
                    ),
            )
    }
}
