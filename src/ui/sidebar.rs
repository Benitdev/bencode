//! Left column in MonoCode's workspace layout: header, Sessions / Explorer /
//! Changes switcher, thread search with filters, and the thread cards.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::git::DiffStat;
use ely_gpui_component::menus::{ContextMenu, Menu, MenuItem, OverflowMenu};
use ely_gpui_component::overlays::{ConfirmDialog, PromptDialog};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, div, prelude::*, px,
};

use crate::app::{BenCodeApp, FilterMode, SidebarMode, ViewMode};
use crate::db::SessionRow;
use crate::harness::catalog;
use crate::ui::app_callback::app_callback;
use crate::ui::theme::harness_color;

const SIDEBAR_WIDTH: gpui::Pixels = px(260.0);

/// A thread-level dialog opened from a card's context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDialog {
    Rename(String),
    Delete(String),
}

const FILTERS: [(FilterMode, &str); 4] = [
    (FilterMode::All, "All threads"),
    (FilterMode::Active, "Active"),
    (FilterMode::Pinned, "Pinned"),
    (FilterMode::Archived, "Archived"),
];

fn keeps(mode: FilterMode, session: &SessionRow) -> bool {
    match mode {
        FilterMode::All => true,
        FilterMode::Active => !session.archived,
        FilterMode::Pinned => session.pinned,
        FilterMode::Archived => session.archived,
    }
}

fn matches_query(session: &SessionRow, query: &str) -> bool {
    query.is_empty() || session.title.to_lowercase().contains(query)
}

/// MonoCode's compact age label: now, 5m, 3h, 2d.
fn relative_time(updated_at: i64, now: i64) -> String {
    let diff = now.saturating_sub(updated_at);
    match diff {
        d if d < 60_000 => "now".to_string(),
        d if d < 3_600_000 => format!("{}m", d / 60_000),
        d if d < 86_400_000 => format!("{}h", d / 3_600_000),
        d => format!("{}d", d / 86_400_000),
    }
}

fn harness_glyph(harness: &str) -> &'static str {
    match harness {
        "claude" => "✴",
        "codex" => "●",
        _ => "▲",
    }
}

impl BenCodeApp {
    /// Sessions | Explorer | diffstat switcher shared by every sidebar panel.
    pub fn render_sidebar_mode_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let files = self
            .git_status
            .staged
            .iter()
            .chain(&self.git_status.unstaged);
        let (added, removed) = files.fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let mode = self.sidebar_mode;

        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(colors.border)
            .child(
                self.mode_tab("tab-sessions", mode == SidebarMode::Sessions, cx)
                    .child("Sessions")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar(SidebarMode::Sessions, ViewMode::Chat, cx)
                    })),
            )
            .child(
                self.mode_tab("tab-files", mode == SidebarMode::Files, cx)
                    .child("Explorer")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar(SidebarMode::Files, ViewMode::Editor, cx)
                    })),
            )
            .child(
                self.mode_tab("tab-changes", mode == SidebarMode::Changes, cx)
                    .map(|tab| {
                        if added + removed > 0 {
                            tab.child(DiffStat::new(added, removed))
                        } else {
                            tab.child("Changes")
                        }
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar(SidebarMode::Changes, ViewMode::Changes, cx);
                        this.refresh_workspace(cx);
                    })),
            )
    }

    fn mode_tab(
        &self,
        id: &'static str,
        active: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let colors = &theme.colors;
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .py_1()
            .rounded(theme.radius(Radius::Md))
            .cursor_pointer()
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(if active {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .text_color(if active { colors.fg } else { colors.fg_muted })
            .when(active, |el| el.bg(colors.active))
            .hover(|s| s.bg(colors.hover))
    }

    fn show_sidebar(&mut self, mode: SidebarMode, view: ViewMode, cx: &mut Context<Self>) {
        self.sidebar_mode = mode;
        self.active_view_mode = view;
        cx.notify();
    }

    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .bg(theme.colors.surface)
            .border_r_1()
            .border_color(theme.colors.border)
            .child(self.render_sidebar_header(cx))
            .child(self.render_sidebar_mode_tabs(cx))
            .child(match self.sidebar_mode {
                SidebarMode::Sessions => self.render_session_list(cx).into_any_element(),
                SidebarMode::Files => self.render_file_tree(cx).into_any_element(),
                SidebarMode::Changes => self.render_git_changes_panel(cx).into_any_element(),
            })
    }

    fn render_sidebar_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Workspace"),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        IconButton::new("workspace-search", IconName::Search)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Search (⌘K)")
                            .on_click(cx.listener(|this, _, _, cx| this.open_search_modal(cx))),
                    )
                    .child(
                        IconButton::new("workspace-new-thread", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("New thread (⌘T)")
                            .on_click(cx.listener(|this, _, _, cx| this.create_new_session(cx))),
                    ),
            )
    }

    fn render_session_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let query = self.search_query.to_lowercase();
        let filter = self.filter_mode;
        let now = crate::app::now_ms();
        let (pinned, rest): (Vec<&SessionRow>, Vec<&SessionRow>) = self
            .sessions
            .iter()
            .filter(|s| keeps(filter, s) && matches_query(s, &query))
            .partition(|s| s.pinned);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius(Radius::Md))
                            .bg(theme.colors.bg)
                            .border_1()
                            .border_color(theme.colors.border)
                            .child(
                                Icon::new(IconName::Search)
                                    .size(IconSize::Xs)
                                    .color(theme.colors.fg_muted),
                            )
                            .child(div().flex_1().min_w_0().child(self.search_input.clone())),
                    )
                    .child(self.render_filter_menu(cx)),
            )
            .child(
                div()
                    .id("thread-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .children(
                        pinned
                            .into_iter()
                            .chain(rest)
                            .map(|s| self.render_session_card(s, now, cx)),
                    ),
            )
    }

    fn render_filter_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let menu = FILTERS.iter().fold(Menu::new(), |menu, &(mode, label)| {
            menu.item(
                MenuItem::radio(label, mode == self.filter_mode).on_click(app_callback(
                    cx,
                    move |this, cx| {
                        this.filter_mode = mode;
                        cx.notify();
                    },
                )),
            )
        });
        OverflowMenu::new("thread-filter", menu)
            .icon(IconName::SlidersHorizontal)
            .tooltip("Filter threads")
    }

    fn session_menu(&self, session: &SessionRow, cx: &Context<Self>) -> Menu {
        let (rename, pin, delete) = (session.id.clone(), session.id.clone(), session.id.clone());
        Menu::new()
            .item(
                MenuItem::new("Rename…")
                    .icon(IconName::Pencil)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.open_rename(&rename, cx)
                    })),
            )
            .item(
                MenuItem::new(if session.pinned { "Unpin" } else { "Pin" })
                    .icon(IconName::Pin)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.toggle_pin_session(&pin, cx)
                    })),
            )
            .separator()
            .item(
                MenuItem::new("Delete…")
                    .icon(IconName::Trash2)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.session_dialog = Some(SessionDialog::Delete(delete.clone()));
                        cx.notify();
                    })),
            )
    }

    fn render_session_card(
        &self,
        session: &SessionRow,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let selected = self.selected_session_id.as_deref() == Some(session.id.as_str());
        let title = if session.title.trim().is_empty() {
            "New session"
        } else {
            session.title.as_str()
        };
        let branch = session.branch.as_deref().unwrap_or(&self.git_status.branch);
        let project = std::path::Path::new(&session.cwd)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let id = session.id.clone();
        let xs = theme.text_size(TextSize::Xs);

        let card = div()
            .id(SharedString::from(format!("session-card-{}", session.id)))
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2p5()
            .py_2()
            .rounded(theme.radius(Radius::Md))
            .cursor_pointer()
            .when(selected, |el| el.bg(colors.active))
            .hover(|s| s.bg(colors.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select_session(id.clone(), cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1p5()
                    .text_size(xs)
                    .text_color(colors.fg_muted)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(harness_color(&session.harness, colors))
                                    .child(harness_glyph(&session.harness)),
                            )
                            .child(div().truncate().child(catalog::label_for(&session.model))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .when(session.pinned, |el| {
                                el.child(
                                    Icon::new(IconName::Pin)
                                        .size(IconSize::Xs)
                                        .color(colors.fg_muted),
                                )
                            })
                            .child(relative_time(session.updated_at, now)),
                    ),
            )
            .child(
                div()
                    .truncate()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.fg)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(xs)
                    .text_color(colors.fg_muted)
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(IconSize::Xs)
                            .color(colors.fg_muted),
                    )
                    .child(div().truncate().child(format!("{project}/{branch}"))),
            );

        ContextMenu::new(
            SharedString::from(format!("session-menu-{}", session.id)),
            self.session_menu(session, cx),
        )
        .child(card)
        .into_any_element()
    }

    pub fn open_rename(&mut self, id: &str, cx: &mut Context<Self>) {
        let title = self
            .sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.title.clone())
            .unwrap_or_default();
        self.rename_input
            .update(cx, |input, cx| input.set_text(title, cx));
        self.session_dialog = Some(SessionDialog::Rename(id.to_string()));
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

    /// The rename or delete dialog, when one is open.
    pub fn render_session_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.session_dialog.clone()?;
        let close = app_callback(cx, |this, cx| {
            this.session_dialog = None;
            cx.notify();
        });
        Some(match dialog {
            SessionDialog::Rename(id) => {
                let submit = cx
                    .listener(move |this, title: &str, _, cx| this.rename_session(&id, title, cx));
                PromptDialog::new("rename-thread", "Rename thread", &self.rename_input, close)
                    .label("Title")
                    .submit("Rename")
                    .check(|text| {
                        if text.trim().is_empty() {
                            Err("A title is required".into())
                        } else {
                            Ok(())
                        }
                    })
                    .on_submit(move |title: &str, window: &mut Window, cx: &mut App| {
                        submit(title, window, cx)
                    })
                    .into_any_element()
            }
            SessionDialog::Delete(id) => {
                let title = self
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map_or("this thread".into(), |s| s.title.clone());
                let delete = app_callback(cx, move |this, cx| this.delete_session(&id, cx));
                ConfirmDialog::new(
                    "delete-thread",
                    "Delete thread?",
                    format!("“{title}” and its transcript will be removed."),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_select_the_right_sessions() {
        let pinned = SessionRow {
            pinned: true,
            ..Default::default()
        };
        let archived = SessionRow {
            archived: true,
            ..Default::default()
        };
        assert!(keeps(FilterMode::Pinned, &pinned) && !keeps(FilterMode::Pinned, &archived));
        assert!(keeps(FilterMode::Archived, &archived) && !keeps(FilterMode::Active, &archived));
        assert!(keeps(FilterMode::All, &archived));
    }

    #[test]
    fn relative_time_buckets() {
        assert_eq!(relative_time(0, 30_000), "now");
        assert_eq!(relative_time(0, 5 * 60_000), "5m");
        assert_eq!(relative_time(0, 3 * 3_600_000), "3h");
        assert_eq!(relative_time(0, 2 * 86_400_000), "2d");
    }

    #[test]
    fn query_matches_titles_case_insensitively() {
        let s = SessionRow {
            title: "Fix Login".into(),
            ..Default::default()
        };
        assert!(matches_query(&s, "login") && matches_query(&s, "") && !matches_query(&s, "auth"));
    }
}
