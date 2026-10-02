//! Prompt composer: model / permission / branch menus, context meter, the
//! prompt field with `/` and `@` suggestions, and send / stop.
//! 100% faithful to MonoCode Composer layout.

mod suggestions;
pub use suggestions::{mention_suggestions, skill_suggestions};

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::chat::TokenCounter;
use ely_gpui_component::menus::{Menu, MenuItem, SearchableMenu};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, rgb,
};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::harness::{HarnessKind, catalog};
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;

const COMPOSER_MAX_WIDTH: gpui::Pixels = px(840.0);
const HARNESS_ORDER: [HarnessKind; 4] = [
    HarnessKind::Claude,
    HarnessKind::Antigravity,
    HarnessKind::Codex,
    HarnessKind::OpenCode,
];
/// A row of the composer "+" menu.
type PlusAction = fn(&mut BenCodeApp, &mut Context<BenCodeApp>);

const PERMISSION_MODES: [(PermissionMode, &str, &str, IconName); 3] = [
    (
        PermissionMode::Auto,
        "Full access",
        "Allow commands, edits, and confirmations without prompts.",
        IconName::Shield,
    ),
    (
        PermissionMode::Confirm,
        "Supervised",
        "Ask before commands and file changes.",
        IconName::Lock,
    ),
    (
        PermissionMode::ReadOnly,
        "Read-only",
        "Read-only inspection without file changes.",
        IconName::Eye,
    ),
];

fn permission_entry(mode: PermissionMode) -> (&'static str, IconName) {
    PERMISSION_MODES
        .iter()
        .find(|(m, ..)| *m == mode)
        .map_or(("Full access", IconName::Shield), |(_, label, _, icon)| {
            (*label, *icon)
        })
}

impl BenCodeApp {
    pub fn render_composer(
        &self,
        session: Option<&SessionRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let running_here = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        let usage = session.and_then(|s| {
            Some((
                usize::try_from(s.context_used?).ok()?,
                usize::try_from(s.context_window?).ok()?,
            ))
        });

        let current_model_key = session.map_or(self.selected_model.as_str(), |s| s.model.as_str());
        let current_model_label = catalog::label_for(current_model_key);
        let current_harness = session
            .map(|s| s.harness.as_str())
            .or_else(|| current_model_key.split_once(':').map(|(h, _)| h))
            .unwrap_or("claude");
        let (perm_label, perm_icon) = permission_entry(self.permission_mode);

        div().flex_none().px_6().pb_4().pt_2().child(
            div()
                .relative()
                .max_w(COMPOSER_MAX_WIDTH)
                .mx_auto()
                .rounded(px(10.0))
                .border_1()
                .border_color(if running_here {
                    colors.accent
                } else {
                    colors.border
                })
                .bg(rgb(0x161420))
                .children(self.render_suggestions(cx))
                // 1. Top row inside card: Folder "Current checkout" + Git branch
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .pt_2p5()
                        .pb_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(12.0))
                                .text_color(rgb(0x8e8a9d))
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(IconSize::Xs)
                                        .color(rgb(0x8e8a9d)),
                                )
                                .child("Current checkout"),
                        )
                        .child(self.branch_menu(session, cx))
                        .when_some(
                            usage.filter(|(_, limit)| *limit > 0),
                            |el, (used, limit)| {
                                el.child(
                                    div()
                                        .ml_auto()
                                        .text_size(px(11.0))
                                        .child(TokenCounter::new(used).limit(limit)),
                                )
                            },
                        ),
                )
                // 2. Middle row: Text input area
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(13.5))
                        .line_height(px(20.0))
                        .child(self.prompt_input.clone()),
                )
                // 3. Floating Popovers when open
                .when(self.is_plus_menu_open, |el| {
                    el.child(self.render_plus_menu_popover(cx))
                })
                .when(self.is_model_picker_open, |el| {
                    el.child(self.render_model_picker_popover(current_model_key, cx))
                })
                .when(self.is_permission_picker_open, |el| {
                    el.child(self.render_permission_picker_popover(cx))
                })
                // 4. Bottom toolbar: [+] [Model chip ∨] [Permission chip ∨] ... [^ Send / Stop]
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .pb_2()
                        .pt_1()
                        // Left group: + button, Model chip, Permission chip
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                // [+] Button
                                .child(
                                    div()
                                        .id("composer-plus")
                                        .size(px(26.0))
                                        .rounded(px(6.0))
                                        .bg(rgb(0x232030))
                                        .border_1()
                                        .border_color(gpui::rgba(0xffffff10))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgb(0x2c293c)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.is_plus_menu_open = !this.is_plus_menu_open;
                                            this.is_model_picker_open = false;
                                            this.is_permission_picker_open = false;
                                            cx.notify();
                                        }))
                                        .child(
                                            Icon::new(IconName::Plus)
                                                .size(IconSize::Xs)
                                                .color(rgb(0x8e8a9d)),
                                        ),
                                )
                                // Model Chip
                                .child(
                                    div()
                                        .id("composer-model-chip")
                                        .h(px(26.0))
                                        .max_w(px(220.0))
                                        .min_w_0()
                                        .px_2()
                                        .rounded(px(6.0))
                                        .bg(rgb(0x232030))
                                        .border_1()
                                        .border_color(gpui::rgba(0xffffff10))
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgb(0x2c293c)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.is_model_picker_open = !this.is_model_picker_open;
                                            this.is_permission_picker_open = false;
                                            this.is_plus_menu_open = false;
                                            cx.notify();
                                        }))
                                        .child(HarnessIcon::new(current_harness).size(px(14.0)))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_size(px(11.0))
                                                .text_color(rgb(0xe2e0ea))
                                                .truncate()
                                                .child(current_model_label),
                                        )
                                        .child(
                                            Icon::new(IconName::ChevronDown)
                                                .size(IconSize::Xs)
                                                .color(gpui::rgba(0xffffff66)),
                                        ),
                                )
                                // Permission Chip
                                .child(
                                    div()
                                        .id("composer-permission-chip")
                                        .h(px(26.0))
                                        .max_w(px(200.0))
                                        .min_w_0()
                                        .px_2()
                                        .rounded(px(6.0))
                                        .bg(rgb(0x232030))
                                        .border_1()
                                        .border_color(gpui::rgba(0xffffff10))
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgb(0x2c293c)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.is_permission_picker_open =
                                                !this.is_permission_picker_open;
                                            this.is_model_picker_open = false;
                                            this.is_plus_menu_open = false;
                                            cx.notify();
                                        }))
                                        .child(
                                            Icon::new(perm_icon)
                                                .size(IconSize::Xs)
                                                .color(rgb(0xf59e0b)),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_size(px(11.0))
                                                .text_color(rgb(0xe2e0ea))
                                                .truncate()
                                                .child(perm_label),
                                        )
                                        .child(
                                            Icon::new(IconName::ChevronDown)
                                                .size(IconSize::Xs)
                                                .color(gpui::rgba(0xffffff66)),
                                        ),
                                ),
                        )
                        // Right group: Send / Stop button
                        .child(self.render_send_button(running_here, cx)),
                ),
        )
    }

    /// Send / Stop. While another thread's agent runs, sending here would be
    /// dropped by the global run lock, so the button is disabled and says why.
    fn render_send_button(&self, running_here: bool, cx: &Context<Self>) -> IconButton {
        let blocked = !running_here && self.is_agent_running();
        let (icon, variant, tooltip) = if running_here {
            (IconName::Square, ButtonVariant::Secondary, "Stop the agent")
        } else if blocked {
            (
                IconName::ArrowUp,
                ButtonVariant::Primary,
                "Another thread's agent is running. Wait for it or stop it first.",
            )
        } else {
            (IconName::ArrowUp, ButtonVariant::Primary, "Send (↩)")
        };
        IconButton::new("composer-send-btn", icon)
            .size(ControlSize::Sm)
            .variant(variant)
            .disabled(blocked)
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, _, _, cx| {
                // Never let a click here stop another thread's run.
                if !blocked {
                    this.handle_send_or_stop(cx);
                }
            }))
    }

    fn render_model_picker_popover(
        &self,
        current_model_key: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let current_key = current_model_key.to_string();

        div()
            .id("composer-model-popover")
            .absolute()
            .bottom(px(36.0))
            .left(px(34.0))
            .w(px(240.0))
            .max_h(px(300.0))
            .overflow_y_scroll()
            .p_1p5()
            .rounded(px(8.0))
            .bg(rgb(0x1a1824))
            .border_1()
            .border_color(gpui::rgba(0xffffff18))
            .shadow_lg()
            .children(HARNESS_ORDER.iter().map(|&kind| {
                let models: Vec<_> = catalog::models_for(kind).collect();
                let label = kind.label();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0x8e8a9d))
                            .child(HarnessIcon::new(kind.id()).size(px(12.0)))
                            .child(label),
                    )
                    .children(models.into_iter().map(|option| {
                        let is_active = option.key == current_key;
                        let key = option.key;
                        let opt_label = option.label;
                        div()
                            .id(SharedString::from(format!("model-opt-{key}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .rounded(px(5.0))
                            .cursor_pointer()
                            .when(is_active, |el| el.bg(rgb(0x2c293c)))
                            .hover(|s| s.bg(rgb(0x252233)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_session_model(key, cx);
                                this.is_model_picker_open = false;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(HarnessIcon::new(kind.id()).size(px(13.0)))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(if is_active {
                                                rgb(0xffffff)
                                            } else {
                                                rgb(0xdedce6)
                                            })
                                            .child(opt_label),
                                    ),
                            )
                            .when(is_active, |el| {
                                el.child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Xs)
                                        .color(rgb(0x388bfd)),
                                )
                            })
                    }))
            }))
    }

    /// MonoCode's "ADD TO MESSAGE" menu. Only actions BenCode can honour are
    /// listed; Plan mode and Draft need harness/DB support first.
    fn render_plus_menu_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let reference: PlusAction = |this, cx| {
            this.append_to_prompt("@", cx);
            this.is_mention_picker_open = true;
            this.mention_query = String::new();
        };
        let recall: PlusAction = |this, cx| this.recall_last_turn(cx);
        let items = [
            (
                "plus-reference",
                IconName::FilePlus,
                "Reference a file",
                "Add an @file to the message",
                reference,
            ),
            (
                "plus-recall",
                IconName::RotateCcw,
                "Recall last prompt",
                "Put your previous message back",
                recall,
            ),
        ];
        div()
            .id("composer-plus-popover")
            .absolute()
            .bottom(px(36.0))
            .left(px(8.0))
            .w(px(250.0))
            .p_1p5()
            .rounded(px(8.0))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_0p5()
                    .text_size(px(10.0))
                    .text_color(colors.fg_subtle)
                    .child("ADD TO MESSAGE"),
            )
            .children(items.into_iter().map(|(id, icon, title, hint, action)| {
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .px_2()
                    .py_2()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.is_plus_menu_open = false;
                        action(this, cx);
                        cx.notify();
                    }))
                    .child(Icon::new(icon).size(IconSize::Sm).color(colors.fg_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.0)).text_color(colors.fg).child(title))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(colors.fg_muted)
                                    .truncate()
                                    .child(hint),
                            ),
                    )
            }))
    }

    fn render_permission_picker_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        let current_mode = self.permission_mode;

        div()
            .id("composer-permission-popover")
            .absolute()
            .bottom(px(36.0))
            .left(px(140.0))
            .w(px(280.0))
            .p_1p5()
            .rounded(px(8.0))
            .bg(rgb(0x1a1824))
            .border_1()
            .border_color(gpui::rgba(0xffffff18))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_0p5()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x8e8a9d))
                    .child("EXECUTION ACCESS"),
            )
            .children(PERMISSION_MODES.iter().map(|&(mode, label, hint, icon)| {
                let is_active = mode == current_mode;
                div()
                    .id(SharedString::from(format!("perm-opt-{mode:?}")))
                    .flex()
                    .items_start()
                    .justify_between()
                    .px_2()
                    .py_2()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .when(is_active, |el| el.bg(rgb(0x2c293c)))
                    .hover(|s| s.bg(rgb(0x252233)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.is_permission_picker_open = false;
                        this.set_permission_mode(mode, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2p5()
                            .flex_1()
                            .min_w_0()
                            .child(Icon::new(icon).size(IconSize::Xs).color(
                                if mode == PermissionMode::Auto {
                                    rgb(0xf59e0b)
                                } else {
                                    rgb(0x8e8a9d)
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(if is_active {
                                                rgb(0xffffff)
                                            } else {
                                                rgb(0xdedce6)
                                            })
                                            .child(label),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(0x8e8a9d))
                                            .child(hint),
                                    ),
                            ),
                    )
                    .when(is_active, |el| {
                        el.child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Xs)
                                .color(rgb(0x388bfd)),
                        )
                    })
            }))
    }

    fn branch_menu(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let current = session.and_then(|s| s.branch.clone()).unwrap_or_else(|| {
            if self.git_status.branch.is_empty() {
                "main".to_string()
            } else {
                self.git_status.branch.clone()
            }
        });
        let menu = self
            .workspace
            .branches
            .iter()
            .fold(Menu::new(), |menu, branch| {
                let name = branch.clone();
                menu.item(
                    MenuItem::radio(branch.clone(), *branch == current)
                        .on_click(app_callback(cx, move |this, cx| {
                            this.set_session_branch(name.clone(), cx)
                        })),
                )
            });
        SearchableMenu::new("composer-branch", current, menu)
            .icon(IconName::GitBranch)
            .placeholder("Find a branch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_mode_has_a_menu_entry() {
        for mode in [
            PermissionMode::Auto,
            PermissionMode::Confirm,
            PermissionMode::ReadOnly,
        ] {
            assert!(PERMISSION_MODES.iter().any(|(m, ..)| *m == mode));
        }
        assert_eq!(permission_entry(PermissionMode::ReadOnly).0, "Read-only");
    }
}
