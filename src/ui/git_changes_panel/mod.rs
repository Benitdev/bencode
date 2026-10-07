//! MonoCode `GitChangesPanel` and `GitHistoryGraph`: the sidebar's Changes
//! view. A header (branch, ahead/behind, Pull), the commit box (message
//! with ✨ to write it, Commit with Commit & Push / … & Create PR / Amend),
//! Publish / Sync / Create PR / View PR, the staged and unstaged files as a
//! list or a tree, and the commit graph under a resize sash.

mod confirm;
mod graph;
mod tree;

pub use confirm::GitConfirm;
pub use tree::{ChangeDir, build_tree};
use tree::{basename, dirname};

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ely_gpui_component::feedback::Alert;
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Severity, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    Animation, AnimationExt, AnyElement, Bounds, Context, FontWeight, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, PathBuilder, Pixels, ScrollHandle, SharedString, Styled, canvas,
    div, percentage, point, prelude::*, relative, rgb,
};

use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::scrollbar;
use crate::app::file_pane::PaneTab;
use crate::ui::appearance::DiffColors;
use crate::git::graph::{self as git_graph, Cmd, Node};
use crate::git::sync::{BranchPr, HistoryCommit};
use crate::git::{
    GitFileChange, GitFileStatus, discard_all, discard_file,
};
use crate::ui::app_callback::app_callback;
use crate::ui::git_menus::{GitMenuKind, TriggerLook};
use crate::ui::icons::ExtraIcon;
use crate::ui::virtual_rows;

/// MonoCode `GRAPH_PANEL_MIN`, `GRAPH_PANEL_DEFAULT`.
const GRAPH_MIN: f32 = 120.0;
const GRAPH_DEFAULT: f32 = 240.0;
const ROW_HEIGHT: f32 = 28.0;
/// MonoCode `FileSection`'s `h-7` header.
const SECTION_HEADER_HEIGHT: f32 = 28.0;
/// The Changes list's `py-1`.
const CHANGES_PAD_Y: f32 = 4.0;

/// The git action running; MonoCode allows one at a time per checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Busy {
    Pull,
    Commit,
    Pr,
    Sync,
    Generate,
    All,
    File(String),
    /// Staging or unstaging a folder of the tree view.
    Folder(String),
}

/// One line of the Changes list, flattened so only those in view are built.
enum ChangeItem<'a> {
    Header { side: Side, files: &'a [GitFileChange] },
    Dir { dir: &'a ChangeDir, depth: usize, open: bool, key: String, side: Side },
    File { file: &'a GitFileChange, side: Side, depth: Option<usize> },
}

impl ChangeItem<'_> {
    fn height(&self) -> f32 {
        match self {
            ChangeItem::Header { .. } => SECTION_HEADER_HEIGHT,
            ChangeItem::Dir { .. } | ChangeItem::File { .. } => ROW_HEIGHT,
        }
    }
}

/// Which list a file row belongs to (MonoCode `GitFileDiffKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Staged,
    Unstaged,
}

/// The panel's state (MonoCode keeps most of it per mount, some per run).
pub struct ChangesUi {
    pub busy: Option<Busy>,
    pub status: Option<String>,
    pub(crate) status_generation: u64,
    /// Amending: the branch and HEAD it was turned on for.
    pub amend: Option<(Option<String>, Option<String>)>,
    pub staged_closed: bool,
    pub changes_closed: bool,
    pub graph_closed: bool,
    /// Tree view (else list), saved in settings.
    pub tree: bool,
    /// Folders closed in the tree, `<side>:<dir>`.
    pub collapsed_dirs: HashSet<String>,
    pub graph_height: f32,
    /// The open Commit options / Branch actions menu.
    pub menu: Option<crate::ui::git_menus::GitMenu>,
    /// A dropdown trigger saw this mouse-down (so it is not "outside").
    pub menu_trigger_hit: bool,
    graph_drag: Option<(f32, f32)>,
    /// The panel's height as last laid out, to cap the graph.
    panel_height: Rc<Cell<f32>>,
    pub(crate) generate_cancel: Option<Arc<AtomicBool>>,
    pub pr: Option<BranchPr>,
    /// `cwd` + branch the pull request was read for.
    pub(crate) pr_key: Option<String>,
    /// The commit field's disabled state as last set.
    input_disabled: Option<bool>,
    /// The Commit options / Branch actions triggers as last laid out, so
    /// their menus open under them.
    commit_anchor: Anchor,
    branch_anchor: Anchor,
    /// The graph row under the pointer (MonoCode's `:hover` node look).
    hovered_commit: Option<String>,
    /// The file or folder row under the pointer, which shows its actions.
    /// Hiding them with a hover style instead would paint children GPUI
    /// never laid out.
    hovered_row: Option<SharedString>,
    /// The file list's scroll pane, read to build only the rows in view.
    scroll: ScrollHandle,
    /// The history graph's scroll pane, likewise.
    graph_scroll: ScrollHandle,
    /// `git_graph::layout` of the current history, kept until it changes.
    graph_rows: RefCell<Option<Rc<Vec<git_graph::Row>>>>,
}

type Anchor = Rc<Cell<Option<Bounds<Pixels>>>>;

impl ChangesUi {
    /// Drops the cached graph layout; called when the history changes.
    pub(crate) fn invalidate_graph(&mut self) {
        self.graph_rows = RefCell::new(None);
    }

    pub(crate) fn menu_anchor(&self, kind: GitMenuKind) -> Anchor {
        match kind {
            GitMenuKind::Commit => self.commit_anchor.clone(),
            GitMenuKind::Branch => self.branch_anchor.clone(),
        }
    }
}

impl Default for ChangesUi {
    fn default() -> Self {
        Self {
            busy: None,
            status: None,
            status_generation: 0,
            amend: None,
            staged_closed: false,
            changes_closed: false,
            graph_closed: false,
            tree: false,
            collapsed_dirs: HashSet::new(),
            graph_height: GRAPH_DEFAULT,
            menu: None,
            menu_trigger_hit: false,
            graph_drag: None,
            panel_height: Rc::new(Cell::new(0.0)),
            generate_cancel: None,
            pr: None,
            pr_key: None,
            input_disabled: None,
            commit_anchor: Rc::default(),
            branch_anchor: Rc::default(),
            hovered_commit: None,
            hovered_row: None,
            scroll: ScrollHandle::default(),
            graph_scroll: ScrollHandle::default(),
            graph_rows: RefCell::new(None),
        }
    }
}

/// A commit waiting on a confirmation: push it, and open a PR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCommit {
    pub push: bool,
    pub pr: bool,
}

/// MonoCode `statusLetter`.
fn status_letter(status: &GitFileStatus) -> &'static str {
    match status {
        GitFileStatus::Untracked => "U",
        GitFileStatus::Added => "A",
        GitFileStatus::Deleted => "D",
        _ => "M",
    }
}

/// MonoCode `statusColor`: `text-sky-400`, `text-diff-add-fg`,
/// `text-diff-del-fg` (the chosen diff palette), `text-amber-400`.
fn status_color(status: &GitFileStatus, diff: DiffColors) -> Hsla {
    match status {
        GitFileStatus::Untracked => rgb(0x38bdf8).into(),
        GitFileStatus::Added => diff.add_fg,
        GitFileStatus::Deleted => diff.del_fg,
        _ => rgb(0xfbbf24).into(),
    }
}

/// MonoCode `border-stroke`: content at 7% (BenCode's `colors.border` is
/// 10%).
fn stroke(fg: Hsla) -> Hsla {
    fg.opacity(0.07)
}

/// MonoCode `animate-spin` (one turn a second, linear) on `icon`.
pub(crate) fn spinning_icon(
    id: SharedString,
    icon: IconName,
    size: IconSize,
    color: Hsla,
) -> AnyElement {
    Icon::new(icon)
        .size(size)
        .color(color)
        .with_animation(
            id,
            Animation::new(Duration::from_secs(1)).repeat(),
            |icon, delta| icon.rotate(percentage(delta)),
        )
        .into_any_element()
}


impl BenCodeApp {

    pub fn render_git_changes_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.check_amend_target();
        let cwd = self.workspace_cwd();
        let colors = cx.theme().colors.clone();
        if matches!(cwd.trim(), "" | "~") {
            return div()
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .line_height(relative(1.5))
                .text_color(colors.fg.opacity(0.5))
                .child("No project folder")
                .into_any_element();
        }
        // The field is usable only with something to commit (MonoCode
        // `canEditMessage`).
        let disabled = !((!self.git_status.staged.is_empty() || self.changes_ui.amend.is_some())
            && self.changes_ui.busy.is_none());
        if self.changes_ui.input_disabled != Some(disabled) {
            self.changes_ui.input_disabled = Some(disabled);
            self.git_commit_input.update(cx, |input, cx| {
                input.set_disabled(disabled, cx);
                input.set_placeholder(
                    if self.changes_ui.amend.is_some() {
                        "Amend message (⌘↩ to amend)"
                    } else {
                        "Message (⌘↩ to commit)"
                    },
                    cx,
                );
            });
        }
        let graph_open = !self.changes_ui.graph_closed;
        let dragging = self.changes_ui.graph_drag.is_some();
        let height = self.changes_ui.panel_height.clone();
        div()
            .id("git-changes-panel")
            .relative()
            // Tailwind preflight's `line-height: 1.5` (GPUI's default is
            // 1.618); rows that set `leading-*` override it.
            .line_height(relative(1.5))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| height.set(crate::ui::scale::logical(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(dragging, |el| {
                el.cursor_row_resize()
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some((start_y, start_height)) = this.changes_ui.graph_drag {
                            let max = (this.changes_ui.panel_height.get() - 160.0).max(GRAPH_MIN);
                            this.changes_ui.graph_height = (start_height
                                - (crate::ui::scale::logical(event.position.y) - start_y))
                                .clamp(GRAPH_MIN, max);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.changes_ui.graph_drag = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.changes_ui.graph_drag = None;
                            cx.notify();
                        }),
                    )
            })
            .child(self.render_changes_header(cx))
            .when_some(self.workspace.git_error.clone(), |el, error| {
                el.child(
                    div().p_2().child(
                        Alert::new("git-error-banner", Severity::Danger, "Git").body(error),
                    ),
                )
            })
            .child(self.render_commit_box(cx))
            .child(self.render_change_sections(cx))
            .when(graph_open, |el| el.child(self.render_graph_sash(cx)))
            .child(self.render_graph(cx))
            .into_any_element()
    }

    /// "Changes", the status, the branch with ↑/↓, and "…" (Pull).
    fn render_changes_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let sync = &self.git_sync;
        let busy = self.changes_ui.busy.is_some();
        let pulling = self.changes_ui.busy == Some(Busy::Pull);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(36.0))
            .px_3()
            .border_b_1()
            .border_color(stroke(fg))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child("Changes"),
            )
            .children(self.changes_ui.status.clone().map(|status| {
                div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.5))
                    .child(status)
            }))
            .child(div().flex_1())
            .children(sync.branch.clone().map(|branch| {
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.5))
                            .child(
                                Icon::new(IconName::GitBranch)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.5)),
                            )
                            .child(div().min_w_0().truncate().child(branch))
                            .when(sync.ahead > 0, |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .text_color(fg.opacity(0.4))
                                        .child(format!("↑{}", sync.ahead)),
                                )
                            })
                            .when(sync.behind > 0, |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .text_color(fg.opacity(0.4))
                                        .child(format!("↓{}", sync.behind)),
                                )
                            }),
                    )
                    // MonoCode's `size-5 rounded-md text-content/50
                    // hover:bg-content/10 hover:text-content` "Branch actions"
                    // button: `MoreHorizontal size-4`, or `Loader size-3.5`
                    // while pulling; disabled while git runs.
                    .child(
                        div().size(px(20.0)).flex_none().child(
                            self.git_menu_trigger(
                                GitMenuKind::Branch,
                                TriggerLook {
                                    icon: if pulling { IconName::LoaderCircle } else { IconName::Ellipsis },
                                    size: if pulling { IconSize::Sm } else { IconSize::Md },
                                    spin: pulling,
                                    color: fg.opacity(0.5),
                                    hover_color: fg,
                                    fill: None,
                                    hover: fg.opacity(0.10),
                                    open: fg.opacity(0.10),
                                    dim_disabled: true,
                                },
                                !busy,
                                cx,
                            )
                            .rounded(px(6.0)),
                        ),
                    )
            }))
    }

    /// The message with ✨, the Commit split button, and the sync row.
    fn render_commit_box(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let bg = colors.bg;
        let ui = &self.changes_ui;
        let generating = ui.busy == Some(Busy::Generate);
        let can_generate = self.has_changes() && ui.busy.is_none();
        let can_commit = self.can_commit(cx);
        let amend = ui.amend.is_some();
        let sync = &self.git_sync;
        // MonoCode `canEditMessage`: the textarea's `disabled:opacity-40`.
        let can_edit = (!self.git_status.staged.is_empty() || amend) && ui.busy.is_none();
        // `bg-content` / `bg-content/40`, `text-background-base`.
        let fill = if can_commit { fg } else { fg.opacity(0.4) };
        // MonoCode `canOpenMenu`.
        let can_open_menu = sync.branch.is_some() && ui.busy.is_none();
        let wand_hover = fg.opacity(0.20);
        let wand_group = SharedString::from("git-generate-message");
        div()
            .flex_none()
            .p_2()
            .border_b_1()
            .border_color(stroke(fg))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .relative()
                    // The textarea: `max-h-40 rounded-md bg-content/10 py-1
                    // pr-8 pl-2 text-[13px] leading-5 text-content`.
                    .child(
                        div()
                            .rounded(px(6.0))
                            .bg(fg.opacity(0.10))
                            .pl_2()
                            .pr_8()
                            .py_1()
                            .text_size(px(13.0))
                            .line_height(px(20.0))
                            .text_color(fg)
                            .max_h(px(160.0))
                            .when(!can_edit, |el| el.opacity(0.4))
                            .child(self.git_commit_input.clone()),
                    )
                    // `absolute top-1 right-1 grid size-5 rounded-md
                    // bg-content/10 hover:bg-content/20 disabled:opacity-40`:
                    // `WandSparkles size-3`, or a spinning `Loader size-3.5`
                    // that turns into `X` on hover while generating.
                    .child(
                        div()
                            .id(wand_group.clone())
                            .group(wand_group.clone())
                            .absolute()
                            .top(px(4.0))
                            .right(px(4.0))
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(fg.opacity(0.10))
                            .when(!generating && !can_generate, |el| el.opacity(0.4))
                            .when(generating || can_generate, |el| {
                                el.cursor_pointer()
                                    .hover(move |s| s.bg(wand_hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_generate(cx)))
                            })
                            .tooltip(Tooltip::text(if generating {
                                "Cancel commit message generation"
                            } else {
                                "Generate commit message"
                            }))
                            .map(|el| {
                                if generating {
                                    el.child(
                                        div()
                                            .absolute()
                                            .flex()
                                            .group_hover(wand_group.clone(), |s| s.invisible())
                                            .child(spinning_icon(
                                                "git-generate-spin".into(),
                                                IconName::LoaderCircle,
                                                IconSize::Sm,
                                                fg,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .flex()
                                            .invisible()
                                            .group_hover(wand_group.clone(), |s| s.visible())
                                            .child(Icon::new(IconName::X).size(IconSize::Sm).color(fg)),
                                    )
                                } else {
                                    el.child(Icon::new(IconName::WandSparkles).size(IconSize::Xs).color(fg))
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .h(px(28.0))
                    // `h-7 flex-1 gap-1.5 rounded-l-md text-[12px] font-medium`
                    // with `Check size-3.5`.
                    .child(
                        div()
                            .id("git-commit-btn")
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .justify_center()
                            .gap_1p5()
                            .rounded_l(px(6.0))
                            .bg(fill)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(bg)
                            .when(can_commit, |el| {
                                el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                                    this.commit_from_panel(
                                        PendingCommit { push: false, pr: false },
                                        false,
                                        false,
                                        cx,
                                    )
                                }))
                            })
                            .child(Icon::new(IconName::Check).size(IconSize::Sm).color(bg))
                            .child(if amend { "Amend Commit" } else { "Commit" }),
                    )
                    // MonoCode's `h-7 w-7 rounded-r-md border-l
                    // border-background-base/10` arrow: the Commit fill,
                    // `hover:bg-content/80` (or `hover:bg-content` while
                    // Commit is off), `aria-expanded:bg-content`, and only
                    // `pointer-events-none` when disabled.
                    .child(
                        div().w(px(28.0)).flex_none().child(
                            self.git_menu_trigger(
                                GitMenuKind::Commit,
                                TriggerLook {
                                    icon: IconName::ChevronDown,
                                    size: IconSize::Sm,
                                    spin: false,
                                    color: bg,
                                    hover_color: bg,
                                    fill: Some(fill),
                                    hover: if can_commit { fg.opacity(0.8) } else { fg },
                                    open: fg,
                                    dim_disabled: false,
                                },
                                can_open_menu,
                                cx,
                            )
                            .rounded_r(px(6.0))
                            .border_l_1()
                            .border_color(bg.opacity(0.1)),
                        ),
                    ),
            )
            .children(self.render_sync_actions(cx))
    }

    /// MonoCode `GitSyncActions`: Publish or Sync, Create PR, View PR.
    fn render_sync_actions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let sync = &self.git_sync;
        sync.remote.as_ref()?;
        let busy = self.changes_ui.busy.clone();
        let can_publish = sync.upstream.is_none();
        let can_sync = sync.upstream.is_some() && (sync.ahead > 0 || sync.behind > 0);
        let show_create = !self.has_open_pr() && !self.on_default_branch();
        let pr = self.changes_ui.pr.clone().filter(|p| p.state == "open");
        if !can_publish && !can_sync && !show_create && pr.is_none() {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (ahead, behind) = (sync.ahead, sync.behind);
        let dest = sync.upstream.clone().unwrap_or_else(|| {
            format!(
                "{}/{}",
                sync.remote.clone().unwrap_or_else(|| "origin".into()),
                sync.branch.clone().unwrap_or_else(|| "HEAD".into())
            )
        });
        let syncing = busy == Some(Busy::Sync);
        let sync_title = if syncing {
            "Synchronizing Changes...".to_string()
        } else if can_publish {
            sync.branch
                .as_ref()
                .map_or("Publish Branch".into(), |b| format!("Publish Branch \"{b}\""))
        } else if ahead > 0 && behind > 0 {
            format!("Pull {behind} and push {ahead} commits between {dest}")
        } else if behind > 0 {
            format!("Pull {behind} commit{} from {dest}", if behind == 1 { "" } else { "s" })
        } else {
            format!("Push {ahead} commit{} to {dest}", if ahead == 1 { "" } else { "s" })
        };
        let button = |id: &'static str, enabled: bool, tip: String| {
            let hover = fg.opacity(0.15);
            div()
                .id(id)
                .flex()
                .h(px(28.0))
                .w_full()
                .min_w_0()
                .items_center()
                .justify_center()
                .gap_1p5()
                .px_2()
                .rounded(px(6.0))
                .bg(fg.opacity(0.10))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg)
                .tooltip(Tooltip::text(tip))
                .when(!enabled, |el| el.opacity(0.4))
                .when(enabled, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        };
        // `size-3.5` icons; MonoCode's loaders spin (`animate-spin`).
        let icon = |name: IconName| Icon::new(name).size(IconSize::Sm).color(fg).into_any_element();
        let spinner =
            |id: &'static str, name: IconName| spinning_icon(id.into(), name, IconSize::Sm, fg);
        let count = |text: String| div().flex_none().text_color(fg.opacity(0.55)).child(text);
        let mut col = div().flex().flex_col().gap(px(6.0));
        if can_publish {
            col = col.child(
                button("git-publish", busy.is_none(), sync_title)
                    .when(busy.is_none(), |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.sync_changes(cx)))
                    })
                    .child(if syncing {
                        spinner("git-publish-spin", IconName::LoaderCircle)
                    } else {
                        icon(IconName::CloudUpload)
                    })
                    .child(div().min_w_0().truncate().child("Publish Branch")),
            );
        } else if can_sync {
            col = col.child(
                button("git-sync", busy.is_none(), sync_title)
                    .when(busy.is_none(), |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.sync_changes(cx)))
                    })
                    // MonoCode spins the `RefreshCw` itself.
                    .child(if syncing {
                        spinner("git-sync-spin", IconName::RefreshCw)
                    } else {
                        icon(IconName::RefreshCw)
                    })
                    .child(div().min_w_0().truncate().child("Sync Changes"))
                    .when(behind > 0, |el| el.child(count(format!("↓{behind}"))))
                    .when(ahead > 0, |el| el.child(count(format!("↑{ahead}")))),
            );
        }
        if show_create {
            let enabled = self.can_create_pr() && busy.is_none();
            let tip = sync.default_branch.as_ref().map_or("Create pull request".into(), |b| {
                format!("Create a pull request into {b}")
            });
            col = col.child(
                button("git-create-pr", enabled, tip)
                    .when(enabled, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.create_pr(false, cx)))
                    })
                    .child(if busy == Some(Busy::Pr) {
                        spinner("git-create-pr-spin", IconName::LoaderCircle)
                    } else {
                        icon(IconName::GitPullRequest)
                    })
                    .child("Create PR"),
            );
        }
        if let Some(pr) = pr {
            let url = pr.url.clone();
            col = col.child(
                button(
                    "git-view-pr",
                    busy.is_none(),
                    format!("View PR #{}: {}", pr.number, pr.title),
                )
                .when(busy.is_none(), |el| el.on_click(move |_, _, cx| cx.open_url(&url)))
                .child(icon(IconName::ExternalLink))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(format!("View PR #{}", pr.number)),
                ),
            );
        }
        Some(col.into_any_element())
    }

    /// The staged and unstaged sections, or what there is instead.
    fn render_change_sections(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let sync = &self.git_sync;
        let staged = &self.git_status.staged;
        let unstaged = &self.git_status.unstaged;
        let body = if staged.is_empty() && unstaged.is_empty() {
            let text = if sync.ahead > 0 && sync.behind > 0 {
                format!(
                    "Diverged from {}",
                    sync.upstream.clone().unwrap_or_else(|| "upstream".into())
                )
            } else if sync.ahead > 0 {
                format!(
                    "{} unpushed commit{}",
                    sync.ahead,
                    if sync.ahead == 1 { "" } else { "s" }
                )
            } else if sync.behind > 0 {
                format!(
                    "{} incoming commit{}",
                    sync.behind,
                    if sync.behind == 1 { "" } else { "s" }
                )
            } else {
                "No uncommitted changes".into()
            };
            div()
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.45))
                .child(text)
                .into_any_element()
        } else {
            // Only the rows in view are built (a big checkout can list
            // thousands of files); spacers stand in for the rest.
            let tree_of = |side: Side, files: &[GitFileChange]| {
                (self.changes_ui.tree && self.section_open(side) && !files.is_empty()).then(|| build_tree(files))
            };
            let (staged_tree, unstaged_tree) = (tree_of(Side::Staged, staged), tree_of(Side::Unstaged, unstaged));
            let mut items = Vec::new();
            if !staged.is_empty() {
                self.push_section(Side::Staged, staged, staged_tree.as_ref(), &mut items);
            }
            if !unstaged.is_empty() {
                self.push_section(Side::Unstaged, unstaged, unstaged_tree.as_ref(), &mut items);
            }
            let heights: Vec<Option<f32>> = items.iter().map(|item| Some(item.height())).collect();
            let visible = virtual_rows::for_scroll(&heights, &self.changes_ui.scroll, CHANGES_PAD_Y);
            div()
                .when(visible.above > 0.0, |el| el.child(div().h(px(visible.above))))
                .children(items[visible.range.clone()].iter().map(|item| self.render_change_item(item, cx)))
                .when(visible.below > 0.0, |el| el.child(div().h(px(visible.below))))
                .into_any_element()
        };
        scrollbar::framed(
            "git-changes-scrollbar",
            &self.changes_ui.scroll,
            div()
                .id("git-changes-scroll")
                .track_scroll(&self.changes_ui.scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(CHANGES_PAD_Y))
                .pr(scrollbar::gutter(&self.changes_ui.scroll))
                .child(body),
        )
    }

    /// MonoCode `FileSection`'s header: chevron, title, count pill and
    /// actions.
    fn render_section_header(
        &self,
        side: Side,
        files: &[GitFileChange],
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let ui = &self.changes_ui;
        let open = self.section_open(side);
        let title = match side {
            Side::Staged => "STAGED CHANGES",
            Side::Unstaged => "CHANGES",
        };
        let id = match side {
            Side::Staged => "staged",
            Side::Unstaged => "unstaged",
        };
        let tree = ui.tree;
        // MonoCode `IconAction`: `size-5 rounded text-content/55
        // hover:bg-content/10 hover:text-content`, `size-3.5` icons.
        let tint = fg.opacity(0.55);
        let action_group = |key: &str| SharedString::from(format!("git-{id}-{key}"));
        // In `tint`, `fg` while its button is hovered (MonoCode
        // `hover:text-content`).
        let glyph = |key: &str, icon: Icon| {
            icon.size(IconSize::Sm)
                .color(tint)
                .group_hover_color(action_group(key), fg)
                .into_any_element()
        };
        let named = |key: &str, icon: IconName| glyph(key, Icon::new(icon));
        let extra = |key: &str, icon: ExtraIcon| glyph(key, icon.icon());
        let action = |key: &str, icon: AnyElement, tip: &'static str| {
            let hover = fg.opacity(0.10);
            div()
                .id(action_group(key))
                .group(action_group(key))
                .size(px(20.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(Tooltip::text(tip))
                .child(icon)
        };
        let mut header = div()
            .flex()
            .items_center()
            .gap_1()
            .h(px(SECTION_HEADER_HEIGHT))
            .px(px(6.0))
            .child(
                div()
                    .id(SharedString::from(format!("git-{id}-toggle")))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match side {
                            Side::Staged => this.changes_ui.staged_closed = !this.changes_ui.staged_closed,
                            Side::Unstaged => {
                                this.changes_ui.changes_closed = !this.changes_ui.changes_closed
                            }
                        }
                        cx.notify();
                    }))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.5)),
                    )
                    // `text-[10px] font-semibold tracking-[0.04em]
                    // text-content/55 uppercase` (GPUI has no letter spacing).
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg.opacity(0.55))
                            .child(title),
                    )
                    .child(
                        div()
                            .ml_1()
                            .h(px(16.0))
                            .min_w(px(16.0))
                            .px_1()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(colors.accent.opacity(0.8))
                            // `text-[8px]`
                            .text_size(px(8.0))
                            .text_color(gpui::white())
                            .child(files.len().to_string()),
                    ),
            )
            .child(
                action(
                    "view",
                    if tree {
                        named("view", IconName::List)
                    } else {
                        extra("view", ExtraIcon::FolderTree)
                    },
                    if tree { "View as List" } else { "View as Tree" },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.changes_ui.tree = !this.changes_ui.tree;
                    this.save_settings(cx);
                    cx.notify();
                })),
            )
            .child(
                action("open-all", extra("open-all", ExtraIcon::FileDiff), "Open All Changes").on_click(cx.listener(
                    move |this, _, _, cx| this.open_all_changes(side, cx),
                )),
            );
        header = match side {
            Side::Staged => header.child(
                action("unstage-all", named("unstage-all", IconName::Minus), "Unstage All Changes")
                    .on_click(cx.listener(|this, _, _, cx| this.all_action(false, cx))),
            ),
            Side::Unstaged => header
                .child(
                    action("discard-all", named("discard-all", IconName::Undo2), "Discard All Changes").on_click(
                        cx.listener(|this, _, _, cx| {
                            if !this.git_status.unstaged.is_empty() && this.changes_ui.busy.is_none()
                            {
                                this.git_confirm = Some(GitConfirm::DiscardAll);
                                cx.notify();
                            }
                        }),
                    ),
                )
                .child(
                    action("stage-all", named("stage-all", IconName::Plus), "Stage All Changes")
                        .on_click(cx.listener(|this, _, _, cx| this.all_action(true, cx))),
                ),
        };
        header.into_any_element()
    }

    /// Whether `side`'s section shows its files.
    fn section_open(&self, side: Side) -> bool {
        match side {
            Side::Staged => !self.changes_ui.staged_closed,
            Side::Unstaged => !self.changes_ui.changes_closed,
        }
    }

    /// `side`'s header and, while open, its rows, in display order.
    fn push_section<'a>(
        &self,
        side: Side,
        files: &'a [GitFileChange],
        tree: Option<&'a ChangeDir>,
        out: &mut Vec<ChangeItem<'a>>,
    ) {
        out.push(ChangeItem::Header { side, files });
        if !self.section_open(side) {
            return;
        }
        match tree {
            Some(root) => self.push_tree(root, 0, side, out),
            None => out.extend(files.iter().map(|file| ChangeItem::File {
                file,
                side,
                depth: None,
            })),
        }
    }

    fn push_tree<'a>(&self, dir: &'a ChangeDir, depth: usize, side: Side, out: &mut Vec<ChangeItem<'a>>) {
        for child in &dir.dirs {
            let key = format!("{side:?}:{}", child.path);
            let open = !self.changes_ui.collapsed_dirs.contains(&key);
            out.push(ChangeItem::Dir {
                dir: child,
                depth,
                open,
                key,
                side,
            });
            if open {
                self.push_tree(child, depth + 1, side, out);
            }
        }
        out.extend(dir.files.iter().map(|file| ChangeItem::File {
            file,
            side,
            depth: Some(depth),
        }));
    }

    /// Tracks the row under the pointer in `hovered_row`.
    fn row_hover(
        row: SharedString,
        cx: &Context<Self>,
    ) -> impl Fn(&bool, &mut gpui::Window, &mut gpui::App) + 'static {
        cx.listener(move |this, hovered: &bool, _, cx| {
            let slot = &mut this.changes_ui.hovered_row;
            if *hovered {
                *slot = Some(row.clone());
            } else if slot.as_ref() == Some(&row) {
                *slot = None;
            } else {
                return;
            }
            cx.notify();
        })
    }

    fn render_change_item(&self, item: &ChangeItem<'_>, cx: &Context<Self>) -> AnyElement {
        match item {
            ChangeItem::Header { side, files } => self.render_section_header(*side, files, cx),
            ChangeItem::Dir {
                dir,
                depth,
                open,
                key,
                side,
            } => self.render_dir_row(dir, *depth, *open, key.clone(), *side, cx),
            ChangeItem::File { file, side, depth } => self.render_change_row(file, *side, *depth, cx),
        }
    }

    /// MonoCode `ChangeDirRow`.
    fn render_dir_row(
        &self,
        dir: &ChangeDir,
        depth: usize,
        open: bool,
        key: String,
        side: Side,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let busy = self.changes_ui.busy.is_some();
        let icon = crate::ui::file_tree::resolve_entry_icon(&dir.name, true, open);
        let group = SharedString::from(format!("git-dir-{key}"));
        let folder = dir.path.clone();
        let (verb, action_icon) = match side {
            Side::Staged => ("Unstage", IconName::Minus),
            Side::Unstaged => ("Stage", IconName::Plus),
        };
        let action_hover = fg.opacity(0.10);
        let action_group = SharedString::from(format!("{group}-stage"));
        // `hidden group-hover:flex`: no room is kept while hidden.
        let hovered = self.changes_ui.hovered_row.as_ref() == Some(&group);
        let folder_action = div()
            .flex_none()
            .child(
                div()
                    .id(action_group.clone())
                    .group(action_group.clone())
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .when(busy, |el| el.opacity(0.4))
                    .when(!busy, |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.bg(action_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.folder_action(folder.clone(), side, cx)
                            }))
                    })
                    .tooltip(Tooltip::text(format!("{verb} Changes in {}", dir.path)))
                    .child(
                        Icon::new(action_icon)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.55))
                            .group_hover_color(action_group, fg),
                    ),
            );
        let dot = dir
            .status
            .as_ref()
            .map_or(fg.opacity(0.4), |status| status_color(status, crate::ui::appearance::diff_colors(cx)));
        let hover = fg.opacity(0.05);
        // `group flex h-7 items-center gap-1 pr-2 leading-none text-content
        // hover:bg-content/5`, indented `8 + depth * 12`.
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .items_center()
            .gap_1()
            .h(px(ROW_HEIGHT))
            .pl(px(8.0 + depth as f32 * 12.0))
            .pr_2()
            .line_height(relative(1.0))
            .text_color(fg)
            .hover(move |s| s.bg(hover))
            .on_hover(Self::row_hover(group.clone(), cx))
            .child(
                // The toggle: `flex-1 gap-1.5`, a `size-4` chevron box,
                // the 16px folder icon, `text-[13px] font-medium`.
                div()
                    .id(SharedString::from(format!("{group}-toggle")))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .tooltip(Tooltip::text(dir.path.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.changes_ui.collapsed_dirs.remove(&key) {
                            this.changes_ui.collapsed_dirs.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div().size(px(16.0)).flex().flex_none().items_center().justify_center().child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.5)),
                        ),
                    )
                    .child(icon.size(IconSize::Md))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(dir.name.clone())),
                    ),
            )
            .when(hovered, |el| el.child(folder_action))
            // `w-3.5` with a `size-1.5` dot in the shared status colour.
            .child(
                div().w(px(14.0)).flex().flex_none().justify_center().child(
                    div().size(px(6.0)).rounded_full().bg(dot),
                ),
            )
            .into_any_element()
    }

    /// MonoCode `ChangeRow`: icon, name (and folder in list view), the
    /// actions on hover or while open, the status letter.
    fn render_change_row(
        &self,
        file: &GitFileChange,
        side: Side,
        depth: Option<usize>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let name = basename(&file.path).to_string();
        let dir = if depth.is_none() { dirname(&file.path) } else { "" };
        let active = matches!(
            self.file_pane.active(),
            Some(PaneTab::Review { cwd, path, side: open })
                if *cwd == self.workspace.cwd && *path == file.path && *open == side
        );
        let busy = self.changes_ui.busy.is_some();
        let icon = crate::ui::file_tree::resolve_entry_icon(&name, false, false);
        let group = SharedString::from(format!("git-row-{side:?}-{}", file.path));
        // MonoCode `bg-selection`.
        let selection = colors.active;
        let hover = fg.opacity(0.05);
        let action = |key: &str, icon: IconName, tip: &'static str| {
            let action_hover = fg.opacity(0.10);
            let action_group = SharedString::from(format!("{group}-{key}"));
            div()
                .id(action_group.clone())
                .group(action_group.clone())
                .size(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .when(busy, |el| el.opacity(0.4))
                .when(!busy, |el| el.cursor_pointer().hover(move |s| s.bg(action_hover)))
                .tooltip(Tooltip::text(tip))
                .child(
                    Icon::new(icon)
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.55))
                        .group_hover_color(action_group, fg),
                )
        };
        let (open_path, discard_path, toggle_path) =
            (file.path.clone(), file.path.clone(), file.path.clone());
        // `flex` while open, else `hidden group-hover:flex`: no room is
        // kept while hidden.
        let show_actions = active || self.changes_ui.hovered_row.as_ref() == Some(&group);
        let mut actions = div().flex().flex_none().items_center();
        if side == Side::Unstaged {
            actions = actions.child(
                action("discard", IconName::Undo2, "Discard Changes").when(!busy, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.file_action(discard_path.clone(), side, true, cx)
                    }))
                }),
            );
        }
        actions = actions.child(
            match side {
                Side::Staged => action("unstage", IconName::Minus, "Unstage Changes"),
                Side::Unstaged => action("stage", IconName::Plus, "Stage Changes"),
            }
            .when(!busy, |el| {
                el.on_click(cx.listener(move |this, _, _, cx| {
                    this.file_action(toggle_path.clone(), side, false, cx)
                }))
            }),
        );
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .items_center()
            .gap_1()
            .h(px(ROW_HEIGHT))
            .pr_2()
            // `leading-none text-content`
            .line_height(relative(1.0))
            .text_color(fg)
            .map(|el| match depth {
                Some(depth) => el.pl(px(8.0 + depth as f32 * 12.0)),
                None => el.pl_2(),
            })
            .when(active, |el| el.bg(selection))
            .when(!active, |el| el.hover(move |s| s.bg(hover)))
            .on_hover(Self::row_hover(group.clone(), cx))
            .child(
                div()
                    .id(SharedString::from(format!("{group}-open")))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .tooltip(Tooltip::text(file.path.clone()))
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        this.open_change(open_path.clone(), side, event.click_count() == 2, cx)
                    }))
                    .when(depth.is_some(), |el| el.child(div().size(px(16.0)).flex_none()))
                    // `FileTypeIcon size={16}`
                    .child(icon.size(IconSize::Md))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name),
                                    )
                                    .when(!dir.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.0))
                                                .text_color(fg.opacity(0.4))
                                                .child(dir.to_string()),
                                        )
                                    }),
                            ),
                    ),
            )
            .when(show_actions, |el| el.child(actions))
            .child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .text_right()
                    .font_family(cx.theme().mono_family.clone())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(status_color(&file.status, crate::ui::appearance::diff_colors(cx)))
                    .child(status_letter(&file.status)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_follow_monocode() {
        assert_eq!(status_letter(&GitFileStatus::Untracked), "U");
        assert_eq!(status_letter(&GitFileStatus::Renamed), "M");
        assert_eq!(dirname("a/b/c.rs"), "a/b");
        assert_eq!(dirname("c.rs"), "");
    }
}
