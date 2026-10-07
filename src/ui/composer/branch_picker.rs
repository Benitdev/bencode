//! MonoCode `BranchPicker` and `WorktreeBasePicker`: a 280px popover rising
//! from its chip, with a search field, the branches (remote ones badged),
//! an inline git error, and, for the branch picker, "Create and checkout
//! …" / "New branch". Keyboard: ↑/↓ (clamped), Enter, Esc.

use ely_gpui_component::overlays::PromptDialog;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, anchored, deferred, div, prelude::*,
};

use crate::ui::scale::px;

use super::focus_later;
use super::menus::popover_surface;
use crate::app::BenCodeApp;
use crate::app::workspace_sync::BranchTarget;
use crate::git;
use crate::ui::app_callback::app_callback;
use crate::ui::sidebar_popovers::popover_glass;

/// MonoCode `MENU_WIDTH`, `MENU_MIN_HEIGHT`, `MENU_MAX_HEIGHT`.
const MENU_WIDTH: f32 = 280.0;
const MENU_MIN_HEIGHT: f32 = 180.0;
const MENU_MAX_HEIGHT: f32 = 280.0;
const ROW_HEIGHT: f32 = 32.0;

/// Which chip the popover belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchPickerKind {
    /// Switch or create the checkout's branch.
    Branch,
    /// The branch a new worktree starts from.
    Base,
}

/// The open popover's highlight, and a switch in flight or its failure.
#[derive(Clone, Debug, Default)]
pub struct BranchPickerUi {
    pub active: usize,
    pub busy: bool,
    pub error: Option<String>,
}

/// One branch row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRow {
    pub branch: git::Branch,
    /// The name shown: a remote branch without its remote.
    pub label: String,
    /// The remote's name, shown as a badge.
    pub remote: Option<String>,
    pub selected: bool,
}

/// MonoCode `rows`: branches whose name (and remote) contain `query`.
pub fn branch_rows(branches: &[git::Branch], query: &str, current: &str) -> Vec<BranchRow> {
    let needle = query.trim().to_lowercase();
    branches
        .iter()
        .filter_map(|branch| {
            let (remote, label) = match branch.name.split_once('/').filter(|_| branch.remote) {
                Some((remote, name)) => (Some(remote.to_string()), name.to_string()),
                None => (None, branch.name.clone()),
            };
            let hay = match &remote {
                Some(remote) => format!("{label} {remote}"),
                None => label.clone(),
            };
            (needle.is_empty() || hay.to_lowercase().contains(&needle)).then(|| BranchRow {
                selected: !branch.remote && branch.name == current,
                branch: branch.clone(),
                label,
                remote,
            })
        })
        .collect()
}

/// MonoCode `WorktreeBasePicker` rows: each ref once, matched on its name.
pub fn base_rows(branches: &[git::Branch], query: &str, base: &str) -> Vec<BranchRow> {
    let needle = query.trim().to_lowercase();
    let mut seen = std::collections::HashSet::new();
    branches
        .iter()
        .filter(|branch| seen.insert(branch.name.clone()))
        .filter(|branch| needle.is_empty() || branch.name.to_lowercase().contains(&needle))
        .map(|branch| BranchRow {
            selected: branch.name == base,
            branch: branch.clone(),
            label: branch.name.clone(),
            remote: None,
        })
        .collect()
}

/// MonoCode `createRow`: the typed name, unless a local branch has it.
pub fn create_name(branches: &[git::Branch], query: &str) -> Option<String> {
    let name = query.trim();
    let taken = branches.iter().any(|b| !b.remote && b.name == name);
    (!taken).then(|| name.to_string())
}

impl BenCodeApp {
    fn open_picker_kind(&self) -> Option<BranchPickerKind> {
        if self.is_branch_picker_open {
            Some(BranchPickerKind::Branch)
        } else if self.is_base_picker_open {
            Some(BranchPickerKind::Base)
        } else {
            None
        }
    }

    /// The branch the focused checkout is on.
    fn current_branch(&self) -> String {
        self.selected_session()
            .and_then(|s| s.branch.clone())
            .or_else(|| Some(self.git_status.branch.clone()).filter(|b| !b.is_empty()))
            .unwrap_or_default()
    }

    fn picker_rows(&self, kind: BranchPickerKind, cx: &App) -> Vec<BranchRow> {
        let query = self.branch_search_input.read(cx).text();
        match kind {
            BranchPickerKind::Branch => {
                branch_rows(&self.workspace.branches, query, &self.current_branch())
            }
            BranchPickerKind::Base => base_rows(
                &self.workspace.branches,
                query,
                self.new_worktree_base().unwrap_or("HEAD"),
            ),
        }
    }

    /// Opens (or closes) a branch popover, empty and focused.
    pub(super) fn toggle_branch_picker(&mut self, kind: BranchPickerKind, cx: &mut Context<Self>) {
        let open = self.open_picker_kind() != Some(kind);
        self.close_composer_popovers(cx);
        if !open {
            self.refocus_prompt(cx);
            return;
        }
        match kind {
            BranchPickerKind::Branch => self.is_branch_picker_open = true,
            BranchPickerKind::Base => self.is_base_picker_open = true,
        }
        self.branch_picker = BranchPickerUi::default();
        let placeholder = match kind {
            BranchPickerKind::Branch => "Search or create a branch...",
            BranchPickerKind::Base => "Search base branches…",
        };
        self.branch_search_input.update(cx, |input, cx| {
            input.set_text("", cx);
            input.set_placeholder(placeholder, cx);
        });
        focus_later(
            gpui::Focusable::focus_handle(self.branch_search_input.read(cx), cx),
            cx,
        );
        cx.notify();
    }

    /// The search changed: back to the top, the error cleared.
    pub fn on_branch_query_changed(&mut self, cx: &mut Context<Self>) {
        self.branch_picker.active = 0;
        self.branch_picker.error = None;
        cx.notify();
    }

    /// Closes the popover; with `refocus` the prompt takes the keyboard.
    pub fn close_branch_picker(&mut self, refocus: bool, cx: &mut Context<Self>) {
        self.is_branch_picker_open = false;
        self.is_base_picker_open = false;
        self.branch_picker = BranchPickerUi::default();
        if refocus {
            self.refocus_prompt(cx);
        }
        cx.notify();
    }

    /// ↑/↓, Enter and Esc in the popover's search.
    pub fn branch_picker_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(kind) = self.open_picker_kind() else {
            return false;
        };
        let rows = self.picker_rows(kind, cx);
        let last = rows.len().saturating_sub(1);
        match key {
            "up" => self.branch_picker.active = self.branch_picker.active.saturating_sub(1),
            "down" => self.branch_picker.active = (self.branch_picker.active + 1).min(last),
            "enter" => match rows.get(self.branch_picker.active) {
                Some(row) => self.pick_branch_row(kind, row.clone(), cx),
                None if kind == BranchPickerKind::Branch => {
                    let query = self.branch_search_input.read(cx).text().to_string();
                    if let Some(name) = create_name(&self.workspace.branches, &query)
                        .filter(|name| !name.is_empty())
                    {
                        self.create_from_picker(name, cx);
                    }
                }
                None => {}
            },
            "escape" => self.close_branch_picker(true, cx),
            _ => return false,
        }
        self.picker_scroll.scroll_to_item(self.branch_picker.active);
        cx.notify();
        true
    }

    fn pick_branch_row(&mut self, kind: BranchPickerKind, row: BranchRow, cx: &mut Context<Self>) {
        if self.branch_picker.busy {
            return;
        }
        match kind {
            BranchPickerKind::Base => {
                self.set_worktree_base(&row.branch.name, cx);
                self.close_branch_picker(true, cx);
            }
            BranchPickerKind::Branch if row.selected => self.close_branch_picker(true, cx),
            BranchPickerKind::Branch => {
                self.branch_picker.busy = true;
                self.branch_picker.error = None;
                self.switch_to_branch(BranchTarget::Existing(row.branch), false, cx);
            }
        }
    }

    /// "Create and checkout name", or with no name the New branch dialog.
    fn create_from_picker(&mut self, name: String, cx: &mut Context<Self>) {
        if self.branch_picker.busy {
            return;
        }
        if name.is_empty() {
            self.close_branch_picker(false, cx);
            self.branch_create_input
                .update(cx, |input, cx| input.set_text("", cx));
            self.branch_create_open = true;
            cx.notify();
            return;
        }
        self.branch_picker.busy = true;
        self.branch_picker.error = None;
        self.switch_to_branch(BranchTarget::New(name), false, cx);
    }

    /// A switch started from the popover ended (MonoCode `run`): done
    /// closes it, a failure stays in it.
    pub fn finish_branch_switch(&mut self, error: Option<String>, cx: &mut Context<Self>) -> bool {
        if !self.is_branch_picker_open {
            return false;
        }
        match error {
            None => self.close_branch_picker(true, cx),
            Some(error) => {
                self.branch_picker.busy = false;
                self.branch_picker.error = Some(error);
                focus_later(
                    gpui::Focusable::focus_handle(self.branch_search_input.read(cx), cx),
                    cx,
                );
                cx.notify();
            }
        }
        true
    }

    /// The popover hanging above its chip (MonoCode `Popover side="top"`),
    /// to be placed at the chip's top-left corner.
    pub(super) fn render_branch_popover(
        &self,
        kind: BranchPickerKind,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let ui = &self.branch_picker;
        let rows = self.picker_rows(kind, cx);
        let query = self.branch_search_input.read(cx).text().to_string();
        let selection = fg.opacity(if cx.theme().is_dark() { 0.10 } else { 0.06 });
        let search = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(10.0))
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::Search)
                    .size(IconSize::Xs)
                    .color(fg.opacity(0.5)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(if kind == BranchPickerKind::Base {
                        12.0
                    } else {
                        13.0
                    }))
                    .when(ui.busy, |el| el.opacity(0.6))
                    .child(self.branch_search_input.clone()),
            );
        let empty = rows.is_empty().then(|| {
            let text = match kind {
                BranchPickerKind::Base => "No matching branches",
                _ if query.trim().is_empty() => "No branches",
                _ => "No matching branches",
            };
            div()
                .px_3()
                .py_4()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.5))
                .child(text)
        });
        let busy = ui.busy;
        let list = rows.into_iter().enumerate().map(|(ix, row)| {
            let highlighted = ix == ui.active;
            let hover = fg.opacity(0.05);
            let picked = row.clone();
            div()
                .id(("branch-row", ix))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h(px(ROW_HEIGHT))
                .px_2()
                .rounded(px(8.0))
                .text_size(px(13.0))
                .text_color(fg)
                .when(highlighted || row.selected, |el| el.bg(selection))
                .when(!highlighted && !row.selected, |el| {
                    el.hover(move |s| s.bg(hover))
                })
                .when(busy, |el| el.opacity(0.6))
                .when(!busy, |el| {
                    el.cursor_pointer()
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.branch_picker.active != ix {
                                this.branch_picker.active = ix;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_branch_row(kind, picked.clone(), cx)
                        }))
                })
                .child(if row.selected {
                    Icon::new(IconName::Check).size(IconSize::Xs).color(fg)
                } else {
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.5))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(row.selected, |el| el.font_weight(FontWeight::MEDIUM))
                        .child(SharedString::from(row.label)),
                )
                .children(row.remote.map(|remote| {
                    div()
                        .flex_none()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(4.0))
                        .bg(fg.opacity(0.06))
                        .text_size(px(10.0))
                        .text_color(fg.opacity(0.4))
                        .child(SharedString::from(remote))
                }))
        });
        let error = ui.error.clone().map(|error| {
            div()
                .id("branch-picker-error")
                .flex_none()
                .max_h(px(64.0))
                .overflow_y_scroll()
                .px(px(10.0))
                .py_2()
                .border_t_1()
                .border_color(colors.border)
                .text_size(px(11.0))
                .line_height(px(16.0))
                .text_color(colors.danger.opacity(0.9))
                .child(error)
        });
        let create = (kind == BranchPickerKind::Branch)
            .then(|| create_name(&self.workspace.branches, &query))
            .flatten()
            .map(|name| {
                let label = if name.is_empty() {
                    "New branch".to_string()
                } else {
                    format!("Create and checkout {name}")
                };
                let hover = fg.opacity(0.08);
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(colors.border)
                    .p_1()
                    .child(
                        div()
                            .id("branch-picker-create")
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .h(px(30.0))
                            .px(px(10.0))
                            .rounded(px(8.0))
                            .text_size(px(13.0))
                            .text_color(fg.opacity(0.75))
                            .when(busy, |el| el.opacity(0.6))
                            .when(!busy, |el| {
                                el.cursor_pointer()
                                    .hover(move |s| s.bg(hover).text_color(fg))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.create_from_picker(name.clone(), cx)
                                    }))
                            })
                            .child(Icon::new(IconName::Plus).size(IconSize::Sm))
                            .child(div().min_w_0().truncate().child(label)),
                    )
            });
        let panel = div()
            .id("composer-branch-popover")
            .w(px(MENU_WIDTH))
            .min_h(px(MENU_MIN_HEIGHT))
            .max_h(px(MENU_MAX_HEIGHT))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(colors.border)
            .bg(popover_glass(cx))
            .shadow_xl()
            .child(search)
            .child(
                div()
                    .id("branch-picker-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.picker_scroll)
                    .p(px(6.0))
                    .children(list)
                    .children(empty),
            )
            .children(error)
            .children(create);
        // Rising from the chip's top-left corner; the chip's top bar clips,
        // so the popover is drawn deferred, over everything.
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(
                deferred(
                    anchored()
                        .anchor(gpui::Anchor::BottomLeft)
                        .offset(gpui::point(px(0.0), px(-4.0)))
                        .snap_to_window()
                        .child(popover_surface(panel, cx)),
                )
                .with_priority(3),
            )
            .into_any_element()
    }

    /// MonoCode `CreateBranchDialog`: "New branch" with no name typed.
    pub fn render_branch_create_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.branch_create_open {
            return None;
        }
        let close = app_callback(cx, |this, cx| {
            this.branch_create_open = false;
            this.refocus_prompt(cx);
            cx.notify();
        });
        let submit = cx.listener(|this, name: &str, _, cx| {
            this.branch_create_open = false;
            this.switch_to_branch(BranchTarget::New(name.trim().to_string()), false, cx);
        });
        Some(
            PromptDialog::new(
                "composer-new-branch",
                "New branch",
                &self.branch_create_input,
                close,
            )
            .label("Branch name")
            .submit("Create and checkout")
            .check(|text| {
                if text.trim().is_empty() {
                    Err("A name is required".into())
                } else {
                    Ok(())
                }
            })
            .on_submit(move |name: &str, window: &mut Window, cx: &mut App| {
                submit(name, window, cx)
            })
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, remote: bool) -> git::Branch {
        git::Branch {
            name: name.into(),
            remote,
            current: false,
        }
    }

    #[test]
    fn rows_badge_remotes_and_match_name_or_remote() {
        let branches = [
            branch("main", false),
            branch("feature/x", false),
            branch("origin/feature/x", true),
        ];
        let rows = branch_rows(&branches, "", "main");
        assert!(rows[0].selected);
        assert_eq!(rows[2].label, "feature/x");
        assert_eq!(rows[2].remote.as_deref(), Some("origin"));
        assert_eq!(branch_rows(&branches, "origin", "main").len(), 1);
        assert_eq!(branch_rows(&branches, "FEAT", "main").len(), 2);
    }

    #[test]
    fn create_row_hides_for_an_existing_local_name() {
        let branches = [branch("main", false), branch("origin/dev", true)];
        assert_eq!(create_name(&branches, " main "), None);
        assert_eq!(
            create_name(&branches, "origin/dev").as_deref(),
            Some("origin/dev")
        );
        assert_eq!(create_name(&branches, "").as_deref(), Some(""));
    }

    #[test]
    fn base_rows_are_unique_refs() {
        let branches = [
            branch("main", false),
            branch("main", false),
            branch("origin/main", true),
        ];
        let rows = base_rows(&branches, "main", "origin/main");
        assert_eq!(rows.len(), 2);
        assert!(rows[1].selected);
    }
}
