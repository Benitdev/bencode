//! Prompt composer: model / permission / branch menus, context meter, the
//! prompt field with `/` and `@` suggestions, and send / stop.

mod suggestions;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::chat::TokenCounter;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem, SearchableMenu};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, Radius, TextSize};
use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::harness::{HarnessKind, catalog};
use crate::ui::app_callback::app_callback;

const COMPOSER_MAX_WIDTH: gpui::Pixels = px(840.0);
const HARNESS_ORDER: [HarnessKind; 4] =
    [HarnessKind::Claude, HarnessKind::Antigravity, HarnessKind::Codex, HarnessKind::OpenCode];
const PERMISSION_MODES: [(PermissionMode, &str, IconName); 3] = [
    (PermissionMode::Auto, "Auto-approve", IconName::Zap),
    (PermissionMode::Confirm, "Ask first", IconName::Shield),
    (PermissionMode::ReadOnly, "Read-only", IconName::Eye),
];

fn permission_entry(mode: PermissionMode) -> (&'static str, IconName) {
    PERMISSION_MODES
        .iter()
        .find(|(m, ..)| *m == mode)
        .map_or(("Auto-approve", IconName::Zap), |(_, label, icon)| (*label, *icon))
}

impl BenCodeApp {
    pub fn render_composer(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let running_here = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        div().flex_none().px_6().pb_4().pt_2().child(
            div()
                .relative()
                .max_w(COMPOSER_MAX_WIDTH)
                .mx_auto()
                .rounded(theme.radius(Radius::Lg))
                .border_1()
                .border_color(if running_here { theme.colors.accent } else { theme.colors.border })
                .bg(theme.colors.surface)
                .children(self.render_suggestions(cx))
                .child(self.render_composer_toolbar(session, cx))
                .child(self.render_prompt_row(running_here, cx)),
        )
    }

    fn render_composer_toolbar(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let usage = session.and_then(|s| {
            Some((usize::try_from(s.context_used?).ok()?, usize::try_from(s.context_window?).ok()?))
        });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.model_menu(session, cx))
                    .child(self.permission_menu(cx))
                    .child(self.branch_menu(session, cx)),
            )
            .when_some(usage.filter(|(_, limit)| *limit > 0), |el, (used, limit)| {
                el.child(div().text_size(theme.text_size(TextSize::Xs)).child(TokenCounter::new(used).limit(limit)))
            })
    }

    fn model_menu(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let current = session.map_or(self.selected_model.as_str(), |s| s.model.as_str());
        let menu = HARNESS_ORDER.iter().fold(Menu::new(), |menu, &kind| {
            let installed = self.harnesses.iter().any(|h| h.id == kind.id() && h.available);
            let title = if installed { kind.label().to_string() } else { format!("{} (not installed)", kind.label()) };
            menu.group(
                title,
                catalog::models_for(kind).map(|option| {
                    let key = option.key;
                    MenuItem::radio(option.label, option.key == current)
                        .disabled(!installed)
                        .on_click(app_callback(cx, move |this, cx| this.set_session_model(key, cx)))
                }),
            )
        });
        DropdownMenu::new("composer-model", catalog::label_for(current), menu).variant(ButtonVariant::Ghost)
    }

    fn permission_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let (label, icon) = permission_entry(self.permission_mode);
        let menu = PERMISSION_MODES.iter().fold(Menu::new(), |menu, &(mode, label, icon)| {
            menu.item(
                MenuItem::radio(label, mode == self.permission_mode)
                    .icon(icon)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.permission_mode = mode;
                        cx.notify();
                    })),
            )
        });
        DropdownMenu::new("composer-permission", label, menu).icon(icon).variant(ButtonVariant::Ghost)
    }

    fn branch_menu(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let current = session.and_then(|s| s.branch.clone()).unwrap_or_else(|| self.git_status.branch.clone());
        let menu = self.workspace.branches.iter().fold(Menu::new(), |menu, branch| {
            let name = branch.clone();
            menu.item(
                MenuItem::radio(branch.clone(), *branch == current)
                    .on_click(app_callback(cx, move |this, cx| this.set_session_branch(name.clone(), cx))),
            )
        });
        SearchableMenu::new("composer-branch", current, menu).icon(IconName::GitBranch).placeholder("Find a branch")
    }

    fn render_prompt_row(&self, running_here: bool, cx: &Context<Self>) -> impl IntoElement {
        let busy_elsewhere = self.is_agent_running() && !running_here;
        let (icon, variant, tooltip) = if running_here {
            (IconName::Square, ButtonVariant::Danger, "Stop the agent")
        } else {
            (IconName::ArrowUp, ButtonVariant::Primary, "Send (⏎) · new line (⇧⏎)")
        };
        div()
            .flex()
            .items_end()
            .gap_2()
            .p_2()
            .child(div().flex_1().min_w_0().px_1().child(self.prompt_input.clone()))
            .child(
                IconButton::new("composer-send", icon)
                    .variant(variant)
                    .size(ControlSize::Md)
                    .disabled(busy_elsewhere)
                    .tooltip(if busy_elsewhere { "Another thread's agent is running" } else { tooltip })
                    .on_click(cx.listener(|this, _, _, cx| this.handle_send_or_stop(cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_mode_has_a_menu_entry() {
        for mode in [PermissionMode::Auto, PermissionMode::Confirm, PermissionMode::ReadOnly] {
            assert!(PERMISSION_MODES.iter().any(|(m, ..)| *m == mode));
        }
        assert_eq!(permission_entry(PermissionMode::ReadOnly).0, "Read-only");
    }
}
