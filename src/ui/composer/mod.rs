//! Prompt composer: model / permission / branch menus, context meter, the
//! prompt field with `/` and `@` suggestions, and send / stop.
//! 100% faithful to MonoCode Composer layout.

mod model_picker;
mod suggestions;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::git::{Branch as ElyBranch, BranchSelector};
use ely_gpui_component::motion::ProgressRing;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::workspace_sync::BranchTarget;
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Popover {
    Plus,
    Model,
    Access,
}

/// MonoCode's composer chip: 26px, icon, 11px label, chevron that turns
/// while open.
fn composer_chip(
    id: &'static str,
    icon: gpui::AnyElement,
    label: String,
    open: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let colors = &cx.theme().colors;
    let hover = colors.fg.opacity(0.08);
    div()
        .id(id)
        .h(px(26.0))
        .max_w(px(160.0))
        .min_w_0()
        .px_1p5()
        .rounded(px(6.0))
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .when(open, |el| el.bg(hover))
        .hover(move |s| s.bg(hover))
        .child(icon)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(11.0))
                .text_color(colors.fg.opacity(0.8))
                .child(label),
        )
        .child(
            Icon::new(if open {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .size(IconSize::Xs)
            .color(colors.fg.opacity(0.5)),
        )
}

/// 1234567 -> "1,234,567".
fn group_digits(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// A row of the composer "+" menu.
type PlusAction = fn(&mut BenCodeApp, &mut Context<BenCodeApp>);

/// MonoCode `AccessPicker` rows: mode, label, hint, icon.
const PERMISSION_MODES: [(PermissionMode, &str, &str, IconName); 4] = [
    (
        PermissionMode::Supervised,
        "Supervised",
        "Ask before commands and file changes.",
        IconName::Lock,
    ),
    (
        PermissionMode::AutoAcceptEdits,
        "Auto-accept edits",
        "Auto-approve edits, ask before other actions.",
        IconName::Pencil,
    ),
    (
        PermissionMode::Auto,
        "Auto",
        "An AI reviewer can approve or deny actions.",
        IconName::Sparkles,
    ),
    (
        PermissionMode::FullAccess,
        "Full access",
        "Allow commands, edits, and supported MCP confirmations without prompts.",
        IconName::Shield,
    ),
];

fn permission_entry(mode: PermissionMode) -> (&'static str, IconName) {
    PERMISSION_MODES
        .iter()
        .find(|(m, ..)| *m == mode)
        .map_or(("Supervised", IconName::Lock), |(_, label, _, icon)| {
            (*label, *icon)
        })
}

impl BenCodeApp {
    /// MonoCode `Composer`: an 8px-rounded box holding the project / branch
    /// bar with the context meter, the prompt, and the + / model / access /
    /// send row.
    pub fn render_composer(
        &self,
        session: Option<&SessionRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let running_here = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        let queue = session.and_then(|s| self.render_message_queue(&s.id, cx));
        div()
            .flex_none()
            .px_6()
            .pb_4()
            .pt_2()
            .children(queue.map(|q| div().max_w(COMPOSER_MAX_WIDTH).mx_auto().child(q)))
            .child(
                div()
                    .relative()
                    .max_w(COMPOSER_MAX_WIDTH)
                    .mx_auto()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(
                        colors
                            .fg
                            .opacity(if self.prompt_focused { 0.2 } else { 0.1 }),
                    )
                    .bg(colors.fg.opacity(0.03))
                    .children(self.render_suggestions(cx))
                    .child(self.composer_top_bar(session, cx))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(14.0))
                            .line_height(px(22.0))
                            .child(self.prompt_input.clone()),
                    )
                    .when(self.is_branch_picker_open, |el| {
                        el.child(self.render_branch_picker(cx))
                    })
                    .when(self.is_plus_menu_open, |el| {
                        el.child(self.render_plus_menu_popover(cx))
                    })
                    .when(self.is_model_picker_open, |el| {
                        let key =
                            session.map_or(self.selected_model.as_str(), |s| s.model.as_str());
                        el.child(self.render_model_picker_popover(key, cx))
                    })
                    .when(self.is_permission_picker_open, |el| {
                        el.child(self.render_permission_picker_popover(cx))
                    })
                    .child(self.composer_bottom_bar(session, running_here, cx)),
            )
    }

    /// Project folder, branch, and the context meter at the far right.
    fn composer_top_bar(
        &self,
        session: Option<&SessionRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let project = std::path::Path::new(&self.current_cwd)
            .file_name()
            .map_or_else(
                || self.current_cwd.clone(),
                |n| n.to_string_lossy().into_owned(),
            );
        div()
            .flex()
            .items_center()
            .gap_2p5()
            .px_3()
            .pt_2p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .h(px(24.0))
                    .text_size(px(11.0))
                    .text_color(colors.fg_muted)
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Xs)
                            .color(colors.fg_muted),
                    )
                    .child(div().max_w(px(160.0)).truncate().child(project)),
            )
            .child(self.branch_menu(session, cx))
            .child(div().flex_1())
            .children(session.and_then(|s| self.context_meter(s)))
    }

    /// MonoCode `ContextMeter`: a 14px ring of the window used, with the
    /// numbers on hover.
    fn context_meter(&self, session: &SessionRow) -> Option<impl IntoElement> {
        let used = session.context_used?.max(0);
        let window = session.context_window.filter(|w| *w > 0)?;
        let share = (used as f32 / window as f32).clamp(0.0, 1.0);
        let detail = format!(
            "{}% context used\n{} / {} tokens",
            (share * 100.0).round(),
            group_digits(used),
            group_digits(window)
        );
        Some(
            div()
                .id("composer-context")
                .flex_none()
                .tooltip(Tooltip::text(detail))
                .child(ProgressRing::new("composer-context-ring", share).size(gpui::rems(0.875))),
        )
    }

    /// "+" (add to message), model, access, then Send / Stop.
    fn composer_bottom_bar(
        &self,
        session: Option<&SessionRow>,
        running_here: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let key = session.map_or(self.selected_model.as_str(), |s| s.model.as_str());
        let harness = session
            .map(|s| s.harness.as_str())
            .or_else(|| key.split_once(':').map(|(h, _)| h))
            .unwrap_or("claude")
            .to_string();
        let mode = self.session_permission_mode(session);
        let (perm_label, perm_icon) = permission_entry(mode);
        let perm_color = if mode == PermissionMode::FullAccess {
            cx.theme().colors.warning
        } else {
            cx.theme().colors.fg_muted
        };
        let model = composer_chip(
            "composer-model-chip",
            HarnessIcon::new(&harness).size(px(16.0)).into_any_element(),
            catalog::label_for(key),
            self.is_model_picker_open,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Model, cx)));
        let access = composer_chip(
            "composer-permission-chip",
            Icon::new(perm_icon)
                .size(IconSize::Xs)
                .color(perm_color)
                .into_any_element(),
            perm_label.to_string(),
            self.is_permission_picker_open,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Access, cx)));
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .pb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.plus_button(cx))
                    .child(model)
                    .child(access),
            )
            .child(self.render_send_button(running_here, cx))
    }

    /// MonoCode `ToolButton`: 26px, selection background, a thin plus.
    fn plus_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let hover = colors.fg.opacity(0.15);
        div()
            .id("composer-plus")
            .size(px(26.0))
            .rounded(px(6.0))
            .bg(colors.fg.opacity(0.1))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(Tooltip::text("Add to message"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Plus, cx)))
            .child(
                Icon::new(IconName::Plus)
                    .size(IconSize::Xs)
                    .color(colors.fg.opacity(0.5)),
            )
    }

    /// Opens one composer popover and closes the others.
    fn toggle_composer_popover(&mut self, which: Popover, cx: &mut Context<Self>) {
        if which == Popover::Model {
            self.toggle_model_picker(cx);
            return;
        }
        let open = match which {
            Popover::Plus => !self.is_plus_menu_open,
            Popover::Model => !self.is_model_picker_open,
            Popover::Access => !self.is_permission_picker_open,
        };
        self.is_plus_menu_open = open && which == Popover::Plus;
        self.is_model_picker_open = open && which == Popover::Model;
        self.is_permission_picker_open = open && which == Popover::Access;
        self.is_branch_picker_open = false;
        cx.notify();
    }

    /// Send / Stop, as MonoCode's `ComposerAction`: a 26px light button with
    /// a dark arrow, or a filled square to stop. While this thread runs an
    /// empty composer offers Stop; typing turns it back into Send (queue).
    fn render_send_button(&self, running_here: bool, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let typed = !self.prompt_input.read(cx).text().trim().is_empty();
        let stop = running_here && !typed;
        let enabled = running_here || typed;
        let (bg, ink) = if enabled {
            (colors.fg, colors.bg)
        } else {
            (colors.fg.opacity(0.3), colors.bg.opacity(0.4))
        };
        let tooltip = match (running_here, typed) {
            (true, false) => "Stop",
            (true, true) => "Queue message (↩)",
            _ => "Send (↩)",
        };
        let hover = colors.fg.opacity(0.9);
        div()
            .id("composer-send-btn")
            .size(px(26.0))
            .flex_none()
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(bg)
            .tooltip(Tooltip::text(tooltip))
            .when(enabled, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.handle_send_or_stop(cx)))
            })
            .child(if stop {
                div()
                    .size(px(10.0))
                    .rounded(px(2.0))
                    .bg(ink)
                    .into_any_element()
            } else {
                Icon::new(IconName::ArrowUp)
                    .size(IconSize::Xs)
                    .color(ink)
                    .into_any_element()
            })
    }

    /// Messages waiting for the running turn (MonoCode `MessageQueue`).
    fn render_message_queue(&self, session_id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let queued = self.queued_prompts(session_id);
        if queued.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let rows = queued.iter().enumerate().map(|(ix, text)| {
            let session = session_id.to_string();
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .min_h(px(28.0))
                .when(ix > 0, |el| el.border_t_1().border_color(colors.border))
                .child(
                    Icon::new(IconName::CornerDownRight)
                        .size(IconSize::Sm)
                        .color(colors.fg_muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(colors.fg_muted)
                        .child(text.clone()),
                )
                .child(
                    IconButton::new(
                        SharedString::from(format!("queue-remove-{ix}")),
                        IconName::Trash2,
                    )
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Remove from queue")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_queued_prompt(&session, ix, cx)
                    })),
                )
        });
        Some(
            div()
                .mx_2()
                .px_2()
                .py_1()
                .rounded_t(px(10.0))
                .border_1()
                .border_b_0()
                .border_color(colors.border)
                .bg(colors.surface)
                .children(rows)
                .into_any_element(),
        )
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
        let colors = &cx.theme().colors;
        let session = self.selected_session();
        let current_mode = self.session_permission_mode(session);
        let busy = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        div()
            .id("composer-permission-popover")
            .absolute()
            .bottom(px(36.0))
            .left(px(140.0))
            .w(px(288.0))
            .p_1()
            .rounded(px(8.0))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .children(PERMISSION_MODES.iter().map(|&(mode, label, hint, icon)| {
                let icon_color = if mode == PermissionMode::FullAccess {
                    colors.warning
                } else {
                    colors.fg_muted
                };
                div()
                    .id(SharedString::from(format!("perm-opt-{}", mode.id())))
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(mode == current_mode, |el| el.bg(colors.active))
                    .hover(|s| s.bg(colors.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.is_permission_picker_open = false;
                        this.set_permission_mode(mode, cx);
                    }))
                    .child(Icon::new(icon).size(IconSize::Sm).color(icon_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors.fg)
                                    .child(label),
                            )
                            .child(div().text_size(px(11.0)).text_color(colors.fg_muted).child(hint)),
                    )
            }))
            .when(busy, |el| {
                el.child(
                    div()
                        .px_2()
                        .py_1p5()
                        .text_size(px(11.0))
                        .text_color(colors.fg_muted)
                        .child("Access changes apply to the next turn. Stop and resend to apply them now."),
                )
            })
    }

    /// The branch chip; opens Ely's `BranchSelector` (MonoCode `BranchPicker`).
    fn branch_menu(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let current = session
            .and_then(|s| s.branch.clone())
            .or_else(|| Some(self.git_status.branch.clone()).filter(|b| !b.is_empty()))
            .unwrap_or_else(|| "main".to_string());
        div()
            .id("composer-branch")
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .h(px(24.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .text_color(colors.fg_muted)
            .hover(|s| s.bg(colors.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.is_branch_picker_open = !this.is_branch_picker_open;
                cx.notify();
            }))
            .child(
                Icon::new(IconName::GitBranch)
                    .size(IconSize::Xs)
                    .color(colors.fg_muted),
            )
            .child(div().max_w(px(160.0)).truncate().child(current))
    }

    fn render_branch_picker(&self, cx: &Context<Self>) -> impl IntoElement {
        let branches: Vec<ElyBranch> = self
            .workspace
            .branches
            .iter()
            .map(|b| ElyBranch {
                name: b.name.clone().into(),
                remote: b.remote,
                current: b.current,
                ahead: 0,
                behind: 0,
                subject: SharedString::default(),
                when: SharedString::default(),
            })
            .collect();
        let close = app_callback(cx, |this, cx| {
            this.is_branch_picker_open = false;
            cx.notify();
        });
        let entity = cx.entity().downgrade();
        let create_entity = entity.clone();
        let known = self.workspace.branches.clone();
        div()
            .absolute()
            .bottom(px(36.0))
            .left(px(8.0))
            .w(px(280.0))
            .child(
                BranchSelector::new("composer-branch-picker", branches, close)
                    .on_pick(move |name, _, cx| {
                        let Some(branch) = known
                            .iter()
                            .find(|b| b.name.as_str() == name.as_ref())
                            .cloned()
                        else {
                            return;
                        };
                        if let Err(err) = entity.update(cx, |this, cx| {
                            this.switch_to_branch(BranchTarget::Existing(branch), false, cx)
                        }) {
                            log::debug!("branch pick after app drop: {err:#}");
                        }
                    })
                    .on_create(move |name, _, cx| {
                        let target = BranchTarget::New(name.to_string());
                        if let Err(err) = create_entity
                            .update(cx, |this, cx| this.switch_to_branch(target, false, cx))
                        {
                            log::debug!("branch create after app drop: {err:#}");
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_group_in_thousands() {
        assert_eq!(group_digits(701_285), "701,285");
        assert_eq!(group_digits(1_000_000), "1,000,000");
        assert_eq!(group_digits(12), "12");
    }

    #[test]
    fn every_permission_mode_has_a_menu_entry() {
        for mode in PermissionMode::ALL {
            assert!(PERMISSION_MODES.iter().any(|(m, ..)| *m == mode));
        }
        assert_eq!(
            permission_entry(PermissionMode::FullAccess).0,
            "Full access"
        );
    }
}
