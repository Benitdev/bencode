//! MonoCode `Sidebar.tsx` Sessions tab: the search row with its filter
//! button, then the list (folders, the Pinned group, loose threads) of
//! session cards. Cards open on click, ⌘-click / ⇧-click pick several for
//! the context menu, drag onto a folder or another card to group them, or
//! onto a pane to split it.

use std::collections::BTreeMap;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::app::session_list::{
    Groups, ListEntry, LiveStates, SessionFilters, Selection, build_list, compare_sessions,
    matches_query, navigation_ids, passes_filters,
};
use crate::db::SessionRow;
use crate::ui::icons::ExtraIcon;
use crate::ui::sidebar_folders::SessionGroup;

/// MonoCode `SESSION_INSERT_WINDOW_MS`.
const INSERT_WINDOW_MS: i64 = 15_000;
/// MonoCode `ParticleText`'s `SWEEP_MS`.
pub const TITLE_SWEEP: std::time::Duration = std::time::Duration::from_millis(1100);

/// MonoCode re-reads card ages every 30s while the tab is open.
pub const AGE_TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// Where a dragged card would land in the list (MonoCode
/// `SessionListDropTarget`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListDrop {
    Folder(String),
    Session(String),
}

/// A title that just changed (MonoCode `ParticleText`).
#[derive(Clone, Debug)]
pub struct TitleChange {
    pub old: String,
    pub at: std::time::Instant,
    /// Keys the sweep's animation, so it plays once.
    pub serial: u64,
}

/// The Sessions tab's state.
#[derive(Default)]
pub struct SessionsUi {
    /// MonoCode `monocode.sessionSidebarFilters`, saved in settings.
    pub filters: SessionFilters,
    /// The thread list's scroll, for its scroll bar.
    pub scroll: gpui::ScrollHandle,
    pub selection: Selection,
    /// The card or folder showing an inline rename field.
    pub renaming_session: Option<String>,
    pub renaming_folder: Option<String>,
    /// MonoCode `monocode.pinnedSessionsCollapsed`, per project.
    pub pinned_collapsed: BTreeMap<String, bool>,
    /// MonoCode `monocode.reminderSessionsCollapsed`, per project.
    pub reminders_collapsed: BTreeMap<String, bool>,
    /// Card order as last drawn, for ⇧-click ranges and menu targets.
    pub order: Vec<String>,
    pub drop: Option<ListDrop>,
    /// Threads each project has shown, so only new ones slide in
    /// (MonoCode `SessionInsertMotion`).
    pub seen: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// Cards that appeared new and slide in.
    pub sliding: std::collections::HashSet<String>,
    /// Titles that just changed: the old one and when (MonoCode
    /// `ParticleText`).
    pub title_changes: std::collections::HashMap<String, TitleChange>,
    title_serial: u64,
    /// The title each card showed last.
    pub shown_titles: std::collections::HashMap<String, String>,
    /// Orchestrator task rows opened in the cards, `<lead>:<index>`.
    pub open_agents: std::collections::HashSet<String>,
    /// The folder menu's custom colour picker is showing.
    pub folder_color_picker: bool,
    /// A custom folder colour was picked and not saved yet.
    pub folder_colors_unsaved: bool,
    /// The filter button saw this mouse-down (so it is not "outside").
    pub filter_button_hit: bool,
}

impl BenCodeApp {
    /// Redraws the cards' ages every 30s while the Sessions tab shows.
    pub fn start_session_age_tick(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AGE_TICK).await;
                let ticked = this.update(cx, |app, cx| {
                    if app.sidebar_mode == crate::app::SidebarMode::Sessions {
                        cx.notify();
                    }
                });
                if ticked.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// MonoCode's busy, approval and unseen-finished ids.
    pub(crate) fn session_live_states(&self) -> LiveStates {
        LiveStates {
            busy: self.runs.keys().cloned().collect(),
            approval: self
                .runs
                .iter()
                .filter(|(_, run)| run.pending_permission.is_some())
                .map(|(id, _)| id.clone())
                .collect(),
            done: self.title_strip.unseen_finished.clone(),
        }
    }

    pub(crate) fn pinned_collapsed(&self) -> bool {
        self.group_collapsed(&self.sessions_ui.pinned_collapsed)
    }

    fn group_collapsed(&self, map: &BTreeMap<String, bool>) -> bool {
        map.get(&self.current_cwd).copied().unwrap_or(false)
    }

    /// Folds or opens the Reminders group (saved per project).
    pub(crate) fn toggle_reminders_group(&mut self, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        let map = &mut self.sessions_ui.reminders_collapsed;
        if map.remove(&project).is_none() {
            map.insert(project, true);
        }
        self.save_settings(cx);
        cx.notify();
    }

    pub(crate) fn toggle_pinned_group(&mut self, cx: &mut Context<Self>) {
        let collapsed = !self.pinned_collapsed();
        let project = self.current_cwd.clone();
        if collapsed {
            self.sessions_ui.pinned_collapsed.insert(project, true);
        } else {
            self.sessions_ui.pinned_collapsed.remove(&project);
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// The project's threads that are listed at all: sent at least once
    /// (MonoCode `has_user_message`) or kept in a folder, in the focused
    /// worktree.
    pub(crate) fn listed_sessions(&self) -> Vec<&SessionRow> {
        let folders = self.project_folders();
        let in_folder = |id: &str| folders.iter().any(|f| f.session_ids.iter().any(|s| s == id));
        let cwd = &self.current_cwd;
        let focus = self.worktree_focus();
        self.sessions
            .iter()
            .filter(|s| crate::app::same_project_path(&s.cwd, cwd))
            .filter(|s| s.has_user_message() || in_folder(&s.id))
            .filter(|s| {
                focus.is_none_or(|focus| crate::app::same_project_path(s.work_dir(), &focus.path))
            })
            .collect()
    }

    /// MonoCode's visible sessions: filters, then search, then sorted.
    fn visible_sessions<'a>(&self, listed: &[&'a SessionRow], states: &LiveStates) -> Vec<&'a SessionRow> {
        let now = crate::app::now_ms();
        let repo = self.workspace.repo.as_deref();
        let mut visible: Vec<&SessionRow> = listed
            .iter()
            .copied()
            .filter(|s| passes_filters(s, &self.sessions_ui.filters, states, now))
            .filter(|s| matches_query(s, &self.search_query, repo))
            .collect();
        visible.sort_by(|a, b| compare_sessions(a, b));
        visible
    }

    /// Loads folder members older than the project's loaded threads, and
    /// drops members the database no longer has (MonoCode
    /// `pruneSessionFolders`).
    pub fn load_folder_members(&mut self, cx: &mut Context<Self>) {
        self.load_sessions_in_background(None, cx);
    }

    pub fn render_session_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors.clone();
        if matches!(self.current_cwd.trim(), "" | "~") {
            return quiet_line("No project folder", colors.fg.opacity(0.5)).into_any_element();
        }
        let states = self.session_live_states();
        let listed = self.listed_sessions();
        let visible = self.visible_sessions(&listed, &states);
        let searching = !self.search_query.trim().is_empty();
        let narrowed = searching || self.sessions_ui.filters.is_active();
        let folders = self.project_folders();
        let reminder_ids: Vec<String> = self.reminders.iter().map(|r| r.session_id.clone()).collect();
        let groups = Groups {
            pinned_collapsed: self.pinned_collapsed(),
            reminders_collapsed: self.group_collapsed(&self.sessions_ui.reminders_collapsed),
            reminder_ids: &reminder_ids,
        };
        let entries = build_list(&visible, folders, groups);
        let order = navigation_ids(&entries, searching);
        let now = crate::app::now_ms();
        let body = if visible.is_empty() {
            if narrowed {
                quiet_line(
                    if searching {
                        "No matching sessions"
                    } else {
                        "No sessions match these filters"
                    },
                    colors.fg.opacity(0.5),
                )
                .into_any_element()
            } else {
                sessions_empty(cx).into_any_element()
            }
        } else {
            let rows: Vec<AnyElement> = entries
                .iter()
                .enumerate()
                .map(|(ix, entry)| {
                    let before_loose = matches!(entries.get(ix + 1), Some(ListEntry::Session(_)));
                    self.render_list_entry(entry, before_loose, searching, &states, now, cx)
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .p(px(6.0))
                .children(rows)
                .into_any_element()
        };
        let shown: Vec<(String, String, i64)> = visible
            .iter()
            .map(|s| {
                let title = crate::app::session_list::display_title(&s.title, &s.harness);
                (s.id.clone(), title, s.created_at)
            })
            .collect();
        self.note_card_motion(&shown, cx);
        let mut selection_changed = false;
        if self.sessions_ui.order != order {
            let before = self.sessions_ui.selection.clone();
            self.sessions_ui.selection.prune(&order);
            selection_changed = before != self.sessions_ui.selection;
            self.sessions_ui.order = order;
        }
        if selection_changed {
            cx.notify();
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_session_search(cx))
            .child(crate::ui::scrollbar::framed(
                "thread-list-scrollbar",
                &self.sessions_ui.scroll,
                div()
                    .id("thread-list")
                    .key_context("SessionList")
                    .track_focus(&self.session_list_focus)
                    .track_scroll(&self.sessions_ui.scroll)
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .pr(crate::ui::scrollbar::gutter())
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .child(body),
            ))
            .into_any_element()
    }

    fn render_list_entry(
        &self,
        entry: &ListEntry<'_>,
        before_loose: bool,
        searching: bool,
        states: &LiveStates,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        match entry {
            ListEntry::Session(session) => self.render_session_card(session, false, states, now, cx),
            ListEntry::Pinned {
                collapsed,
                sessions,
            }
            | ListEntry::Reminders {
                collapsed,
                sessions,
            } => {
                let kind = if matches!(entry, ListEntry::Pinned { .. }) {
                    SessionGroup::Pinned
                } else {
                    SessionGroup::Reminders
                };
                let expanded = searching || !collapsed;
                let cards = expanded.then(|| {
                    sessions
                        .iter()
                        .map(|s| self.render_session_card(s, true, states, now, cx))
                        .collect()
                });
                self.render_session_group(kind, sessions, cards, before_loose, searching, states, cx)
            }
            ListEntry::Folder { folder, sessions } => {
                let expanded = searching || !folder.collapsed;
                let cards = expanded.then(|| {
                    sessions
                        .iter()
                        .map(|s| self.render_session_card(s, true, states, now, cx))
                        .collect()
                });
                self.render_session_folder(folder, sessions, cards, before_loose, searching, states, cx)
            }
        }
    }

    /// The search field and the filter button (MonoCode `h-9` row).
    fn render_session_search(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let active = self.sessions_ui.filters.is_active() || self.sidebar_menu_is_filter();
        // MonoCode `SessionsHeaderButton`: `size-6 rounded-md`, the glyph
        // at half strength until hovered (`content/10`), opened or active
        // (`bg-selection`).
        let filter_group = SharedString::from("filter-sessions");
        let filter_glyph = |color: gpui::Hsla| ExtraIcon::ListFilter.icon().size(IconSize::Xs).color(color);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(36.0))
            .px_2()
            .border_b_1()
            .border_color(fg.opacity(crate::ui::sidebar::STROKE_OPACITY))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.0))
                    .items_center()
                    .child(
                        div()
                            .absolute()
                            .left(px(8.0))
                            .child(Icon::new(IconName::Search).size(IconSize::Xs).color(fg.opacity(0.5))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(28.0))
                            .pr_2()
                            .text_size(px(12.0))
                            .text_color(fg)
                            .child(self.search_input.clone()),
                    ),
            )
            .child(
                div()
                    .id("filter-sessions")
                    .group(filter_group.clone())
                    .relative()
                    .size(px(24.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .when(active, |el| el.bg(colors.active))
                    .when(!active, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
                    .tooltip(Tooltip::text("Filter sessions"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.sessions_ui.filter_button_hit = true;
                            this.toggle_filter_menu(event.position, cx);
                        }),
                    )
                    .map(|el| {
                        if active {
                            el.child(filter_glyph(fg))
                        } else {
                            el.child(
                                div()
                                    .group_hover(filter_group.clone(), |s| s.invisible())
                                    .child(filter_glyph(fg.opacity(0.5))),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .invisible()
                                    .group_hover(filter_group.clone(), |s| s.visible())
                                    .child(filter_glyph(fg)),
                            )
                        }
                    }),
            )
    }

    /// Records which cards are new to this project (they slide in) and
    /// which titles changed (they sweep in).
    fn note_card_motion(&mut self, shown: &[(String, String, i64)], cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        let now = crate::app::now_ms();
        // A project's first listing is just the list; nothing slides.
        let first_look = !self.sessions_ui.seen.contains_key(&project);
        let seen = self.sessions_ui.seen.entry(project).or_default();
        for (id, _, created_at) in shown {
            if seen.insert(id.clone()) && !first_look && now - created_at < INSERT_WINDOW_MS {
                self.sessions_ui.sliding.insert(id.clone());
            }
        }
        let mut changed = false;
        for (id, title, _) in shown {
            match self.sessions_ui.shown_titles.insert(id.clone(), title.clone()) {
                Some(old) if old != *title => {
                    self.sessions_ui.title_serial += 1;
                    let change = TitleChange {
                        old,
                        at: std::time::Instant::now(),
                        serial: self.sessions_ui.title_serial,
                    };
                    self.sessions_ui.title_changes.insert(id.clone(), change);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            // Clear the finished sweeps once they are done.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TITLE_SWEEP).await;
                let cleared = this.update(cx, |app, cx| {
                    app.sessions_ui
                        .title_changes
                        .retain(|_, change| change.at.elapsed() < TITLE_SWEEP);
                    cx.notify();
                });
                if let Err(err) = cleared {
                    log::debug!("title sweep after app drop: {err:#}");
                }
            })
            .detach();
        }
    }

    /// MonoCode `SESSION_INSERT_WINDOW_MS`: a thread this new, not seen
    /// in the list before, slides in.
    pub(crate) fn card_is_fresh(&self, session: &SessionRow) -> bool {
        self.sessions_ui.sliding.contains(&session.id)
            && crate::app::now_ms() - session.created_at < INSERT_WINDOW_MS
    }

    /// F2 on the picks: renames the one picked card.
    pub fn rename_selected_session(&mut self, cx: &mut Context<Self>) {
        if self.inline_rename_active() {
            return;
        }
        let ids = &self.sessions_ui.selection.ids;
        if ids.len() == 1
            && let Some(id) = ids.iter().next().cloned()
        {
            self.start_session_rename(&id, cx);
        }
    }

    /// ⌫ on the picks: deletes them, in list order.
    pub fn delete_selected_sessions(&mut self, cx: &mut Context<Self>) {
        let selection = &self.sessions_ui.selection;
        let ids: Vec<String> = self
            .sessions_ui
            .order
            .iter()
            .filter(|id| selection.ids.contains(*id))
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        self.request_delete_sessions(ids, cx);
    }

    pub(crate) fn set_list_drop(&mut self, target: ListDrop, over: bool, cx: &mut Context<Self>) {
        let current = self.sessions_ui.drop.as_ref();
        if over && current != Some(&target) {
            self.sessions_ui.drop = Some(target);
            cx.notify();
        } else if !over && current == Some(&target) {
            self.sessions_ui.drop = None;
            cx.notify();
        }
    }

    /// MonoCode `applySessionListDrop`: onto a folder (or a card in one)
    /// joins it; onto a loose card makes a new folder of the two, named
    /// inline.
    pub(crate) fn drop_on_list(&mut self, dragged: &str, target: ListDrop, cx: &mut Context<Self>) {
        use crate::app::session_folders::{FolderTarget, folder_of};
        self.sessions_ui.drop = None;
        // MonoCode keeps reminder threads out of folder drops.
        let reminded = |id: &str| self.reminders.iter().any(|r| r.session_id == id);
        if reminded(dragged) || matches!(&target, ListDrop::Session(other) if reminded(other)) {
            cx.notify();
            return;
        }
        let folders = self.project_folders().to_vec();
        match target {
            ListDrop::Folder(folder_id) => {
                self.place_session_in_folder(dragged, &FolderTarget::Existing(folder_id), cx);
            }
            ListDrop::Session(other) if other != dragged => {
                if let Some(dest) = folder_of(&folders, &other) {
                    if folder_of(&folders, dragged).map(|f| &f.id) != Some(&dest.id) {
                        let id = dest.id.clone();
                        self.place_session_in_folder(dragged, &FolderTarget::Existing(id), cx);
                    }
                } else if let Some(created) =
                    self.new_folder_with_sessions(&[dragged.to_string(), other], cx)
                {
                    self.start_folder_rename(&created, cx);
                }
            }
            ListDrop::Session(_) => {}
        }
        cx.notify();
    }
}

fn quiet_line(text: &'static str, color: gpui::Hsla) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(12.0))
        .text_color(color)
        .child(text)
}

/// MonoCode `SessionsEmpty`'s pixel terminal and its dimmer glow, on a
/// 16-column grid.
const EMPTY_TERMINAL: [&str; 12] = [
    "..############..",
    ".##..........##.",
    ".#............#.",
    ".#..#.........#.",
    ".#...#...###..#.",
    ".#..#....###..#.",
    ".#............#.",
    ".#............#.",
    ".##..........##.",
    "..############..",
    "......####......",
    "....########....",
];
const EMPTY_GLOW: [&str; 12] = [
    "#.#..........#.#",
    "................",
    "#..............#",
    "................",
    "................",
    "................",
    "................",
    "#..............#",
    "................",
    "#.#..........#.#",
    "................",
    "..#..........#..",
];
/// `w-24` across 16 cells.
const EMPTY_CELL: f32 = 96.0 / 16.0;

/// MonoCode `SessionsEmpty`: the hint (`text-[13px] leading-relaxed
/// text-content/45`) over the pixel terminal (`w-24 text-content/25`, the
/// glow at 40%), `gap-5 px-6 py-10`.
fn sessions_empty(cx: &Context<BenCodeApp>) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    let art = fg.opacity(0.25);
    let glow = fg.opacity(0.25 * 0.4);
    let rows = EMPTY_TERMINAL.iter().zip(EMPTY_GLOW).map(move |(sprite, halo)| {
        div().flex().children(sprite.bytes().zip(halo.bytes()).map(move |(s, g)| {
            let cell = div().flex_none().size(px(EMPTY_CELL));
            match (s, g) {
                (b'#', _) => cell.bg(art),
                (_, b'#') => cell.bg(glow),
                _ => cell,
            }
        }))
    });
    div()
        .flex()
        .flex_col()
        .min_h_full()
        .items_center()
        .justify_center()
        .gap_5()
        .px_6()
        .py_10()
        .child(
            div()
                .text_size(px(13.0))
                .line_height(gpui::relative(1.625))
                .text_color(fg.opacity(0.45))
                .text_center()
                .child("Sessions you start will show up here"),
        )
        .child(div().flex().flex_col().flex_none().children(rows))
}
