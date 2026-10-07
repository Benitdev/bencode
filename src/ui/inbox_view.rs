//! Inbox: MonoCode's review queue, GitHub source only. Open issues and pull
//! requests of the rail's projects come from the `gh` CLI; the list sits
//! left (resizable, with read state), the selected item right
//! (`detail`): its body, pull request checks and CI repair (`checks`),
//! merge and state actions (`pr_actions`), the conversation and a comment
//! box (`comments`). "Send to agent" starts a thread in the chosen project
//! with the issue as a composer card (`features/inbox/ui/InboxView.tsx`,
//! `App.tsx` `onStartInboxItem`).

mod checks;
pub mod ci_repair;
mod comments;
mod detail;
mod pr_actions;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use ely_gpui_component::buttons::{ButtonVariant, IconButton, SegmentedControl};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, prelude::*, px, uniform_list,
};

use crate::app::{BenCodeApp, Surface};
use crate::github::{self, Details, Kind, Status, WorkItem};
use crate::ui::composer::cards::ComposerCard;
use crate::ui::composer::inbox_card::{InboxCard, label_chip};

pub use checks::{Repair, RepairForm};
pub use comments::ReplyTarget;
pub use pr_actions::PrActionUi;

/// MonoCode keeps a fetched list this long before refetching on open.
const FRESH_FOR: Duration = Duration::from_secs(60);
/// MonoCode `POLL_MS`, and a pause after launch before the first poll.
const POLL_EVERY: Duration = Duration::from_secs(30);
const POLL_DELAY: Duration = Duration::from_secs(5);
/// MonoCode's list column: default and limits of its resize handle.
pub const DEFAULT_LIST_WIDTH: f32 = 340.0;
const MIN_LIST_WIDTH: f32 = 260.0;
const MAX_LIST_WIDTH: f32 = 560.0;
const ROW_HEIGHT: f32 = 76.0;

/// MonoCode's kind filter (`hiddenKinds`), as one choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KindFilter {
    #[default]
    All,
    Issues,
    PullRequests,
}

impl KindFilter {
    const ALL: [(Self, &'static str, &'static str); 3] = [
        (Self::All, "all", "All"),
        (Self::Issues, "issues", "Issues"),
        (Self::PullRequests, "prs", "Pull requests"),
    ];

    fn key(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(f, ..)| *f == self)
            .map_or("all", |e| e.1)
    }

    fn from_key(key: &str) -> Self {
        Self::ALL
            .iter()
            .find(|e| e.1 == key)
            .map_or(Self::All, |e| e.0)
    }

    fn admits(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Issues => kind == Kind::Issue,
            Self::PullRequests => kind == Kind::Pr,
        }
    }
}

/// Something loaded per item (or per job) off the UI thread.
pub struct Loaded<T> {
    pub value: HashMap<String, Result<T, String>>,
    loading: HashSet<String>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self {
            value: HashMap::new(),
            loading: HashSet::new(),
        }
    }
}

impl<T> Loaded<T> {
    pub fn get(&self, key: &str) -> Option<&Result<T, String>> {
        self.value.get(key)
    }

    pub fn is_loading(&self, key: &str) -> bool {
        self.loading.contains(key)
    }

    fn clear(&mut self) {
        self.value.clear();
        self.loading.clear();
    }
}

/// The Inbox's data: what `gh` said last, the filters and selection, what
/// was loaded for items, and the read state.
pub struct InboxState {
    /// `None` until the first fetch answers.
    pub status: Option<Status>,
    pub items: Vec<WorkItem>,
    pub errors: Vec<String>,
    pub loading: bool,
    /// A fetch (shown or the poll's) is running.
    pub fetching: bool,
    pub fetched_at: Option<Instant>,
    pub selected: Option<String>,
    pub kind: KindFilter,
    pub assigned_to_me: bool,
    pub details: Loaded<Details>,
    pub threads: Loaded<github::Thread>,
    pub checks: Loaded<github::Checks>,
    /// Actions job steps and annotations, by job id.
    pub jobs: Loaded<github::CheckDetails>,
    /// Checks opened to show their steps (`item key` + `\0` + name).
    pub expanded_checks: HashSet<String>,
    pub repair: Option<RepairForm>,
    pub repairs: Vec<Repair>,
    pub reply_to: Option<ReplyTarget>,
    pub comment_posting: bool,
    pub comment_error: Option<String>,
    pub pr: PrActionUi,
    /// The project "Send to agent" starts in, when changed from the item's.
    pub start_project: HashMap<String, String>,
    /// MonoCode `inboxSeen`: `updatedAt` (ms) of each item when read.
    pub seen: BTreeMap<String, i64>,
    pub seen_seeded: bool,
    pub list_width: f32,
    /// A resize drag: the pointer's x and the width when it began.
    resizing: Option<(f32, f32)>,
    /// Bumped by an explicit refresh: item loads from before are dropped.
    generation: u64,
    /// Bumped by every fetch: only the latest list lands.
    list_generation: u64,
    /// The list's scroll, kept on the item ↑/↓ selects.
    pub scroll: gpui::UniformListScrollHandle,
}

impl Default for InboxState {
    fn default() -> Self {
        Self {
            status: None,
            items: Vec::new(),
            errors: Vec::new(),
            loading: false,
            fetching: false,
            fetched_at: None,
            selected: None,
            kind: KindFilter::All,
            assigned_to_me: false,
            details: Loaded::default(),
            threads: Loaded::default(),
            checks: Loaded::default(),
            jobs: Loaded::default(),
            expanded_checks: HashSet::new(),
            repair: None,
            repairs: Vec::new(),
            reply_to: None,
            comment_posting: false,
            comment_error: None,
            pr: PrActionUi::default(),
            start_project: HashMap::new(),
            seen: BTreeMap::new(),
            seen_seeded: false,
            list_width: DEFAULT_LIST_WIDTH,
            resizing: None,
            generation: 0,
            list_generation: 0,
            scroll: gpui::UniformListScrollHandle::new(),
        }
    }
}

/// MonoCode `inboxUpdatedAt`.
pub fn updated_ms(item: &WorkItem) -> i64 {
    item.updated_at
        .parse::<jiff::Timestamp>()
        .map_or(0, |t| t.as_millisecond())
}

impl InboxState {
    pub fn item(&self, key: &str) -> Option<&WorkItem> {
        self.items.iter().find(|i| i.key() == key)
    }

    /// MonoCode `isInboxEntryUnseen`: newer than when it was last read
    /// (nothing is new before the first list was taken as read).
    pub fn is_unseen(&self, item: &WorkItem) -> bool {
        self.seen_seeded
            && self
                .seen
                .get(&item.key())
                .is_none_or(|was| updated_ms(item) > *was)
    }

    /// MonoCode `mergeSeen`.
    fn mark_seen(&mut self, item: &WorkItem) -> bool {
        let at = updated_ms(item);
        let entry = self.seen.entry(item.key()).or_insert(0);
        let changed = *entry < at;
        *entry = (*entry).max(at);
        changed
    }

    /// MonoCode `seedInboxSeenIfNeeded`: the first list counts as read.
    fn seed_seen(&mut self) -> bool {
        if self.seen_seeded || self.items.is_empty() {
            return false;
        }
        for item in self.items.clone() {
            self.mark_seen(&item);
        }
        self.seen_seeded = true;
        true
    }
}

/// MonoCode `formatRelativeTime` (Intl, `numeric: "auto"`), in English.
pub fn relative_time(iso: &str, now_ms: i64) -> String {
    let Ok(then) = iso.parse::<jiff::Timestamp>() else {
        return String::new();
    };
    let secs = (now_ms - then.as_millisecond()) / 1000;
    let ago = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match secs {
        i64::MIN..60 => "now".to_string(),
        60..3600 => ago(secs / 60, "minute"),
        3600..86_400 => ago(secs / 3600, "hour"),
        86_400..172_800 => "yesterday".to_string(),
        172_800..604_800 => ago(secs / 86_400, "day"),
        604_800..2_629_800 => ago(secs / 604_800, "week"),
        2_629_800..31_557_600 => ago(secs / 2_629_800, "month"),
        _ => ago(secs / 31_557_600, "year"),
    }
}

/// MonoCode `inboxStatusMark`: the glyph and its tint.
pub fn status_mark(item: &WorkItem, cx: &gpui::App) -> (IconName, gpui::Hsla, &'static str) {
    let colors = &cx.theme().colors;
    let state = item.state.to_uppercase();
    match (item.kind, state.as_str()) {
        (Kind::Pr, "MERGED") => (IconName::GitMerge, colors.accent, "Merged"),
        (Kind::Pr, "CLOSED") => (IconName::GitPullRequestClosed, colors.danger, "Closed"),
        (Kind::Pr, _) if item.draft => (
            IconName::GitPullRequestDraft,
            colors.fg.opacity(0.5),
            "Draft",
        ),
        (Kind::Pr, _) => (IconName::GitPullRequest, colors.success, "Open"),
        (Kind::Issue, "CLOSED") if item.state_reason.eq_ignore_ascii_case("completed") => {
            (IconName::CircleCheck, colors.accent, "Closed")
        }
        (Kind::Issue, "CLOSED") => (IconName::CircleX, colors.danger, "Closed"),
        (Kind::Issue, _) => (IconName::CircleDot, colors.success, "Open"),
    }
}

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Pr => "Pull request",
        Kind::Issue => "Issue",
    }
}

/// A project's folder name.
pub fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

impl BenCodeApp {
    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Inbox, cx);
        let stale = self
            .inbox
            .fetched_at
            .is_none_or(|at| at.elapsed() > FRESH_FOR);
        if stale && !self.inbox.fetching {
            self.refresh_inbox(cx);
        }
    }

    /// The rail's projects that are still folders (MonoCode
    /// `collectRailProjects`).
    pub(crate) fn inbox_projects(&self) -> Vec<String> {
        let mut projects: Vec<String> = Vec::new();
        for path in std::iter::once(&self.current_cwd).chain(self.recent_projects.iter()) {
            let real = !matches!(path.trim(), "" | "~") && std::path::Path::new(path).is_dir();
            if real
                && !projects
                    .iter()
                    .any(|p| crate::app::same_project_path(p, path))
            {
                projects.push(path.clone());
            }
        }
        projects
    }

    /// Refetches the list from `gh` off the UI thread; what was loaded for
    /// the selected item is loaded again.
    pub fn refresh_inbox(&mut self, cx: &mut Context<Self>) {
        self.fetch_inbox(false, cx);
    }

    /// MonoCode `useInboxActivity`: every 30s the list is fetched again in
    /// the background, so the rail can show new activity.
    pub fn start_inbox_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POLL_DELAY).await;
            loop {
                let polled = this.update(cx, |app, cx| {
                    if !app.inbox.fetching {
                        app.fetch_inbox(true, cx);
                    }
                });
                if polled.is_err() {
                    break;
                }
                cx.background_executor().timer(POLL_EVERY).await;
            }
        })
        .detach();
    }

    /// MonoCode `markInboxItemsSeen` over every item ("Mark all as read").
    pub fn mark_all_inbox_seen(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for item in self.inbox.items.clone() {
            changed |= self.inbox.mark_seen(&item);
        }
        if changed {
            self.save_settings(cx);
            cx.notify();
        }
    }

    /// Any listed item has activity the user has not read.
    pub fn inbox_has_unseen(&self) -> bool {
        self.inbox
            .items
            .iter()
            .any(|item| self.inbox.is_unseen(item))
    }

    /// Fetches the list. A `quiet` fetch (the poll) shows no spinner and
    /// keeps what was loaded for items; an explicit one reloads it all.
    fn fetch_inbox(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let projects = self.inbox_projects();
        let assigned = self.inbox.assigned_to_me;
        self.inbox.fetching = true;
        self.inbox.list_generation += 1;
        let list_generation = self.inbox.list_generation;
        if !quiet {
            self.inbox.loading = true;
            self.inbox.generation += 1;
        }
        let task = cx.background_executor().spawn(async move {
            match github::status() {
                Status::Ready => (Status::Ready, github::inbox(&projects, assigned)),
                other => (other, github::InboxList::default()),
            }
        });
        cx.spawn(async move |this, cx| {
            let (status, list) = task.await;
            let updated = this.update(cx, |app, cx| {
                if app.inbox.list_generation != list_generation {
                    return;
                }
                let inbox = &mut app.inbox;
                inbox.status = Some(status);
                // A linked thread's item opened before the list landed
                // stays, even when the list does not include it.
                let kept = inbox
                    .selected
                    .as_deref()
                    .and_then(|key| inbox.items.iter().find(|i| i.key() == key))
                    .filter(|item| !list.items.iter().any(|l| l.key() == item.key()))
                    .cloned();
                inbox.items = list.items;
                inbox.items.extend(kept);
                inbox.errors = list.errors;
                inbox.loading = false;
                inbox.fetching = false;
                inbox.fetched_at = Some(Instant::now());
                if !quiet {
                    inbox.details.clear();
                    inbox.threads.clear();
                    inbox.checks.clear();
                    inbox.jobs.clear();
                }
                let seeded = inbox.seed_seen();
                let gone = inbox
                    .selected
                    .as_ref()
                    .is_some_and(|key| inbox.item(key).is_none());
                if gone {
                    inbox.selected = None;
                }
                if seeded {
                    app.save_settings(cx);
                }
                if let Some(key) = app.inbox.selected.clone().filter(|_| !quiet) {
                    app.load_inbox_item(&key, cx);
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("inbox fetched after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// The list after the kind filter and the search field.
    fn shown_inbox_items(&self, cx: &gpui::App) -> Vec<usize> {
        let query = self
            .inbox_search_input
            .read(cx)
            .text()
            .trim()
            .to_lowercase();
        let number = query.trim_start_matches('#');
        self.inbox
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| self.inbox.kind.admits(item.kind))
            .filter(|(_, item)| {
                query.is_empty()
                    || item.title.to_lowercase().contains(&query)
                    || item.repo.to_lowercase().contains(&query)
                    || item.number.to_string() == number
                    || item
                        .labels
                        .iter()
                        .any(|l| l.name.to_lowercase().contains(&query))
            })
            .map(|(ix, _)| ix)
            .collect()
    }

    /// Opens Inbox item `key` (a linked thread's badge).
    pub fn select_inbox_key(&mut self, key: String, cx: &mut Context<Self>) {
        self.select_inbox_item(key, cx);
    }

    /// MonoCode `onSelect`: the item is read and opens on the right.
    fn select_inbox_item(&mut self, key: String, cx: &mut Context<Self>) {
        if self.inbox.selected.as_deref() != Some(key.as_str()) {
            self.inbox.reply_to = None;
            self.inbox.comment_error = None;
            self.inbox.pr = PrActionUi::default();
            self.inbox.repair = None;
        }
        self.inbox.selected = Some(key.clone());
        if let Some(item) = self.inbox.item(&key).cloned()
            && self.inbox.mark_seen(&item)
        {
            self.save_settings(cx);
        }
        self.load_inbox_item(&key, cx);
        cx.notify();
    }

    /// ↑/↓ (and j/k) in the focused list: the next item opens and the list
    /// scrolls to keep it in view.
    pub fn step_inbox_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let shown = self.shown_inbox_items(cx);
        if shown.is_empty() {
            return;
        }
        let current = self.inbox.selected.as_deref().and_then(|key| {
            shown
                .iter()
                .position(|ix| self.inbox.items[*ix].key() == key)
        });
        let row = match current {
            Some(row) => row.saturating_add_signed(delta).min(shown.len() - 1),
            None if delta < 0 => shown.len() - 1,
            None => 0,
        };
        let key = self.inbox.items[shown[row]].key();
        self.inbox
            .scroll
            .scroll_to_item(row, gpui::ScrollStrategy::Top);
        self.select_inbox_item(key, cx);
    }

    /// MonoCode "Mark all as read" over the listed items.
    fn mark_inbox_read(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<WorkItem> = self
            .shown_inbox_items(cx)
            .into_iter()
            .map(|ix| self.inbox.items[ix].clone())
            .collect();
        let mut changed = false;
        for item in &shown {
            changed |= self.inbox.mark_seen(item);
        }
        if changed {
            self.save_settings(cx);
        }
        cx.notify();
    }

    /// The body, the conversation and, for a pull request, its checks.
    fn load_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(item) = self.inbox.item(key).cloned() else {
            return;
        };
        load(
            self,
            |inbox| &mut inbox.details,
            key,
            {
                let item = item.clone();
                move || {
                    github::details(
                        std::path::Path::new(&item.project),
                        &item.repo,
                        item.kind,
                        item.number,
                    )
                }
            },
            cx,
        );
        self.load_inbox_thread(key, false, cx);
        if item.kind == Kind::Pr {
            self.load_pr_checks(key, false, cx);
        }
    }

    pub(crate) fn load_inbox_thread(&mut self, key: &str, force: bool, cx: &mut Context<Self>) {
        let Some(item) = self.inbox.item(key).cloned() else {
            return;
        };
        if force {
            self.inbox.threads.value.remove(key);
        }
        load(
            self,
            |inbox| &mut inbox.threads,
            key,
            move || {
                github::thread(
                    std::path::Path::new(&item.project),
                    &item.repo,
                    item.kind,
                    item.number,
                )
            },
            cx,
        );
    }

    /// MonoCode `onStartInboxItem`: a thread in the chosen project titled
    /// "#42 Title", its composer holding the issue card.
    fn start_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(item) = self.inbox.item(key).cloned() else {
            return;
        };
        let title = format!("#{} {}", item.number, item.title.trim());
        let cwd = self.inbox_start_project(&item);
        if !crate::app::same_project_path(&cwd, &self.current_cwd) {
            self.switch_project(cwd.clone(), cx);
        }
        self.close_surface(cx);
        self.open_thread_with_card(
            &cwd,
            move |session| session.title = title,
            ComposerCard::Inbox(InboxCard::from_item(&item)),
            cx,
        );
    }

    /// The project "Send to agent" (and CI repair) starts in.
    pub(crate) fn inbox_start_project(&self, item: &WorkItem) -> String {
        self.inbox
            .start_project
            .get(&item.key())
            .cloned()
            .filter(|p| std::path::Path::new(p).is_dir())
            .or_else(|| Some(item.project.clone()).filter(|p| std::path::Path::new(p).is_dir()))
            .unwrap_or_else(|| self.current_cwd.clone())
    }

    pub(crate) fn render_inbox_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let width = self.inbox.list_width;
        let resizing = self.inbox.resizing.is_some();
        let handle_hover = colors.fg.opacity(0.10);
        let handle_drag = colors.fg.opacity(0.15);
        div()
            .id("inbox")
            .size_full()
            .flex()
            // MonoCode's resize handle: the drag follows the pointer
            // anywhere over the Inbox.
            .when(resizing, |el| {
                el.cursor_col_resize()
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some((start_x, start_width)) = this.inbox.resizing {
                            let x = f32::from(event.position.x);
                            this.inbox.list_width =
                                (start_width + x - start_x).clamp(MIN_LIST_WIDTH, MAX_LIST_WIDTH);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_inbox_resize(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_inbox_resize(cx)),
                    )
            })
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .flex_none()
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(colors.border)
                    .child(self.render_inbox_toolbar(cx))
                    .child(div().flex_1().min_h_0().child(self.render_inbox_list(cx)))
                    .child(
                        div()
                            .id("inbox-resize")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(-3.0))
                            .w(px(6.0))
                            .cursor_col_resize()
                            .when(resizing, |el| el.bg(handle_drag))
                            .when(!resizing, |el| el.hover(move |s| s.bg(handle_hover)))
                            .tooltip(Tooltip::text("Resize inbox list"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                    if event.click_count >= 2 {
                                        this.inbox.list_width = DEFAULT_LIST_WIDTH;
                                        this.save_settings(cx);
                                    } else {
                                        this.inbox.resizing = Some((
                                            f32::from(event.position.x),
                                            this.inbox.list_width,
                                        ));
                                    }
                                    cx.notify();
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_inbox_detail(cx)),
            )
            .into_any_element()
    }

    fn end_inbox_resize(&mut self, cx: &mut Context<Self>) {
        if self.inbox.resizing.take().is_some() {
            self.save_settings(cx);
            cx.notify();
        }
    }

    /// The kind filter and "Mine", then the search, Mark all as read and
    /// Refresh.
    fn render_inbox_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let kinds = KindFilter::ALL.iter().fold(
            SegmentedControl::new("inbox-kind", self.inbox.kind.key()).size(ControlSize::Sm),
            |control, (_, key, label)| control.segment(*key, *label, None),
        );
        let mine = self.inbox.assigned_to_me;
        let fg = colors.fg;
        let any_unseen = self
            .shown_inbox_items(cx)
            .into_iter()
            .any(|ix| self.inbox.is_unseen(&self.inbox.items[ix]));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(36.0))
                    .px_2()
                    .child(
                        kinds.on_change(cx.listener(|this, key: &SharedString, _, cx| {
                            this.inbox.kind = KindFilter::from_key(key);
                            cx.notify();
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("inbox-assigned")
                            .px_2()
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .text_color(if mine { fg } else { fg.opacity(0.5) })
                            .when(mine, |el| el.bg(fg.opacity(0.10)))
                            .hover(move |s| s.bg(fg.opacity(0.08)))
                            .tooltip(Tooltip::text("Only items assigned to you"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inbox.assigned_to_me = !this.inbox.assigned_to_me;
                                this.refresh_inbox(cx);
                            }))
                            .child("Mine"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(36.0))
                    .px_2()
                    .border_t_1()
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
                            .text_size(px(12.0))
                            .child(self.inbox_search_input.clone()),
                    )
                    .child(
                        IconButton::new("inbox-mark-read", IconName::CheckCheck)
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("Mark all as read")
                            .disabled(!any_unseen)
                            .on_click(cx.listener(|this, _, _, cx| this.mark_inbox_read(cx))),
                    )
                    .child(
                        IconButton::new(
                            "inbox-refresh",
                            if self.inbox.loading {
                                IconName::LoaderCircle
                            } else {
                                IconName::RefreshCw
                            },
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .tooltip("Refresh")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_inbox(cx))),
                    ),
            )
    }

    fn render_inbox_list(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let message = |text: String| {
            div()
                .px_3()
                .py_3()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.5))
                .child(text)
                .into_any_element()
        };
        match self.inbox.status {
            None => return message("Loading…".into()),
            Some(Status::NotInstalled) => {
                return message(
                    "Install the GitHub CLI (`brew install gh`) to fill the Inbox.".into(),
                );
            }
            Some(Status::SignedOut) => {
                return message("Run `gh auth login` in a terminal to connect GitHub.".into());
            }
            Some(Status::Ready) => {}
        }
        let shown = self.shown_inbox_items(cx);
        if shown.is_empty() {
            let narrowed = !self.inbox_search_input.read(cx).text().trim().is_empty()
                || self.inbox.kind != KindFilter::All
                || self.inbox.assigned_to_me;
            let text = if self.inbox.loading {
                "Loading…".to_string()
            } else if let Some(error) = self.inbox.errors.first() {
                error.clone()
            } else if self.inbox_projects().is_empty() {
                "Open a project to fill the inbox".to_string()
            } else if narrowed {
                "No issues or pull requests match these filters".to_string()
            } else {
                "No matching issues or pull requests".to_string()
            };
            return message(text);
        }
        let now = crate::app::now_ms();
        let len = shown.len();
        let rows = cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
            range
                .filter_map(|row| shown.get(row).copied())
                .map(|ix| this.render_inbox_row(ix, now, cx))
                .collect::<Vec<_>>()
        });
        div()
            .id("inbox-list-focus")
            .key_context("InboxList")
            .track_focus(&self.inbox_focus)
            .size_full()
            .flex()
            .flex_col()
            .child(crate::ui::scrollbar::framed(
                "inbox-list-scrollbar",
                &self.inbox.scroll,
                uniform_list("inbox-list", len, rows)
                    .track_scroll(&self.inbox.scroll)
                    .size_full()
                    .p(px(6.0))
                    .pr(px(6.0) + crate::ui::scrollbar::gutter()),
            ))
            .into_any_element()
    }

    /// MonoCode `InboxCard`: kind · #n, the time and the unread dot, the
    /// title, the repo and up to two labels.
    fn render_inbox_row(&self, ix: usize, now: i64, cx: &Context<Self>) -> AnyElement {
        let item = &self.inbox.items[ix];
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let active = self.inbox.selected.as_deref() == Some(key.as_str());
        let unseen = self.inbox.is_unseen(item);
        let (icon, tint, _) = status_mark(item, cx);
        let selection = fg.opacity(if cx.theme().is_dark() { 0.10 } else { 0.06 });
        let hover = fg.opacity(0.05);
        let repairs = self
            .inbox
            .repairs
            .iter()
            .filter(|r| r.item_key == key)
            .count();
        div()
            .id(SharedString::from(format!("inbox-row-{key}")))
            .h(px(ROW_HEIGHT))
            .pb_0p5()
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .when(active, |el| el.bg(selection))
                    .when(!active, |el| el.hover(move |s| s.bg(hover)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(Icon::new(icon).size(IconSize::Xs).color(tint))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(fg.opacity(0.5))
                                    .child(format!("{} · #{}", kind_label(item.kind), item.number)),
                            )
                            .when(repairs > 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_0p5()
                                        .text_size(px(11.0))
                                        .text_color(colors.accent)
                                        .child(
                                            Icon::new(IconName::MessageSquare)
                                                .size(IconSize::Xs)
                                                .color(colors.accent),
                                        )
                                        .child(repairs.to_string()),
                                )
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.0))
                                    .text_color(fg.opacity(0.45))
                                    .child(relative_time(&item.updated_at, now)),
                            )
                            .when(unseen, |el| {
                                el.child(
                                    div()
                                        .size(px(6.0))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(colors.accent),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt_1()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(SharedString::from(item.title.clone())),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(fg.opacity(0.45))
                                    .child(SharedString::from(item.repo.clone())),
                            )
                            .children(item.labels.iter().take(2).map(|l| label_chip(l, cx))),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // The list takes the keyboard, so ↑/↓ go on from here.
                window.focus(&this.inbox_focus, cx);
                this.select_inbox_item(key.clone(), cx)
            }))
            .into_any_element()
    }
}

/// Loads `fetch` into `slot(inbox)[key]` off the UI thread, once at a time.
fn load<T: Send + 'static>(
    app: &mut BenCodeApp,
    slot: fn(&mut InboxState) -> &mut Loaded<T>,
    key: &str,
    fetch: impl FnOnce() -> Result<T, String> + Send + 'static,
    cx: &mut Context<BenCodeApp>,
) {
    let loaded = slot(&mut app.inbox);
    if loaded.value.contains_key(key) || !loaded.loading.insert(key.to_string()) {
        return;
    }
    let generation = app.inbox.generation;
    let key = key.to_string();
    let task = cx.background_executor().spawn(async move { fetch() });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        if let Err(err) = &result {
            log::warn!("inbox: could not load {key}: {err}");
        }
        let updated = this.update(cx, |app, cx| {
            let loaded = slot(&mut app.inbox);
            loaded.loading.remove(&key);
            // A refresh started since; its own load replaces this one.
            if app.inbox.generation == generation {
                slot(&mut app.inbox).value.insert(key, result);
            }
            cx.notify();
        });
        if let Err(err) = updated {
            log::debug!("inbox load after app drop: {err:#}");
        }
    })
    .detach();
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(updated: &str) -> WorkItem {
        WorkItem {
            kind: Kind::Issue,
            number: 1,
            title: "t".into(),
            url: String::new(),
            state: "OPEN".into(),
            state_reason: String::new(),
            created_at: String::new(),
            updated_at: updated.into(),
            labels: Vec::new(),
            assignees: Vec::new(),
            draft: false,
            repo: "o/r".into(),
            project: "/p".into(),
        }
    }

    #[test]
    fn relative_times_read_like_intl() {
        let now = "2026-01-10T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(relative_time("2026-01-10T11:59:30Z", now), "now");
        assert_eq!(relative_time("2026-01-10T11:00:00Z", now), "1 hour ago");
        assert_eq!(relative_time("2026-01-09T10:00:00Z", now), "yesterday");
        assert_eq!(relative_time("2026-01-05T12:00:00Z", now), "5 days ago");
        assert_eq!(relative_time("not a date", now), "");
    }

    #[test]
    fn kind_filter_round_trips_its_keys() {
        for (filter, key, _) in KindFilter::ALL {
            assert_eq!(KindFilter::from_key(key), filter);
        }
        assert!(KindFilter::Issues.admits(Kind::Issue));
        assert!(!KindFilter::Issues.admits(Kind::Pr));
    }

    #[test]
    fn read_state_follows_monocode() {
        let mut inbox = InboxState::default();
        let old = item("2026-01-01T00:00:00Z");
        // Nothing is new before the first list was seeded.
        assert!(!inbox.is_unseen(&old));
        inbox.items = vec![old.clone()];
        assert!(inbox.seed_seen());
        assert!(!inbox.is_unseen(&old));
        let updated = item("2026-01-02T00:00:00Z");
        assert!(inbox.is_unseen(&updated), "activity after it was read");
        assert!(inbox.mark_seen(&updated));
        assert!(!inbox.is_unseen(&updated));
        assert!(!inbox.mark_seen(&old), "an older read never rewinds");
    }
}
