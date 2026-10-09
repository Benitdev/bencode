//! Prompt composer: model / permission / branch menus, context meter, the
//! prompt field with `/` and `@` suggestions, and send / stop.
//! 100% faithful to MonoCode Composer layout.

pub mod add_to_chat;
pub mod attachments;
pub mod branch_picker;
pub mod cards;
mod context_ring;
pub mod edit_last_turn;
mod folder_picker;
pub mod handoff;
pub mod inbox_card;
mod mcp_picker;
pub mod mcp_tags;
pub mod mentions;
pub(crate) mod menus;
pub mod mode_commands;
mod model_picker;
pub mod new_skill;
pub mod new_worktree;
pub mod note_card;
pub mod prompt_marks;
pub mod question;
mod removed_worktree;
pub mod runner;
pub mod runner_view;
mod search_popover;
mod suggestions;
pub mod tokens;
pub mod usage_limit;

use ely_gpui_component::buttons::ButtonVariant;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnimationExt, AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*,
};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::SessionRow;
use crate::harness::{HarnessKind, catalog};
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_glass;
pub use mcp_picker::McpPicker;
pub use menus::{MenuState, focus_later};
use menus::{over_composer, popover_anchor, popover_surface};
pub use prompt_marks::{MentionMark, prompt_highlights};

/// MonoCode `max-w-4xl`, the same column as the transcript.
const COMPOSER_MAX_WIDTH: f32 = 896.0;
/// MonoCode's default placeholder (trailing space included).
pub const PROMPT_PLACEHOLDER: &str = "Ask, build, / for commands, @ for references... ";
/// MonoCode `CwdPicker` `PREVIEW`.
const RECENT_PROJECTS_SHOWN: usize = 5;
const HARNESS_ORDER: [HarnessKind; 4] = [
    HarnessKind::Claude,
    HarnessKind::Antigravity,
    HarnessKind::Codex,
    HarnessKind::OpenCode,
];
/// The composer's toolbar popover; at most one is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerPopover {
    Plus,
    Model,
    /// MonoCode `AccessPicker`.
    Access,
    Branch,
    /// MonoCode `WorktreeBasePicker` (the "From main" chip).
    Base,
}

/// The `/` or `@` picker following the token at the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPicker {
    Skill,
    Mention,
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
    icon: impl Into<TriggerIcon>,
    label: String,
    enabled: bool,
    open: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    git_label(id, icon, label, Some(enabled), open, cx)
}

/// A trigger's glyph: an Ely icon, or MonoCode's worktree folder (which
/// Ely does not ship).
#[derive(Clone, Copy)]
enum TriggerIcon {
    Named(IconName),
    FolderTree,
}

impl From<IconName> for TriggerIcon {
    fn from(name: IconName) -> Self {
        Self::Named(name)
    }
}

impl TriggerIcon {
    fn render(self, color: gpui::Hsla) -> AnyElement {
        match self {
            Self::Named(name) => Icon::new(name)
                .size(IconSize::Xs)
                .color(color)
                .into_any_element(),
            Self::FolderTree => crate::ui::icons::ExtraIcon::FolderTree
                .icon()
                .size(IconSize::Xs)
                .color(color)
                .into_any_element(),
        }
    }
}

/// `GitPickerTrigger` when `enabled` is set, else MonoCode's static
/// `WorkspaceIdentity` label (`text-content/45`, no hover).
fn git_label(
    id: &'static str,
    icon: impl Into<TriggerIcon>,
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
        .child(icon.into().render(fg.opacity(0.55)))
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

pub(crate) fn permission_entry(mode: PermissionMode) -> (&'static str, IconName) {
    PERMISSION_MODES
        .iter()
        .find(|(m, ..)| *m == mode)
        .map_or(("Supervised", IconName::Lock), |(_, label, _, icon)| {
            (*label, *icon)
        })
}

/// MonoCode's model chip title: "Harness · Provider · Model · Effort",
/// then how to reach the recent models.
fn model_chip_tooltip(
    key: &str,
    values: Option<&serde_json::Map<String, serde_json::Value>>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = catalog::find(key) {
        parts.push(model.harness.label().to_string());
        parts.extend(model.provider.clone());
    }
    parts.push(catalog::label_for(key));
    if let Some(effort) = catalog::effort_setting(key) {
        parts.push(effort.value_label(values).to_string());
    }
    format!("{} · Recent models: right-click or ⌘.", parts.join(" · "))
}

fn focus_id_key(session: Option<&SessionRow>) -> String {
    session.map_or_else(String::new, |s| s.id.clone())
}

/// MonoCode `useComposerDockMotion`: 480ms on `cubic-bezier(0.22, 1, 0.36,
/// 1)`, only when docking follows a send within 1.5s.
const DOCK_MOTION: std::time::Duration = std::time::Duration::from_millis(480);
const DOCK_WINDOW: std::time::Duration = std::time::Duration::from_millis(1500);

/// What the centred composer of a new thread measures each frame, so the
/// docked one can start where it was.
#[derive(Clone, Copy, Debug)]
pub enum DockProbe {
    /// The centred composer's box.
    Composer,
    /// The empty pane under it; the docked composer ends at its bottom.
    Pane,
}

/// The last measured centred composer (top, height) and pane bottom.
#[derive(Default)]
pub struct DockMeasure {
    pub composer: std::cell::Cell<Option<(f32, f32)>>,
    pub pane_bottom: std::cell::Cell<Option<f32>>,
}

/// A send from the centred composer: the thread, when, and how far below
/// its docked place the composer starts.
pub struct DockLaunch {
    pub session_id: String,
    pub at: std::time::Instant,
    pub rise: f32,
}

impl BenCodeApp {
    /// A zero-cost element recording `probe`'s bounds as it is laid out.
    pub fn dock_probe_canvas(&self, probe: DockProbe) -> impl IntoElement {
        let measure = self.dock_measure.clone();
        gpui::canvas(
            move |bounds, _, _| match probe {
                DockProbe::Composer => measure.composer.set(Some((
                    crate::ui::scale::logical(bounds.origin.y),
                    crate::ui::scale::logical(bounds.size.height),
                ))),
                DockProbe::Pane => measure.pane_bottom.set(Some(crate::ui::scale::logical(
                    bounds.origin.y + bounds.size.height,
                ))),
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// Called as the first message leaves the centred composer.
    pub fn launch_dock_motion(&mut self, session_id: &str) {
        let (Some((top, height)), Some(bottom)) = (
            self.dock_measure.composer.get(),
            self.dock_measure.pane_bottom.get(),
        ) else {
            return;
        };
        // Docked, the box is 8px shorter (py-3 instead of py-4).
        let docked_top = bottom - (height - 8.0);
        let rise = top - docked_top;
        if rise.abs() < 1.0 {
            return;
        }
        self.dock_launch = Some(DockLaunch {
            session_id: session_id.to_string(),
            at: std::time::Instant::now(),
            rise,
        });
    }

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
    ) -> gpui::AnyElement {
        let colors = &cx.theme().colors;
        let running_here = session.is_some_and(|s| self.is_agent_running_in(&s.id));
        let queue = session.and_then(|s| self.render_message_queue(&s.id, cx));
        // MonoCode `fileDrag`: files held over this thread's pane.
        let file_drag =
            session.is_some_and(|s| self.active_file_drop_target.as_deref() == Some(s.id.as_str()));
        let editing = focused && self.is_editing_last_turn();
        let field_pad = if shell { px(16.0) } else { px(12.0) };
        let field = if focused {
            self.prompt_input.clone().into_any_element()
        } else {
            // Another pane's draft, as it was left.
            let draft = session
                .and_then(|s| self.thread(&s.id)?.draft.as_ref())
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
        // The docked composer drops in from where the centred one stood.
        let dock_rise = self
            .dock_launch
            .as_ref()
            .filter(|launch| {
                !shell
                    && session.is_some_and(|s| s.id == launch.session_id)
                    && launch.at.elapsed() < DOCK_WINDOW
            })
            .map(|launch| (launch.rise, launch.at));
        let composer = div()
            .id(SharedString::from(format!(
                "composer-{}",
                session.map_or("none", |s| s.id.as_str())
            )))
            // MonoCode binds ⌘⇧G to the workspace only in a new thread.
            .when(focused && self.is_draft_workspace(), |el| {
                el.key_context("DraftComposer")
            })
            .flex_none()
            .w_full()
            .max_w(px(COMPOSER_MAX_WIDTH))
            .mx_auto()
            .px(px(6.0))
            .pb(px(6.0))
            // MonoCode stacks the question form, then the queue, on the box;
            // the mascot runs on top of that stack when there is one.
            .child(
                div()
                    .relative()
                    .children(
                        session
                            .filter(|_| focused)
                            .and_then(|s| self.render_question_form(&s.id, cx)),
                    )
                    .children(
                        session
                            .filter(|_| focused)
                            .and_then(|s| self.render_usage_limit(&s.id, cx)),
                    )
                    .children(queue.filter(|_| focused))
                    .when(focused, |el| {
                        el.child(runner_view::measure(&self.runner_geometry.ledge))
                    }),
            )
            .child(
                div()
                    .relative()
                    .when(focused, |el| {
                        el.child(runner_view::measure(&self.runner_geometry.track))
                    })
                    // A press anywhere on the box that no button took
                    // (buttons prevent default) puts the caret in the prompt;
                    // an open popover keeps its own focus.
                    .when(focused, |el| {
                        el.on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                if window.default_prevented()
                                    || this.composer_popover.is_some()
                                    || this.mcp_picker.is_some()
                                    || this.folder_picker.is_some()
                                {
                                    return;
                                }
                                let handle =
                                    gpui::Focusable::focus_handle(this.prompt_input.read(cx), cx);
                                window.focus(&handle, cx);
                            }),
                        )
                    })
                    .rounded(px(8.0))
                    .border_1()
                    // MonoCode `edit-last-turn-composer`: dashed, in the accent.
                    .when(editing && !file_drag, |el| el.border_dashed())
                    .border_color(if file_drag {
                        colors.accent.opacity(0.6)
                    } else if editing {
                        colors.accent.opacity(0.32)
                    } else {
                        colors
                            .fg
                            .opacity(if self.prompt_focused { 0.2 } else { 0.1 })
                    })
                    .bg(colors.fg.opacity(0.03))
                    // MonoCode light theme: the page colour, lifted by a shadow.
                    .when(!cx.theme().is_dark(), |el| el.bg(colors.bg).shadow_md())
                    .when(focused, |el| {
                        el.children(
                            self.render_mcp_picker(cx)
                                .or_else(|| self.render_folder_picker(cx))
                                .or_else(|| self.render_suggestions(cx)),
                        )
                        .children(self.render_mention_marks(cx))
                    })
                    .child(self.composer_top_bar(session, cx))
                    .children(
                        session
                            .filter(|_| focused)
                            .and_then(|s| self.render_composer_card(&s.id, cx)),
                    )
                    .when(focused, |el| el.children(self.render_attachment_chips(cx)))
                    .when_some(
                        self.composer_error.clone().filter(|_| focused),
                        |el, error| {
                            el.child(
                                div()
                                    .px_3()
                                    .pt_2()
                                    .text_size(px(12.0))
                                    .text_color(colors.danger)
                                    .child(error),
                            )
                        },
                    )
                    .child(
                        div()
                            .px_3()
                            .py(field_pad)
                            .max_h(px(160.0) + field_pad * 2.0)
                            .text_size(px(14.0))
                            .line_height(px(22.0))
                            .child(field),
                    )
                    .when(focused && self.popover_open(ComposerPopover::Plus), |el| {
                        el.child(over_composer(popover_surface(
                            self.render_plus_menu_popover(cx),
                            cx,
                        )))
                    })
                    .child(self.composer_bottom_bar(session, running_here, focused, cx))
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
            );
        match dock_rise {
            Some((rise, at)) if at.elapsed() < DOCK_MOTION => composer
                .relative()
                .with_animation(
                    SharedString::from(format!("composer-dock-{}", focus_id_key(session))),
                    gpui::Animation::new(DOCK_MOTION).with_easing(gpui::ease_out_quint()),
                    move |el, delta| el.top(px(rise * (1.0 - delta))),
                )
                .into_any_element(),
            _ => composer.into_any_element(),
        }
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
            .map(|el| match session.filter(|s| s.worktree_removed) {
                Some(session) => el.child(self.removed_worktree_picker(session, cx)),
                None => el
                    .child(self.workspace_identity(session, empty && !busy, cx))
                    .child(match self.new_worktree_base().filter(|_| empty) {
                        Some(base) => self.base_chip(base, cx).into_any_element(),
                        None => self.branch_menu(session, !busy, cx).into_any_element(),
                    }),
            })
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .children(session.and_then(|s| self.context_meter(s, cx))),
            )
    }

    /// MonoCode `CwdPicker`: the five latest other projects, the rest
    /// under "More Projects", then "Open folder…" and "New terminal".
    fn project_picker(&self, project: &str, cx: &Context<Self>) -> impl IntoElement {
        let item = |path: &String| {
            let name = std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            let target = path.clone();
            MenuItem::new(name)
                .icon(IconName::Folder)
                .on_click(app_callback(cx, move |this, cx| {
                    this.switch_project(target.clone(), cx)
                }))
        };
        let others: Vec<&String> = self
            .recent_projects
            .iter()
            .filter(|path| !crate::app::same_project_path(path, &self.current_cwd))
            .collect();
        let (recent, more) = others.split_at(others.len().min(RECENT_PROJECTS_SHOWN));
        let mut menu = recent
            .iter()
            .fold(Menu::new(), |menu, path| menu.item(item(path)));
        if !more.is_empty() {
            let rest = more
                .iter()
                .fold(Menu::new(), |menu, path| menu.item(item(path)));
            menu = menu.item(MenuItem::submenu("More Projects", rest));
        }
        if !others.is_empty() {
            menu = menu.separator();
        }
        let menu = menu
            .item(
                MenuItem::new("Open folder…")
                    .icon(IconName::FolderPlus)
                    .on_click(app_callback(cx, |this, cx| this.open_project_dialog(cx))),
            )
            .item(
                MenuItem::new("New terminal")
                    .icon(IconName::Terminal)
                    .keys("⌘`")
                    .on_click(app_callback(cx, |this, cx| this.new_terminal(cx))),
            );
        DropdownMenu::new("composer-project", project.to_string(), menu)
            .variant(ButtonVariant::Ghost)
            .icon(IconName::Folder)
    }

    /// The checkout the thread runs in (MonoCode `WorkspaceIdentity`, and
    /// `WorkspacePicker` while the thread is new): "Current checkout",
    /// "New worktree" (made on the first send) or "Worktree".
    fn workspace_identity(
        &self,
        session: Option<&SessionRow>,
        can_switch: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let in_worktree = session.map_or(self.worktree_focus().is_some(), |s| {
            s.worktree_cwd.as_deref().is_some_and(|w| !w.is_empty())
        });
        let preparing =
            session.is_some_and(|s| self.thread(&s.id).is_some_and(|t| t.preparing_worktree));
        let new_tree = can_switch && self.new_worktree_base().is_some();
        let (label, icon) = if preparing {
            ("Creating worktree…", TriggerIcon::FolderTree)
        } else if new_tree {
            ("New worktree", TriggerIcon::FolderTree)
        } else if in_worktree {
            ("Worktree", TriggerIcon::FolderTree)
        } else {
            ("Current checkout", TriggerIcon::Named(IconName::Folder))
        };
        let switchable = can_switch && !preparing && self.is_draft_workspace();
        let open = self.composer_menus.workspace_menu.is_some();
        let trigger = git_label(
            "composer-workspace",
            icon,
            label.to_string(),
            switchable.then_some(true),
            open,
            cx,
        )
        .when(switchable, |el| {
            el.tooltip(Tooltip::text(format!("Workspace: {label} (⌘⇧G)")))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_workspace_menu(cx)))
        });
        div()
            .relative()
            .flex_none()
            .child(popover_anchor(trigger, cx))
            // Anchored from the trigger's top, so the menu rises above it.
            .children(open.then(|| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(self.render_workspace_menu(cx))
            }))
    }

    /// MonoCode `WorktreeBasePicker` trigger: "From main".
    fn base_chip(&self, base: &str, cx: &Context<Self>) -> impl IntoElement {
        let chip = git_trigger(
            "composer-worktree-base",
            IconName::GitBranch,
            format!("From {base}"),
            true,
            self.popover_open(ComposerPopover::Base),
            cx,
        )
        .tooltip(Tooltip::text(format!("Create worktree from {base}")))
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_branch_picker(branch_picker::BranchPickerKind::Base, cx)
        }));
        div()
            .relative()
            .flex_none()
            .child(popover_anchor(chip, cx))
            .when(self.popover_open(ComposerPopover::Base), |el| {
                el.child(self.render_branch_popover(branch_picker::BranchPickerKind::Base, cx))
            })
    }

    /// "+" (add to message), model, access, then Send / Stop.
    fn composer_bottom_bar(
        &self,
        session: Option<&SessionRow>,
        running_here: bool,
        focused: bool,
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
        let values = session.map_or(Some(&self.last_model_settings), |s| {
            s.model_settings.as_ref()
        });
        let model_tip = model_chip_tooltip(key, values);
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
            self.popover_open(ComposerPopover::Model) || self.composer_menus.recent_open,
            160.0,
            cx,
        )
        .tooltip(Tooltip::text(model_tip))
        .on_click(
            cx.listener(|this, _, _, cx| this.toggle_composer_popover(ComposerPopover::Model, cx)),
        )
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(|this, _, _, cx| this.toggle_recent_models(cx)),
        );
        // Popovers hang from their chip (MonoCode `Popover anchor`), so pills
        // before it never push them out of line.
        let model = div()
            .relative()
            .flex_none()
            .child(popover_anchor(model, cx))
            .when(focused && self.popover_open(ComposerPopover::Model), |el| {
                el.child(over_composer(popover_surface(
                    self.render_model_picker_popover(key, cx),
                    cx,
                )))
            })
            .when(focused && self.composer_menus.recent_open, |el| {
                el.child(over_composer(popover_surface(
                    self.render_recent_models_popover(cx),
                    cx,
                )))
            });
        let busy_note = if running_here {
            " Changes apply to the next turn."
        } else {
            ""
        };
        let access_tip = format!(
            "{}{busy_note}",
            PERMISSION_MODES
                .iter()
                .find(|(m, ..)| *m == mode)
                .map_or("", |(_, _, hint, _)| *hint)
        );
        let access = composer_chip(
            "composer-permission-chip",
            Icon::new(perm_icon)
                .size(IconSize::Xs)
                .color(perm_color)
                .into_any_element(),
            perm_label.to_string(),
            None,
            self.popover_open(ComposerPopover::Access),
            208.0,
            cx,
        )
        .tooltip(Tooltip::text(access_tip))
        .on_click(
            cx.listener(|this, _, _, cx| this.toggle_composer_popover(ComposerPopover::Access, cx)),
        );
        let access = div()
            .relative()
            .flex_none()
            .child(popover_anchor(access, cx))
            .when(
                focused && self.popover_open(ComposerPopover::Access),
                |el| el.child(over_composer(self.render_permission_picker_popover(cx))),
            );
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
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .children(self.render_cancel_edit(cx))
                    .child(self.render_send_button(running_here, cx)),
            )
    }

    /// MonoCode `ToolButton`: 26px on the selection fill, brighter while
    /// its menu is open.
    fn plus_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let open = self.popover_open(ComposerPopover::Plus);
        let (fill, hover, emphasis) = (selection(cx), selection_hover(cx), selection_emphasis(cx));
        let button =
            div()
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
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_composer_popover(ComposerPopover::Plus, cx)
                }))
                .child(Icon::new(IconName::Plus).size(IconSize::Xs).color(if open {
                    colors.fg
                } else {
                    colors.fg.opacity(0.5)
                }));
        popover_anchor(button, cx)
    }

    /// Whether `which` is the open toolbar popover.
    pub(crate) fn popover_open(&self, which: ComposerPopover) -> bool {
        self.composer_popover == Some(which)
    }

    /// Closes `which` if it is the open one; another popover stays.
    pub(crate) fn close_popover(&mut self, which: ComposerPopover) {
        if self.composer_popover == Some(which) {
            self.composer_popover = None;
        }
    }

    pub(crate) fn skill_picker_open(&self) -> bool {
        self.token_picker == Some(TokenPicker::Skill)
    }

    pub(crate) fn mention_picker_open(&self) -> bool {
        self.token_picker == Some(TokenPicker::Mention)
    }

    /// Esc: closes whichever composer popover is open. True if one was.
    pub fn close_composer_popovers(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.composer_popover.is_some()
            || self.composer_menus.recent_open
            || self.composer_menus.workspace_menu.is_some()
            || self.composer_menus.context_pinned;
        if open {
            self.composer_menus.context_pinned = false;
            self.composer_menus.workspace_menu = None;
            self.composer_menus.recent_open = false;
            self.composer_menus.model_submenu = None;
            self.composer_popover = None;
            cx.notify();
        }
        open
    }

    /// Opens one composer popover and closes the others.
    fn toggle_composer_popover(&mut self, which: ComposerPopover, cx: &mut Context<Self>) {
        if which == ComposerPopover::Model {
            self.toggle_model_picker(cx);
            return;
        }
        let open = !self.popover_open(which);
        self.composer_menus.recent_open = false;
        self.composer_popover = open.then_some(which);
        if self.popover_open(ComposerPopover::Access) {
            let mode = self.session_permission_mode(self.selected_session());
            self.composer_menus.access_index = PERMISSION_MODES
                .iter()
                .position(|(m, ..)| *m == mode)
                .unwrap_or(0);
            self.focus_composer_menu(cx);
        } else if which == ComposerPopover::Access {
            self.refocus_prompt(cx);
        }
        cx.notify();
    }

    /// Send / Stop, as MonoCode's `ComposerAction`: a 26px light button with
    /// a dark arrow, or a filled square to stop. While this thread runs an
    /// empty composer offers Stop; typing turns it back into Send (queue).
    fn render_send_button(&self, running_here: bool, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        // A card can be sent on its own.
        let typed = self.composer_has_value(cx);
        let removed = self
            .selected_session_id
            .as_deref()
            .is_some_and(|id| self.worktree_removed(id));
        let stop = running_here && !typed;
        // MonoCode: no working copy, no Send.
        let enabled = (running_here || typed) && !(removed && !stop);
        // MonoCode `html.has-user-accent .primary-action`: the accent the
        // user picked, else the light button.
        let accent = crate::ui::appearance::user_accent(cx);
        let (bg, ink) = match (accent, enabled) {
            (Some(accent), true) => (accent.color, accent.foreground),
            (Some(accent), false) => (accent.color.opacity(0.28), colors.fg.opacity(0.35)),
            (None, true) => (colors.fg, colors.bg),
            (None, false) => (colors.fg.opacity(0.3), colors.bg.opacity(0.4)),
        };
        let drafting = self
            .selected_session_id
            .as_deref()
            .is_some_and(|id| self.thread(id).is_some_and(|t| t.draft_mode))
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
        let hover = match accent {
            Some(accent) => crate::ui::appearance::mix(accent.color, colors.fg, 0.12),
            None => colors.fg.opacity(0.9),
        };
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
                let (edit_sid, remove_sid, steer_sid) = (
                    session_id.to_string(),
                    session_id.to_string(),
                    session_id.to_string(),
                );
                let steer_hover = colors.fg.opacity(0.10);
                let running_steer = self.is_agent_running_in(session_id);
                row.child(
                    Icon::new(IconName::List)
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
                // MonoCode's Steer: this message into the running turn now, or
                // as a turn of its own while the agent is idle.
                .when(
                    self.can_steer(session_id) || !self.is_agent_running_in(session_id),
                    |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("queue-steer-{ix}")))
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .h(px(24.0))
                                .px_1p5()
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(move |s| s.bg(steer_hover))
                                .tooltip(Tooltip::text(if running_steer {
                                    "Send into the running turn"
                                } else {
                                    "Send now"
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.steer_queued(&steer_sid, ix, cx)
                                }))
                                .child(
                                    Icon::new(IconName::CornerDownRight)
                                        .size(IconSize::Xs)
                                        .color(colors.fg.opacity(0.55)),
                                )
                                .child("Steer"),
                        )
                    },
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
        let plan_on = self.thread(&sid).is_some_and(|t| t.plan_mode)
            || typed == Some(mode_commands::ModeCommand::Plan);
        let draft_on = self.thread(&sid).is_some_and(|t| t.draft_mode)
            || typed == Some(mode_commands::ModeCommand::Draft);
        let upload: PlusAction = |this, cx| this.open_attachment_dialog(cx);
        let plan: PlusAction = |this, cx| this.toggle_mode(false, cx);
        let draft: PlusAction = |this, cx| this.toggle_mode(true, cx);
        let can_draft = self.can_save_draft(self.selected_session_id.as_deref());
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
            .bg(popover_glass(cx))
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
                    // MonoCode offers Draft only where one can be saved.
                    .filter(|item| item.0 != "plus-draft" || can_draft)
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
                                this.close_popover(ComposerPopover::Plus);
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
            .occlude()
            .absolute()
            .bottom(px(32.0))
            .left_0()
            .w(px(288.0))
            .p_1()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
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
                        this.close_popover(ComposerPopover::Access);
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

    /// The branch trigger (MonoCode `BranchPicker`); its popover opens
    /// above it. Locked while the agent works.
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
        // Only the focused composer's chip owns the open popover.
        let focused = session.map(|s| s.id.as_str()) == self.selected_session_id.as_deref();
        let open = focused && self.popover_open(ComposerPopover::Branch);
        let chip = git_trigger(
            "composer-branch",
            IconName::GitBranch,
            current,
            enabled,
            open,
            cx,
        )
        .when(enabled, |el| {
            el.on_click(cx.listener(|this, _, _, cx| {
                this.toggle_branch_picker(branch_picker::BranchPickerKind::Branch, cx)
            }))
        });
        div()
            .relative()
            .flex_none()
            .child(popover_anchor(chip, cx))
            .when(open, |el| {
                el.child(self.render_branch_popover(branch_picker::BranchPickerKind::Branch, cx))
            })
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
