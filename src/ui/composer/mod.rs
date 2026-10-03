//! Prompt composer: model / permission / branch menus, context meter, the
//! prompt field with `/` and `@` suggestions, and send / stop.
//! 100% faithful to MonoCode Composer layout.

mod attachments;
mod context_ring;
pub mod mentions;
mod menus;
pub mod mode_commands;
mod model_picker;
mod suggestions;

use ely_gpui_component::buttons::ButtonVariant;
use ely_gpui_component::git::{Branch as ElyBranch, BranchSelector};
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
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
pub use menus::{MenuState, focus_later};
use menus::{popover_anchor, popover_surface};

/// MonoCode `max-w-4xl`, the same column as the transcript.
const COMPOSER_MAX_WIDTH: gpui::Pixels = px(896.0);
/// MonoCode's default placeholder (trailing space included).
pub const PROMPT_PLACEHOLDER: &str = "Ask, build, / for commands, @ for references... ";
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

/// MonoCode's toolbar chip (`ModelPicker` / `AccessPicker` trigger):
/// `h-6.5 rounded-md px-1.5 gap-1 bg-selection hover:bg-selection-hover`,
/// an icon, an 11px label, an optional dimmed detail (the model's effort),
/// and a chevron that turns over while the menu is open.
fn composer_chip(
    id: &'static str,
    icon: gpui::AnyElement,
    label: String,
    detail: Option<String>,
    open: bool,
    max_width: f32,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let colors = &cx.theme().colors;
    let (fill, hover) = (selection(cx), selection_hover(cx));
    div()
        .id(id)
        .h(px(26.0))
        .max_w(px(max_width))
        .min_w_0()
        .flex_none()
        .px_1p5()
        .rounded(px(6.0))
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .text_color(colors.fg)
        .bg(fill)
        .hover(move |s| s.bg(hover))
        .child(div().flex_none().child(icon))
        .child(div().min_w_0().truncate().text_size(px(11.0)).child(label))
        .children(detail.map(|detail| {
            div()
                .flex_none()
                .text_size(px(11.0))
                .text_color(colors.fg.opacity(0.5))
                .child(detail)
        }))
        .child(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::Xs)
                .color(colors.fg.opacity(0.5))
                .when(open, |icon| {
                    icon.rotate(gpui::radians(std::f32::consts::PI))
                }),
        )
}

/// MonoCode `bg-selection`: the text colour at 10% (6% in light mode).
fn selection(cx: &gpui::App) -> gpui::Hsla {
    let colors = &cx.theme().colors;
    colors
        .fg
        .opacity(if cx.theme().is_dark() { 0.10 } else { 0.06 })
}

/// MonoCode `bg-selection-hover`: 15% (10% in light mode).
fn selection_hover(cx: &gpui::App) -> gpui::Hsla {
    let colors = &cx.theme().colors;
    colors
        .fg
        .opacity(if cx.theme().is_dark() { 0.15 } else { 0.10 })
}

/// MonoCode `bg-selection-emphasis`: 20% (14% in light mode).
fn selection_emphasis(cx: &gpui::App) -> gpui::Hsla {
    let colors = &cx.theme().colors;
    colors
        .fg
        .opacity(if cx.theme().is_dark() { 0.20 } else { 0.14 })
}

/// MonoCode `GitPickerTrigger`: the top bar's 12px workspace and branch
/// buttons, dim until hovered.
fn git_trigger(
    id: &'static str,
    icon: IconName,
    label: String,
    enabled: bool,
    open: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    git_label(id, icon, label, Some(enabled), open, cx)
}

/// `GitPickerTrigger` when `enabled` is set, else MonoCode's static
/// `WorkspaceIdentity` label (`text-content/45`, no hover).
fn git_label(
    id: &'static str,
    icon: IconName,
    label: String,
    enabled: Option<bool>,
    open: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let interactive = enabled.is_some();
    let enabled = enabled.unwrap_or(true);
    let colors = &cx.theme().colors;
    let (fg, hover) = (colors.fg, colors.fg.opacity(0.08));
    div()
        .id(id)
        .ml(px(-6.0))
        .flex()
        .flex_none()
        .max_w(px(256.0))
        .items_center()
        .gap_1p5()
        .h(px(24.0))
        .px_1p5()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .text_color(fg.opacity(if interactive { 0.55 } else { 0.45 }))
        .when(open, |el| el.bg(hover).text_color(fg))
        .when(enabled && interactive, |el| {
            el.cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(fg))
        })
        .when(!enabled, |el| el.opacity(0.4))
        .child(Icon::new(icon).size(IconSize::Xs).color(fg.opacity(0.55)))
        .child(div().min_w_0().truncate().child(label))
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

/// What the prompt paints as you type (MonoCode's highlight layer): a
/// leading `/plan` in the plan colour, `/draft` dimmed, known `/skill`
/// words in the skill colour, and known `@file` labels in the mention
/// colour.
pub fn prompt_highlights(
    text: &str,
    mentions: &mentions::MentionIndex,
    skills: &[String],
    cx: &gpui::App,
) -> Vec<(std::ops::Range<usize>, ely_gpui_component::forms::Highlight)> {
    use ely_gpui_component::forms::Highlight;
    let colors = &cx.theme().colors;
    let mut spans: Vec<(std::ops::Range<usize>, Highlight)> = Vec::new();
    if let Some((mode, range)) = mode_commands::leading_mode(text) {
        let color = match mode {
            mode_commands::ModeCommand::Plan => colors.warning.opacity(0.9),
            mode_commands::ModeCommand::Draft => colors.fg.opacity(0.7),
        };
        spans.push((range, Highlight::new(color)));
    }
    let skill = Highlight::new(colors.warning);
    spans.extend(
        mode_commands::skill_tokens(text, skills)
            .into_iter()
            .map(|range| (range, skill)),
    );
    let mention = Highlight::new(colors.info);
    spans.extend(
        mentions
            .scan(text)
            .into_iter()
            .map(|(range, _, _)| (range, mention)),
    );
    spans.sort_by_key(|(range, _)| range.start);
    // Overlaps would confuse the field; the earlier span wins.
    let mut end = 0;
    spans.retain(|(range, _)| {
        let keep = range.start >= end;
        if keep {
            end = range.end;
        }
        keep
    });
    spans
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
        self.render_composer_view(session, true, false, cx)
    }

    /// The composer of a pane. `focused` is the one holding the prompt; the
    /// others show their thread's draft and focus the pane on a click, as
    /// each MonoCode pane keeps its own composer. `shell` is the centred
    /// composer of a new thread (`py-4` field).
    pub fn render_composer_view(
        &self,
        session: Option<&SessionRow>,
        focused: bool,
        shell: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let running_here = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        let queue = session.and_then(|s| self.render_message_queue(&s.id, cx));
        // MonoCode `fileDrag`: files held over this thread's pane.
        let file_drag =
            session.is_some_and(|s| self.active_file_drop_target.as_deref() == Some(s.id.as_str()));
        let field_pad = if shell { px(16.0) } else { px(12.0) };
        let field = if focused {
            self.prompt_input.clone().into_any_element()
        } else {
            // Another pane's draft, as it was left.
            let draft = session
                .and_then(|s| self.drafts.get(&s.id))
                .filter(|d| !d.is_empty());
            div()
                .min_h(px(22.0))
                .max_h(px(160.0))
                .overflow_hidden()
                .text_color(if draft.is_some() {
                    colors.fg
                } else {
                    colors.fg.opacity(0.4)
                })
                .child(
                    draft
                        .cloned()
                        .unwrap_or_else(|| PROMPT_PLACEHOLDER.to_string()),
                )
                .into_any_element()
        };
        let focus_id = session.map(|s| s.id.clone());
        div()
            .id(SharedString::from(format!(
                "composer-{}",
                session.map_or("none", |s| s.id.as_str())
            )))
            .flex_none()
            .w_full()
            .max_w(COMPOSER_MAX_WIDTH)
            .mx_auto()
            .px(px(6.0))
            .pb(px(6.0))
            .children(queue.filter(|_| focused))
            .child(
                div()
                    .relative()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(if file_drag {
                        colors.accent.opacity(0.6)
                    } else {
                        colors
                            .fg
                            .opacity(if self.prompt_focused { 0.2 } else { 0.1 })
                    })
                    .bg(colors.fg.opacity(0.03))
                    .when(focused, |el| el.children(self.render_suggestions(cx)))
                    .child(self.composer_top_bar(session, cx))
                    .children(self.render_attachment_chips(cx))
                    .child(
                        div()
                            .px_3()
                            .py(field_pad)
                            .max_h(px(160.0) + field_pad * 2.0)
                            .text_size(px(14.0))
                            .line_height(px(22.0))
                            .child(field),
                    )
                    .when(focused && self.is_branch_picker_open, |el| {
                        el.child(popover_surface(self.render_branch_picker(cx), cx))
                    })
                    .when(focused && self.is_plus_menu_open, |el| {
                        el.child(popover_surface(self.render_plus_menu_popover(cx), cx))
                    })
                    .when(focused && self.is_model_picker_open, |el| {
                        let key =
                            session.map_or(self.selected_model.as_str(), |s| s.model.as_str());
                        el.child(popover_surface(
                            self.render_model_picker_popover(key, cx),
                            cx,
                        ))
                    })
                    .when(focused && self.composer_menus.recent_open, |el| {
                        el.child(popover_surface(self.render_recent_models_popover(cx), cx))
                    })
                    .when(focused && self.is_permission_picker_open, |el| {
                        el.child(self.render_permission_picker_popover(cx))
                    })
                    .child(self.composer_bottom_bar(session, running_here, cx))
                    // A resting pane's composer only wakes its pane.
                    .when(!focused, |el| {
                        el.child(
                            div()
                                .id("composer-wake")
                                .absolute()
                                .inset_0()
                                .cursor_text()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(id) = focus_id.clone() {
                                        this.focus_pane(id, cx);
                                    }
                                })),
                        )
                    })
                    .when(file_drag, |el| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(8.0))
                                .bg(colors.accent.opacity(0.08))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.0))
                                .text_color(colors.fg.opacity(0.7))
                                .child("Drop files to attach"),
                        )
                    }),
            )
    }

    /// MonoCode's top bar: the project only for a new thread outside any
    /// project (`showDeckProjectPicker`), the checkout, the branch, and the
    /// context meter at the far right.
    fn composer_top_bar(
        &self,
        session: Option<&SessionRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let empty = session.is_none_or(|s| !s.blocks.iter().any(|b| b.role == "user"));
        let projectless = session.is_none_or(|s| matches!(s.cwd.trim(), "" | "~"));
        let busy = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2p5()
            .overflow_hidden()
            .px_3()
            .pt_2p5()
            .when(empty && projectless, |el| {
                let project = std::path::Path::new(&self.current_cwd)
                    .file_name()
                    .map_or_else(
                        || self.current_cwd.clone(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                el.child(self.project_picker(&project, cx))
            })
            .child(self.workspace_identity(session, empty && !busy, cx))
            .child(self.branch_menu(session, !busy, cx))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .children(session.and_then(|s| self.context_meter(s, cx))),
            )
    }

    /// MonoCode `CwdPicker`: the project chip opens recent projects and
    /// "Open folder…".
    fn project_picker(&self, project: &str, cx: &Context<Self>) -> impl IntoElement {
        let menu = self
            .recent_projects
            .iter()
            .fold(Menu::new(), |menu, path| {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                let target = path.clone();
                menu.item(
                    MenuItem::radio(name, crate::app::same_project_path(path, &self.current_cwd))
                        .on_click(app_callback(cx, move |this, cx| {
                            this.switch_project(target.clone(), cx)
                        })),
                )
            })
            .separator()
            .item(
                MenuItem::new("Open folder…")
                    .icon(IconName::FolderPlus)
                    .on_click(app_callback(cx, |this, cx| this.open_project_dialog(cx))),
            );
        DropdownMenu::new("composer-project", project.to_string(), menu)
            .variant(ButtonVariant::Ghost)
            .icon(IconName::Folder)
    }

    /// The checkout the thread runs in (MonoCode `WorkspaceIdentity`, and
    /// `WorkspacePicker` while the thread is new): "Current checkout" or
    /// "Worktree". A new thread in a project with worktrees can switch.
    fn workspace_identity(
        &self,
        session: Option<&SessionRow>,
        can_switch: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let in_worktree = session.map_or(self.worktree_focus().is_some(), |s| {
            s.worktree_cwd.as_deref().is_some_and(|w| !w.is_empty())
        });
        let label = if in_worktree {
            "Worktree"
        } else {
            "Current checkout"
        };
        let switchable = can_switch
            && self
                .workspace
                .worktrees
                .iter()
                .any(|w| !w.is_main && !w.missing);
        let open = self.composer_menus.workspace_menu.is_some();
        let trigger = git_label(
            "composer-workspace",
            IconName::Folder,
            label.to_string(),
            switchable.then_some(true),
            open,
            cx,
        )
        .when(switchable, |el| {
            el.on_click(cx.listener(|this, _, _, cx| this.toggle_workspace_menu(cx)))
        });
        div()
            .relative()
            .flex_none()
            .child(popover_anchor(trigger, cx))
            .children(open.then(|| self.render_workspace_menu(cx)))
    }

    /// MonoCode `ContextMeter`: a 14px ring of the window used, with the
    /// numbers on hover.
    fn context_meter(&self, session: &SessionRow, cx: &Context<Self>) -> Option<impl IntoElement> {
        let used = session.context_used?.max(0);
        let window = session.context_window.filter(|w| *w > 0)?;
        let share = (used as f32 / window as f32).clamp(0.0, 1.0);
        let count = |n: i64| crate::ui::transcript::turns::format_metric_count(n as f64);
        let headline = format!("{}% context used", (share * 100.0).round());
        let detail = format!("{} / {} tokens", count(used), count(window));
        Some(
            div()
                .id("composer-context")
                .flex_none()
                .tooltip(Tooltip::rich(move |_, cx| {
                    let colors = &cx.theme().colors;
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.0)).child(headline.clone()))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(colors.tooltip_fg.opacity(0.5))
                                .child(detail.clone()),
                        )
                        .into_any_element()
                }))
                .child(context_ring::context_ring(share, cx)),
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
        // MonoCode tints only Full access (`text-amber-400/90`).
        let perm_color = if mode == PermissionMode::FullAccess {
            cx.theme().colors.warning.opacity(0.9)
        } else {
            cx.theme().colors.fg
        };
        let model = composer_chip(
            "composer-model-chip",
            HarnessIcon::new(&harness).size(px(16.0)).into_any_element(),
            catalog::label_for(key),
            catalog::effort_setting(key).map(|effort| {
                effort
                    .value_label(session.map_or(Some(&self.last_model_settings), |s| {
                        s.model_settings.as_ref()
                    }))
                    .to_string()
            }),
            self.is_model_picker_open || self.composer_menus.recent_open,
            160.0,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Model, cx)))
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(|this, _, _, cx| this.toggle_recent_models(cx)),
        );
        let model = popover_anchor(model, cx);
        let access = composer_chip(
            "composer-permission-chip",
            Icon::new(perm_icon)
                .size(IconSize::Xs)
                .color(perm_color)
                .into_any_element(),
            perm_label.to_string(),
            None,
            self.is_permission_picker_open,
            208.0,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Access, cx)));
        let access = popover_anchor(access, cx);
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
                    .children(self.render_mode_pills(cx))
                    .child(model)
                    .child(access),
            )
            .child(self.render_send_button(running_here, cx))
    }

    /// MonoCode `ToolButton`: 26px on the selection fill, brighter while
    /// its menu is open.
    fn plus_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let open = self.is_plus_menu_open;
        let (fill, hover, emphasis) = (selection(cx), selection_hover(cx), selection_emphasis(cx));
        let button = div()
            .id("composer-plus")
            .size(px(26.0))
            .flex_none()
            .rounded(px(6.0))
            .bg(if open { emphasis } else { fill })
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(!open, |el| el.hover(move |s| s.bg(hover)))
            .tooltip(Tooltip::text("Add files or choose a mode"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_popover(Popover::Plus, cx)))
            .child(Icon::new(IconName::Plus).size(IconSize::Xs).color(if open {
                colors.fg
            } else {
                colors.fg.opacity(0.5)
            }));
        popover_anchor(button, cx)
    }

    /// Esc: closes whichever composer popover is open. True if one was.
    pub fn close_composer_popovers(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.is_plus_menu_open
            || self.is_permission_picker_open
            || self.is_branch_picker_open
            || self.is_model_picker_open
            || self.composer_menus.recent_open
            || self.composer_menus.workspace_menu.is_some();
        if open {
            self.composer_menus.workspace_menu = None;
            self.composer_menus.recent_open = false;
            self.composer_menus.model_submenu = None;
            self.is_plus_menu_open = false;
            self.is_permission_picker_open = false;
            self.is_branch_picker_open = false;
            self.is_model_picker_open = false;
            cx.notify();
        }
        open
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
        self.composer_menus.recent_open = false;
        self.is_plus_menu_open = open && which == Popover::Plus;
        self.is_model_picker_open = open && which == Popover::Model;
        self.is_permission_picker_open = open && which == Popover::Access;
        self.is_branch_picker_open = false;
        if self.is_permission_picker_open {
            let mode = self.session_permission_mode(self.selected_session());
            self.composer_menus.access_index = PERMISSION_MODES
                .iter()
                .position(|(m, ..)| *m == mode)
                .unwrap_or(0);
            self.focus_composer_menu(cx);
        } else if which == Popover::Access {
            self.refocus_prompt(cx);
        }
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
        let drafting = self
            .selected_session_id
            .as_deref()
            .is_some_and(|id| self.draft_mode.contains(id))
            || mode_commands::leading_mode(self.prompt_input.read(cx).text())
                .is_some_and(|(m, _)| m == mode_commands::ModeCommand::Draft);
        // MonoCode `ComposerAction` labels.
        let tooltip = if stop {
            "Stop"
        } else if drafting {
            "Save draft"
        } else {
            "Send"
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

    /// MonoCode `MessageQueue`: a tab sitting on the composer listing the
    /// messages that wait for the turn — each with Edit (in place) and
    /// Remove — headed by "Queue paused" with Resume once the agent was
    /// stopped.
    fn render_message_queue(&self, session_id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let queued = self.queued_prompts(session_id);
        if queued.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let icon_button = |id: SharedString, icon: IconName, tip: &'static str| {
            let hover = colors.fg.opacity(0.10);
            div()
                .id(id)
                .size(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(Tooltip::text(tip))
                .child(
                    Icon::new(icon)
                        .size(IconSize::Xs)
                        .color(colors.fg.opacity(0.55)),
                )
        };
        let editing = self
            .queue_editing
            .as_ref()
            .filter(|(sid, _)| sid == session_id)
            .map(|(_, ix)| *ix);
        let rows =
            queued.iter().enumerate().map(|(ix, item)| {
                let row = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_h(px(28.0))
                    .text_size(px(12.0))
                    .when(ix > 0, |el| el.border_t_1().border_color(colors.border));
                if editing == Some(ix) {
                    return row
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(colors.fg.opacity(0.3))
                                .bg(colors.fg.opacity(0.05))
                                .child(self.queue_edit_input.clone()),
                        )
                        .child(
                            icon_button(format!("queue-save-{ix}").into(), IconName::Check, "Save")
                                .on_click(cx.listener(|this, _, _, cx| this.save_queue_edit(cx))),
                        )
                        .child(
                            icon_button(format!("queue-cancel-{ix}").into(), IconName::X, "Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_queue_edit(cx);
                                })),
                        )
                        .into_any_element();
                }
                let label = if item.text.trim().is_empty() {
                    let n = item.attachments.len();
                    format!("{n} attachment{}", if n == 1 { "" } else { "s" })
                } else {
                    item.text.lines().next().unwrap_or("").to_string()
                };
                let (edit_sid, remove_sid) = (session_id.to_string(), session_id.to_string());
                row.child(
                    Icon::new(IconName::CornerDownRight)
                        .size(IconSize::Xs)
                        .color(colors.fg.opacity(0.55)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.fg.opacity(0.8))
                        .child(label),
                )
                .child(
                    icon_button(
                        format!("queue-edit-{ix}").into(),
                        IconName::Pencil,
                        "Edit queued message",
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.start_queue_edit(&edit_sid, ix, cx)),
                    ),
                )
                .child(
                    icon_button(
                        format!("queue-remove-{ix}").into(),
                        IconName::Trash2,
                        "Remove queued message",
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_queued_prompt(&remove_sid, ix, cx)
                    })),
                )
                .into_any_element()
            });
        let paused = self.queue_paused(session_id).then(|| {
            let sid = session_id.to_string();
            let hover = colors.fg.opacity(0.10);
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .border_b_1()
                .border_color(colors.border)
                .text_size(px(12.0))
                .text_color(colors.fg.opacity(0.55))
                .child(
                    Icon::new(IconName::Pause)
                        .size(IconSize::Xs)
                        .color(colors.fg.opacity(0.55)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child("Queue paused because you interrupted"),
                )
                .child(
                    div()
                        .id("queue-resume")
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .h(px(24.0))
                        .px_1p5()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.resume_queue(&sid, cx)))
                        .child(
                            Icon::new(IconName::Play)
                                .size(IconSize::Xs)
                                .color(colors.fg.opacity(0.55)),
                        )
                        .child("Resume"),
                )
        });
        Some(
            div()
                .px_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_t(px(10.0))
                        .border_1()
                        .border_b_0()
                        .border_color(colors.border)
                        .bg(colors.fg.opacity(0.03))
                        .children(paused)
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    /// MonoCode's "ADD TO MESSAGE" menu: Upload file, Plan mode, Draft;
    /// the active modes carry a check.
    fn render_plus_menu_popover(&self, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        let sid = self.selected_session_id.clone().unwrap_or_default();
        let typed = mode_commands::leading_mode(self.prompt_input.read(cx).text()).map(|(m, _)| m);
        let plan_on =
            self.plan_mode.contains(&sid) || typed == Some(mode_commands::ModeCommand::Plan);
        let draft_on =
            self.draft_mode.contains(&sid) || typed == Some(mode_commands::ModeCommand::Draft);
        let upload: PlusAction = |this, cx| this.open_attachment_dialog(cx);
        let plan: PlusAction = |this, cx| this.toggle_mode(false, cx);
        let draft: PlusAction = |this, cx| this.toggle_mode(true, cx);
        let items = [
            (
                "plus-upload",
                IconName::FilePlus,
                colors.fg_muted,
                "Upload file",
                "Attach files or images",
                false,
                upload,
            ),
            (
                "plus-plan",
                IconName::Lightbulb,
                colors.warning.opacity(0.8),
                "Plan mode",
                "Review a plan before building",
                plan_on,
                plan,
            ),
            (
                "plus-draft",
                IconName::CircleDashed,
                colors.fg.opacity(0.6),
                "Draft",
                "Save this message without starting the agent",
                draft_on,
                draft,
            ),
        ];
        div()
            .id("composer-plus-popover")
            .absolute()
            .bottom(px(40.0))
            .left(px(8.0))
            .w(px(250.0))
            .p_1p5()
            .rounded(px(12.0))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .pt_0p5()
                    .pb_1()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.fg.opacity(0.4))
                    .child("ADD TO MESSAGE"),
            )
            .children(
                items
                    .into_iter()
                    .map(|(id, icon, tint, title, hint, active, action)| {
                        let hover = colors.fg.opacity(0.10);
                        div()
                            .id(id)
                            .flex()
                            .items_start()
                            .gap(px(10.0))
                            .px_2()
                            .py_2()
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.is_plus_menu_open = false;
                                action(this, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .mt_0p5()
                                    .child(Icon::new(icon).size(IconSize::Sm).color(tint)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(colors.fg)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(colors.fg.opacity(0.45))
                                            .child(hint),
                                    ),
                            )
                            .when(active, |el| {
                                el.child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Xs)
                                        .color(colors.accent),
                                )
                            })
                    }),
            )
    }

    fn render_permission_picker_popover(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let session = self.selected_session();
        let current_mode = self.session_permission_mode(session);
        let busy = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        let active = self.composer_menus.access_index;
        let menu = div()
            .id("composer-permission-popover")
            .track_focus(&self.composer_menus.focus)
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
            .children(PERMISSION_MODES.iter().enumerate().map(|(ix, &(mode, label, hint, icon))| {
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
                    .when(mode == current_mode || ix == active, |el| el.bg(colors.active))
                    .hover(|s| s.bg(colors.hover))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.composer_menus.access_index != ix {
                            this.composer_menus.access_index = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.is_permission_picker_open = false;
                        this.set_permission_mode(mode, cx);
                        this.refocus_prompt(cx);
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
            });
        popover_surface(menu, cx)
    }

    /// The branch trigger; opens Ely's `BranchSelector` (MonoCode
    /// `BranchPicker`). Locked while the agent works.
    fn branch_menu(
        &self,
        session: Option<&SessionRow>,
        enabled: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let current = session
            .and_then(|s| s.branch.clone())
            .or_else(|| Some(self.git_status.branch.clone()).filter(|b| !b.is_empty()))
            .unwrap_or_else(|| "main".to_string());
        let chip = git_trigger(
            "composer-branch",
            IconName::GitBranch,
            current,
            enabled,
            self.is_branch_picker_open,
            cx,
        )
        .when(enabled, |el| {
            el.on_click(cx.listener(|this, _, _, cx| {
                let open = !this.is_branch_picker_open;
                this.close_composer_popovers(cx);
                this.is_branch_picker_open = open;
                cx.notify();
            }))
        });
        popover_anchor(chip, cx)
    }

    fn render_branch_picker(&self, cx: &Context<Self>) -> gpui::Div {
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
