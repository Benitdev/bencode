//! Left column in MonoCode's workspace layout: header, Sessions / Explorer /
//! Changes switcher, thread search with filters, and the thread cards.

use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, div, prelude::*, relative,
};

use crate::app::{BenCodeApp, SidebarMode};
use crate::ui::scale::px;
use crate::ui::window_drag::claim_press;
use crate::ui::app_callback::app_callback;
use crate::ui::diff_counts::diff_counts;

/// MonoCode `border-stroke`: content at 7%.
pub const STROKE_OPACITY: f32 = 0.07;

/// MonoCode's title bar and sidebar header are `h-10`.
pub const TITLEBAR_HEIGHT: f32 = 40.0;

/// MonoCode `MIN_WIDTH` / `MAX_WIDTH` / `DEFAULT_WIDTH`; the sidebar never
/// takes more than half the window.
pub const SIDEBAR_MIN_WIDTH: f32 = 260.0;
const SIDEBAR_MAX_WIDTH: f32 = 560.0;

/// Drag payload of a workspace tab being reordered.
#[derive(Clone, Copy, Debug)]
pub struct DraggedSidebarTab(pub SidebarMode);

impl gpui::Render for DraggedSidebarTab {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// MonoCode `DEFAULT_SIDEBAR_TAB_ORDER` ids.
pub fn tab_id(tab: SidebarMode) -> &'static str {
    match tab {
        SidebarMode::Sessions => "sessions",
        SidebarMode::Files => "files",
        SidebarMode::Changes => "changes",
    }
}

/// MonoCode `loadSidebarTabOrder`: known ids in order, missing ones
/// appended, anything else the default.
pub fn parse_tab_order(ids: &[String]) -> Vec<SidebarMode> {
    const ALL: [SidebarMode; 3] = [SidebarMode::Sessions, SidebarMode::Files, SidebarMode::Changes];
    let mut order: Vec<SidebarMode> = Vec::new();
    for id in ids {
        if let Some(tab) = ALL.into_iter().find(|t| tab_id(*t) == id)
            && !order.contains(&tab)
        {
            order.push(tab);
        }
    }
    for tab in ALL {
        if !order.contains(&tab) {
            order.push(tab);
        }
    }
    order
}

/// Drag payload of the sidebar's resize sash.
#[derive(Clone, Debug)]
pub struct SidebarResize {
    pub start_x: f32,
    pub start_width: f32,
}

impl gpui::Render for SidebarResize {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The width a drag to `x` gives, clamped (MonoCode `useSidebarResize`).
pub fn resized_width(drag: &SidebarResize, x: f32, window_width: f32) -> f32 {
    let max = SIDEBAR_MAX_WIDTH.min(window_width * 0.5).max(SIDEBAR_MIN_WIDTH);
    (drag.start_width + x - drag.start_x).round().clamp(SIDEBAR_MIN_WIDTH, max)
}

/// A thread-level dialog opened from a card's context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDialog {
    Delete(String),
    /// Every thread of a title-bar tab.
    DeleteMany(Vec<String>),
    /// The cards picked in the sidebar.
    DeleteSelected(Vec<String>),
}

impl BenCodeApp {
    /// Sessions | Explorer | Changes, in the user's order (MonoCode drags
    /// them to reorder and remembers it); Changes shows its diffstat.
    pub fn render_sidebar_mode_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let files = self
            .git_status
            .staged
            .iter()
            .chain(&self.git_status.unstaged);
        let (added, removed) = files.fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let mode = self.sidebar_mode;
        let accent = colors.accent;
        let tabs = self.sidebar_tab_order.iter().map(|&tab| {
            let (id, label) = match tab {
                SidebarMode::Sessions => ("tab-sessions", "Sessions"),
                SidebarMode::Files => ("tab-files", "Explorer"),
                SidebarMode::Changes => ("tab-changes", "Changes"),
            };
            self.mode_tab(id, mode == tab, cx)
                .map(|el| {
                    // MonoCode `DiffStat`: 11px semibold, gap-1.5; the
                    // label is `leading-label`.
                    if tab == SidebarMode::Changes && added + removed > 0 {
                        el.child(diff_counts(added, removed, crate::ui::appearance::diff_colors(cx)).gap_1p5().text_size(px(11.0)))
                    } else {
                        el.child(div().min_w_0().truncate().line_height(px(12.0 * 1.4)).child(label))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.show_sidebar(tab, cx);
                    if tab == SidebarMode::Changes {
                        this.refresh_workspace(cx);
                    }
                }))
                .on_drag(DraggedSidebarTab(tab), |dragged, _, _, cx| {
                    let dragged = dragged.clone();
                    cx.new(|_| dragged)
                })
                .drag_over::<DraggedSidebarTab>(move |style, _, _, _| style.bg(accent.opacity(0.15)))
                .on_drop(cx.listener(move |this, dragged: &DraggedSidebarTab, _, cx| {
                    this.move_sidebar_tab(dragged.0, tab, cx);
                }))
        });

        div()
            .flex()
            .items_center()
            .gap(px(1.0))
            .flex_none()
            .h(px(36.0))
            .px_2()
            .border_b_1()
            .border_color(colors.fg.opacity(STROKE_OPACITY))
            .children(tabs)
    }

    /// Drops tab `from` where `to` is (MonoCode `useAnimatedReorder`).
    fn move_sidebar_tab(&mut self, from: SidebarMode, to: SidebarMode, cx: &mut Context<Self>) {
        let order = &mut self.sidebar_tab_order;
        let (Some(a), Some(b)) = (
            order.iter().position(|t| *t == from),
            order.iter().position(|t| *t == to),
        ) else {
            return;
        };
        if a == b {
            return;
        }
        let tab = order.remove(a);
        order.insert(b, tab);
        self.save_settings(cx);
        cx.notify();
    }

    fn mode_tab(
        &self,
        id: &'static str,
        active: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        // MonoCode: h-6 rounded-md px-2 text-[12px] leading-none, the
        // active one on the selection fill, the others at half strength.
        div()
            .id(id)
            .flex()
            .flex_1()
            .min_w_0()
            .h(px(24.0))
            .items_center()
            .justify_center()
            .px_2()
            .rounded(px(6.0))
            .text_size(px(12.0))
            .line_height(relative(1.0))
            .text_color(if active { colors.fg } else { colors.fg.opacity(0.5) })
            .when(active, |el| el.bg(colors.active))
    }

    /// MonoCode clears the search, the picks and the filter popover when
    /// the Sessions tab is left.
    pub(crate) fn show_sidebar(&mut self, mode: SidebarMode, cx: &mut Context<Self>) {
        if self.sidebar_mode == SidebarMode::Sessions && mode != SidebarMode::Sessions {
            self.search_input.update(cx, |input, cx| input.set_text("", cx));
            self.search_query.clear();
            self.sessions_ui.selection.clear();
            self.close_sidebar_menu(cx);
            self.cancel_inline_rename(cx);
        }
        self.sidebar_mode = mode;
        cx.notify();
    }

    pub fn start_session_rename(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let title = crate::app::session_list::display_title(&session.title, &session.harness);
        self.sessions_ui.renaming_folder = None;
        self.sessions_ui.renaming_session = Some(id.to_string());
        self.begin_inline_rename(title, cx);
    }

    pub fn start_folder_rename(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        let Some(name) = self
            .project_folders()
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.clone())
        else {
            return;
        };
        self.sessions_ui.renaming_session = None;
        self.sessions_ui.renaming_folder = Some(folder_id.to_string());
        self.begin_inline_rename(name, cx);
    }

    /// Fills the shared field, selects it and focuses it.
    fn begin_inline_rename(&mut self, text: String, cx: &mut Context<Self>) {
        let len = text.len();
        self.rename_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.select(0..len, cx);
        });
        let handle = gpui::Focusable::focus_handle(self.rename_input.read(cx), cx);
        crate::ui::composer::focus_later(handle, cx);
        cx.notify();
    }

    pub fn inline_rename_active(&self) -> bool {
        self.sessions_ui.renaming_session.is_some() || self.sessions_ui.renaming_folder.is_some()
    }

    pub fn cancel_inline_rename(&mut self, cx: &mut Context<Self>) {
        if self.sessions_ui.renaming_session.take().is_some()
            | self.sessions_ui.renaming_folder.take().is_some()
        {
            cx.notify();
        }
    }

    /// Enter or blur: a non-blank name is kept, a blank one cancels
    /// (MonoCode `SessionRenameRow` / `FolderRenameRow`).
    pub fn commit_inline_rename(&mut self, cx: &mut Context<Self>) {
        let text = self.rename_input.read(cx).text().trim().to_string();
        let session = self.sessions_ui.renaming_session.take();
        let folder = self.sessions_ui.renaming_folder.take();
        if !text.is_empty() {
            if let Some(id) = session {
                self.rename_session(&id, &text, cx);
            } else if let Some(id) = folder {
                self.rename_folder(&id, &text, cx);
            }
        }
        cx.notify();
    }

    pub fn rename_session(&mut self, id: &str, title: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        session.title = title.trim().to_string();
        self.persist_session(id);
        cx.notify();
    }

    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        // MonoCode `body-glass` is the base background, edged with
        // `border-stroke`; text inherits the web's 1.5 line height.
        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(px(self.sidebar_width))
            .h_full()
            .bg(glass.body(colors.bg))
            .border_r_1()
            .border_color(colors.fg.opacity(STROKE_OPACITY))
            .line_height(relative(1.5))
            .child(self.render_sidebar_header(cx))
            .child(self.render_sidebar_mode_tabs(cx))
            .child(match self.sidebar_mode {
                SidebarMode::Sessions => self.render_session_list(cx).into_any_element(),
                SidebarMode::Files => self.render_file_tree(cx).into_any_element(),
                SidebarMode::Changes => self.render_git_changes_panel(cx).into_any_element(),
            })
            .child(self.render_sidebar_sash(cx))
    }

    /// MonoCode's resize separator on the sidebar's right edge; a double
    /// click restores the default width.
    fn render_sidebar_sash(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let dragging = self.sidebar_resizing;
        let width = self.sidebar_width;
        div()
            .id("sidebar-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-1.0))
            .w(px(6.0))
            .cursor_col_resize()
            .when(dragging, |el| el.bg(fg.opacity(0.15)))
            .when(!dragging, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.sidebar_width = SIDEBAR_MIN_WIDTH;
                        cx.notify();
                    }
                    this.sidebar_drag_x = crate::ui::scale::logical(event.position.x);
                }),
            )
            .on_drag(SidebarResize { start_x: 0.0, start_width: width }, |drag, _, _, cx| {
                let drag = drag.clone();
                cx.new(|_| drag)
            })
    }

    fn render_sidebar_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let valid_worktrees: Vec<&crate::git::Worktree> = self
            .workspace
            .worktrees
            .iter()
            .filter(|w| !w.missing)
            .collect();
        let has_worktrees = valid_worktrees.iter().any(|w| !w.is_main);

        // MonoCode: `h-10 gap-1 pl-3 pr-1.5`, 40px like the title bar, and
        // takes the traffic-light space when the project rail is hidden.
        self.window_drag_region(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(TITLEBAR_HEIGHT))
            .pl_3()
            .pr_1p5()
            .border_b_1()
            .border_color(colors.fg.opacity(STROKE_OPACITY))
            // The icon rail (or the title bar above it) holds that space.
            .when(!self.is_rail_open && !self.compact_rail_active() && cfg!(target_os = "macos"), |el| {
                el.child(div().flex_none().w(gpui::px(78.0))) // MonoCode `w-[78px]`
            })
            .child(
                div().flex().flex_1().min_w_0().items_center().child(
                    if has_worktrees || self.worktree_focus().is_some() {
                        self.render_worktree_switcher(cx).into_any_element()
                    } else {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.0))
                            .line_height(relative(1.25))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Workspace")
                            .into_any_element()
                    },
                ),
            )
            // MonoCode `WorkspaceTitleActions`: `gap-0.5`.
            .child(
                claim_press(div())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.0))
                    .child(title_icon_button(
                        "workspace-search",
                        IconName::Search,
                        "Go to File (⌘P)",
                        cx,
                        cx.listener(|this, _, _, cx| this.open_quick_open(cx)),
                    ))
                    .child(title_icon_button(
                        "workspace-new-thread",
                        IconName::Plus,
                        "New session (⌘T)",
                        cx,
                        cx.listener(|this, _, _, cx| this.create_new_session(cx)),
                    )),
            )
    }

    /// MonoCode `SidebarWorktreeSwitcher`'s title button: the focused
    /// worktree's branch, else "Workspace", over the working copies.
    fn render_worktree_switcher(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let focus = self.worktree_focus();
        let title = focus.map_or("Workspace".to_string(), |f| {
            f.branch.clone().unwrap_or_else(|| "Detached worktree".to_string())
        });
        let tip = match focus {
            Some(f) => format!("{}\n{}", f.branch.as_deref().unwrap_or("detached"), f.path),
            None => self
                .workspace
                .worktrees
                .iter()
                .find(|w| w.is_main)
                .and_then(|w| w.branch.clone())
                .unwrap_or_else(|| "Project folder".into()),
        };
        let open = self.sidebar_menu_is_worktrees();
        // MonoCode: `-ml-1.5 h-6.5 gap-2 rounded-md px-1.5 text-sm
        // font-medium leading-tight`, the chevrons `size-3.5`.
        claim_press(div())
            .id("worktree-switcher")
            .ml(px(-6.0))
            .flex()
            .min_w_0()
            .max_w_full()
            .items_center()
            .gap_2()
            .h(px(26.0))
            .px(px(6.0))
            .rounded(px(6.0))
            .text_size(px(14.0))
            .line_height(relative(1.25))
            .font_weight(FontWeight::MEDIUM)
            .when(open, |el| el.bg(fg.opacity(0.08)))
            .hover(move |s| s.bg(fg.opacity(0.08)))
            .tooltip(Tooltip::text(tip))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.sessions_ui.filter_button_hit = true;
                    this.toggle_worktree_menu(event.position, cx);
                }),
            )
            .child(div().min_w_0().truncate().child(title))
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Sm)
                    .color(fg.opacity(0.45)),
            )
    }

    /// The rename or delete dialog, when one is open.
    pub fn render_session_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.session_dialog.clone()?;
        let close = app_callback(cx, |this, cx| {
            this.session_dialog = None;
            cx.notify();
        });
        Some(match dialog {
            SessionDialog::Delete(id) => {
                let title = self
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map_or("this session".into(), |s| {
                        crate::app::session_list::display_title(&s.title, &s.harness)
                    });
                let delete = app_callback(cx, move |this, cx| this.delete_session(&id, cx));
                // MonoCode `DeleteSessionDialog`.
                ConfirmDialog::new(
                    "delete-thread",
                    "Delete session?",
                    format!("“{title}” will be permanently deleted."),
                    close,
                )
                .confirm("Delete")
                .destructive()
                .on_confirm(delete)
                .into_any_element()
            }
            SessionDialog::DeleteSelected(ids) => {
                let count = ids.len();
                let delete = app_callback(cx, move |this, cx| {
                    for id in &ids {
                        this.delete_session(id, cx);
                    }
                });
                ConfirmDialog::new(
                    "delete-selected-threads",
                    "Delete sessions?",
                    format!("Delete {count} selected conversations? This can’t be undone."),
                    close,
                )
                .confirm("Delete")
                .destructive()
                .on_confirm(delete)
                .into_any_element()
            }
            SessionDialog::DeleteMany(ids) => {
                let count = ids.len();
                let delete = app_callback(cx, move |this, cx| {
                    for id in &ids {
                        this.delete_session(id, cx);
                    }
                });
                ConfirmDialog::new(
                    "delete-threads",
                    format!("Delete {count} threads?"),
                    format!("All {count} conversations in this tab and their transcripts will be removed."),
                    close,
                )
                .confirm("Delete")
                .destructive()
                .on_confirm(delete)
                .into_any_element()
            }
        })
    }
}

/// MonoCode `TitleBar` `IconButton`: `size-6.5 rounded-md`, a `size-3.5`
/// glyph at half strength that lights up with a `content/10` fill on hover.
fn title_icon_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    let group = SharedString::from(id);
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .hover(move |s| s.bg(fg.opacity(0.10)))
        .tooltip(Tooltip::text(tip))
        .on_click(on_click)
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(fg.opacity(0.5))
                .group_hover_color(group, fg),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_orders_keep_known_ids_and_fill_in_the_rest() {
        let order = parse_tab_order(&["changes".into(), "bogus".into(), "changes".into()]);
        assert_eq!(order, [SidebarMode::Changes, SidebarMode::Sessions, SidebarMode::Files]);
        assert_eq!(parse_tab_order(&[]).len(), 3);
    }

    #[test]
    fn resizing_is_clamped_to_half_the_window() {
        let drag = SidebarResize { start_x: 100.0, start_width: 260.0 };
        assert_eq!(resized_width(&drag, 200.0, 2000.0), 360.0);
        assert_eq!(resized_width(&drag, 900.0, 2000.0), 560.0);
        assert_eq!(resized_width(&drag, 900.0, 700.0), 350.0);
        assert_eq!(resized_width(&drag, 0.0, 2000.0), 260.0);
    }
}
