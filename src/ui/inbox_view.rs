//! Inbox: MonoCode's review queue. Open issues and pull requests of the
//! rail's projects come from the `gh` CLI, and a connected Backlog space's
//! open issues from its REST API (`backlog.rs`); the list sits
//! left (resizable, with read state, "Needs you" pinned and grouped:
//! `toolbar`, `list`, `grouping`), the selected item right
//! (`detail`): its body, pull request checks and CI repair (`checks`),
//! merge and state actions (`pr_actions`), the conversation and a comment
//! box (`comments`). "Send to agent" starts a thread in the chosen project
//! with the issue as a composer card (`features/inbox/ui/InboxView.tsx`,
//! `App.tsx` `onStartInboxItem`).

mod backlog_fields;
mod batch;
mod blocks;
mod checks;
pub mod ci_repair;
mod comments;
mod detail;
mod grouping;
mod keyboard;
mod list;
mod picks;
mod pr_actions;
mod properties;
mod row;
mod selection;
mod tab;
mod threads;
mod toolbar;
mod translate;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use bencode_ui::primitives::{IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use crate::app::item_threads::{PrimaryAction, primary_action};
use crate::app::session_list::Selection;
use crate::app::{BenCodeApp, Surface};
use crate::backlog;
use crate::github::{self, Kind, Status, WorkItem};
use crate::ui::composer::cards::ComposerCard;
use crate::ui::composer::inbox_card::{CardTask, InboxCard};
use crate::ui::scale::px;
use crate::work_items::Provider;

use crate::ui::tokens::{Fill, Ink, Tone as _};
use crate::work_items::attention::{self, Attention};
use crate::work_items::tickets::TicketLinks;
use blocks::Described;
pub use checks::{Repair, RepairForm};
pub use comments::ReplyTarget;
pub(crate) use detail::{DetailList, DetailView};
#[cfg(any(test, feature = "snapshot"))]
pub use grouping::GroupBy;
pub use grouping::ListPrefs;
#[cfg(test)]
pub use grouping::SortBy;
pub use pr_actions::PrActionUi;
pub(crate) use row::{key_tail, short_key};
pub use translate::Translations;

/// How "Send to agent" starts a thread: in a new worktree (the composer's
/// ⌘⇧G mode), in Plan mode, asking what (a pull request is reviewed or
/// its review addressed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartWith {
    pub worktree: bool,
    pub plan: bool,
    pub task: CardTask,
}

impl StartWith {
    /// The main button's start: an issue worked on, a pull request
    /// reviewed.
    pub fn main(kind: Kind) -> Self {
        Self {
            task: match kind {
                Kind::Issue => CardTask::Work,
                Kind::Pr => CardTask::Review,
            },
            ..Self::default()
        }
    }
}

/// MonoCode keeps a fetched list this long before refetching on open.
const FRESH_FOR: Duration = Duration::from_secs(60);
/// MonoCode `POLL_MS`, and a pause after launch before the first poll.
const POLL_EVERY: Duration = Duration::from_secs(30);
const POLL_DELAY: Duration = Duration::from_secs(5);
/// MonoCode's list column: default and limits of its resize handle.
pub const DEFAULT_LIST_WIDTH: f32 = 340.0;
const MIN_LIST_WIDTH: f32 = 260.0;
const MAX_LIST_WIDTH: f32 = 560.0;

/// MonoCode's pull request sections: the description, checks and
/// conversation, or the code it changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrTab {
    #[default]
    Summary,
    Files,
}

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
        (Self::PullRequests, "prs", "PRs"),
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

/// What a load lands in: the item's text, where a selection is made by
/// block, or something beside it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Text,
    Other,
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
    /// The last fetch found a Backlog connection.
    pub backlog_connected: bool,
    pub items: Vec<WorkItem>,
    /// Items fetched by key for a tab beside the chat, the list holding
    /// them or not (`work_item_tab.rs`); `item` finds them too.
    pub known: Loaded<WorkItem>,
    /// Which listed pull requests are named after which tickets, taken
    /// when the list lands.
    pub tickets: TicketLinks,
    /// The connected Backlog space's project keys, as the last fetch found
    /// them: tickets of these are looked for in pull requests.
    ticket_projects: Vec<String>,
    /// What the listed Backlog keys share, left out of the rows
    /// (`row::backlog_key_prefix`).
    pub key_prefix: String,
    pub errors: Vec<String>,
    pub loading: bool,
    /// A fetch (shown or the poll's) is running.
    pub fetching: bool,
    pub fetched_at: Option<Instant>,
    pub selected: Option<String>,
    /// Items picked for a batch send (⌘-click, ⇧-click, `x`), apart from
    /// the one open (`picks.rs`).
    pub picks: Selection,
    /// The batch send's confirmation, while open (`batch.rs`).
    pub batch: Option<batch::BatchSend>,
    pub kind: KindFilter,
    /// The one tracker shown; `None` is all of them.
    pub source: Option<Provider>,
    /// The statuses shown, by name; empty is all of them.
    pub statuses: Vec<String>,
    pub assigned_to_me: bool,
    /// Only unread items (and the one open).
    pub unread_only: bool,
    /// Grouping, sort and folded groups (`grouping.rs`).
    pub list_prefs: ListPrefs,
    /// What the trackers notified the user of: "Needs you".
    pub attention: Attention,
    /// The item open, kept in "Needs you" though opening it read it.
    kept_pinned: Option<String>,
    /// The toolbar menu open, and where.
    menu: Option<(toolbar::Menu, gpui::Point<gpui::Pixels>)>,
    /// A menu's button took this press: the menu's press-outside must
    /// not close what the button just toggled.
    menu_button_hit: bool,
    pub details: Loaded<Described>,
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
    /// The section a pull request opens on; kept from one to the next, so
    /// reading several pull requests' code takes no click each.
    pub pr_tab: PrTab,
    /// The project "Send to agent" starts in, when changed from the item's.
    pub start_project: HashMap<String, String>,
    /// Items' titles and descriptions in another language (`translate.rs`).
    pub translations: Translations,
    /// Folders the inbox may name that the last list found gone from disk:
    /// drawing reads this, never the disk (`inbox_projects_shown`).
    pub missing_dirs: HashSet<String>,
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
    pub scroll: gpui::ScrollHandle,
    /// The selected item's rows (`detail`), virtual: a long description
    /// scrolls by drawing only what shows.
    detail: DetailList,
    /// The rows of the item in the file pane's tab, apart: each view keeps
    /// its own scroll and measured rows.
    tab_detail: DetailList,
}

impl Default for InboxState {
    fn default() -> Self {
        Self {
            status: None,
            backlog_connected: false,
            items: Vec::new(),
            known: Loaded::default(),
            tickets: TicketLinks::default(),
            ticket_projects: Vec::new(),
            key_prefix: String::new(),
            errors: Vec::new(),
            loading: false,
            fetching: false,
            fetched_at: None,
            selected: None,
            picks: Selection::default(),
            batch: None,
            kind: KindFilter::All,
            source: None,
            statuses: Vec::new(),
            assigned_to_me: false,
            unread_only: false,
            list_prefs: ListPrefs::default(),
            attention: Attention::default(),
            kept_pinned: None,
            menu: None,
            menu_button_hit: false,
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
            pr_tab: PrTab::default(),
            start_project: HashMap::new(),
            translations: Translations::default(),
            missing_dirs: HashSet::new(),
            seen: BTreeMap::new(),
            seen_seeded: false,
            list_width: DEFAULT_LIST_WIDTH,
            resizing: None,
            generation: 0,
            list_generation: 0,
            scroll: gpui::ScrollHandle::new(),
            detail: DetailList::default(),
            tab_detail: DetailList::default(),
        }
    }
}

/// The snapshots' Inbox (`app/snapshot.rs`), staged with no fetch.
#[cfg(feature = "snapshot")]
impl InboxState {
    /// A landed list with Backlog connected, read but for `unseen` (keys).
    pub(crate) fn stage_list(&mut self, items: Vec<WorkItem>, unseen: &[String]) {
        self.status = Some(Status::Ready);
        self.backlog_connected = true;
        self.loading = false;
        self.fetching = false;
        self.fetched_at = Some(Instant::now());
        self.errors.clear();
        self.list_prefs = ListPrefs::default();
        self.source = None;
        self.statuses.clear();
        self.unread_only = false;
        self.set_items(items);
        self.seen.clear();
        self.seen_seeded = true;
        for item in self.items.clone() {
            if unseen.contains(&item.key()) {
                self.mark_unseen(&item);
            } else {
                self.mark_seen(&item);
            }
        }
    }

    /// What the trackers notified the user of, and who the user is on
    /// each, as an ask would land it.
    pub(crate) fn stage_attention(
        &mut self,
        provider: Provider,
        notified: Vec<attention::Notified>,
        me: HashMap<String, String>,
    ) {
        let found = attention::Found {
            notified,
            me,
            ..Default::default()
        };
        self.attention.land(provider, Some(found), Instant::now());
    }

    /// Items `keys` picked, as ⌘-clicks leave them; with `confirm`, Send
    /// to agents asked for them too (its dialog open).
    pub(crate) fn stage_picks(&mut self, keys: &[String], confirm: bool) {
        self.picks.clear();
        for key in keys {
            self.picks.toggle(key);
        }
        self.batch = confirm.then(|| batch::BatchSend {
            keys: keys.to_vec(),
            start: StartWith::default(),
            include_linked: false,
            starting: false,
        });
    }

    /// Scrolls the open item's detail to its `row`th row: the head out of
    /// view, so the compact bar shows.
    pub(crate) fn stage_detail_scroll(&mut self, row: usize) {
        self.detail.list.scroll_to(gpui::ListOffset {
            item_ix: row,
            offset_in_item: gpui::px(0.0),
        });
    }

    /// What opening item `key` loads: its details and conversation.
    pub(crate) fn stage_item(
        &mut self,
        key: &str,
        details: github::Details,
        thread: github::Thread,
    ) {
        (self.details.value).insert(key.to_string(), Ok(Described::from(details)));
        self.threads.value.insert(key.to_string(), Ok(thread));
    }
}

/// MonoCode `inboxUpdatedAt`.
pub fn updated_ms(item: &WorkItem) -> i64 {
    item.updated_at
        .parse::<jiff::Timestamp>()
        .map_or(0, |t| t.as_millisecond())
}

impl InboxState {
    /// The source filter in force: none while GitHub is the only source.
    fn source_filter(&self) -> Option<Provider> {
        self.source.filter(|_| self.backlog_connected)
    }

    /// Item `key`'s description as shown: its translation's blocks, or
    /// the original's.
    fn description_blocks<'a>(&'a self, key: &str, described: &'a Described) -> &'a [SharedString] {
        match self.translations.shown(key) {
            Some(t) => &t.blocks,
            None => &described.blocks,
        }
    }

    /// Item `key`, listed or fetched for a tab.
    pub fn item(&self, key: &str) -> Option<&WorkItem> {
        (self.items.iter().find(|i| i.key() == key))
            .or_else(|| self.known.get(key).and_then(|found| found.as_ref().ok()))
    }

    /// Item `key` to change in place, listed or fetched for a tab.
    pub(crate) fn item_mut(&mut self, key: &str) -> Option<&mut WorkItem> {
        match self.items.iter().position(|i| i.key() == key) {
            Some(ix) => self.items.get_mut(ix),
            None => (self.known.value.get_mut(key)).and_then(|found| found.as_mut().ok()),
        }
    }

    /// The listed pull requests named after ticket `key`.
    pub(crate) fn ticket_prs(&self, key: &str) -> Vec<&WorkItem> {
        (self.tickets.prs(key).iter())
            .filter_map(|pr| self.item(pr))
            .collect()
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

    /// Item `item` needs the user (`work_items/attention.rs`); nothing
    /// does before the first list was taken as read.
    pub fn needs_you(&self, item: &WorkItem) -> bool {
        let key = item.key();
        self.seen_seeded
            && attention::needs_you(
                self.attention.notified_at(&key),
                self.attention.assigned_to_me(item),
                self.seen.get(&key).copied(),
                self.is_unseen(item),
            )
    }

    /// MonoCode `mergeSeen`. A notification about it is read too, though
    /// it may be stamped a moment after the item's update.
    fn mark_seen(&mut self, item: &WorkItem) -> bool {
        let at = updated_ms(item).max(self.attention.notified_at(&item.key()).unwrap_or(0));
        let entry = self.seen.entry(item.key()).or_insert(0);
        let changed = *entry < at;
        *entry = (*entry).max(at);
        changed
    }

    /// Counts `item` as unread again: its last read is forgotten.
    fn mark_unseen(&mut self, item: &WorkItem) -> bool {
        self.seen.remove(&item.key()).is_some()
    }

    /// Read becomes unread and unread read.
    fn toggle_seen(&mut self, item: &WorkItem) -> bool {
        if self.is_unseen(item) {
            self.mark_seen(item)
        } else {
            self.mark_unseen(item)
        }
    }

    /// The list as it lands: the items, what their rows leave out, and
    /// the translations their titles still match.
    fn set_items(&mut self, items: Vec<WorkItem>) {
        self.key_prefix = row::backlog_key_prefix(&items);
        self.tickets = TicketLinks::build(&items, &self.ticket_projects);
        self.items = items;
        // Picks of items the list no longer has go.
        let keys: Vec<String> = self.items.iter().map(WorkItem::key).collect();
        self.picks.prune(&keys);
        self.refresh_translated_titles();
        if let Some(key) = self.selected.clone() {
            self.sync_translations(&key);
        }
    }

    /// An explicit refresh: what was loaded for items is loaded again.
    /// Translations stay, shown while their source text is unchanged.
    fn forget_item_loads(&mut self) {
        self.details.clear();
        self.translations.forget_errors();
        self.threads.clear();
        self.checks.clear();
        self.jobs.clear();
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

/// How long ago the ISO timestamp `iso` was (`relative_time::since`).
pub fn relative_time(iso: &str, now_ms: i64) -> String {
    match iso.parse::<jiff::Timestamp>() {
        Ok(then) => crate::ui::relative_time::since(then.as_millisecond(), now_ms),
        Err(_) => String::new(),
    }
}

/// The item's state as the list and the status filter name it: Backlog's
/// own status, or GitHub's Open / Draft / Merged / Closed.
pub fn status_name(item: &WorkItem) -> &str {
    if item.provider == Provider::Backlog {
        return if item.state.is_empty() {
            "Open"
        } else {
            item.state.as_str()
        };
    }
    match (item.kind, item.state.to_uppercase().as_str()) {
        (Kind::Pr, "MERGED") => "Merged",
        (_, "CLOSED") => "Closed",
        (Kind::Pr, _) if item.draft => "Draft",
        _ => "Open",
    }
}

/// The statuses the filter offers: those of `items`, in a workflow's
/// order where Backlog gave one (`ordered`), the rest after.
pub fn status_choices(items: &[&WorkItem], ordered: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for item in items {
        let name = status_name(item);
        if !found.iter().any(|f| f == name) {
            found.push(name.to_string());
        }
    }
    let rank = |name: &String| ordered.iter().position(|o| o == name).unwrap_or(usize::MAX);
    found.sort_by_key(rank);
    found
}

/// MonoCode `inboxStatusMark`: the glyph, its tint and the state's name.
pub fn status_mark(item: &WorkItem, cx: &gpui::App) -> (IconName, gpui::Hsla, SharedString) {
    let colors = &cx.theme().colors;
    if item.provider == Provider::Backlog {
        // Backlog's statuses are the project's own, each with its colour.
        let tint = crate::ui::composer::inbox_card::label_color(&item.state_color).unwrap_or(
            if item.closed {
                colors.accent
            } else {
                colors.success
            },
        );
        let icon = if item.closed {
            IconName::CircleCheck
        } else {
            IconName::CircleDot
        };
        return (
            icon,
            tint,
            SharedString::from(status_name(item).to_string()),
        );
    }
    let state = item.state.to_uppercase();
    let (icon, tint, name) = match (item.kind, state.as_str()) {
        (Kind::Pr, "MERGED") => (IconName::GitMerge, colors.accent, "Merged"),
        (Kind::Pr, "CLOSED") => (IconName::GitPullRequestClosed, colors.danger, "Closed"),
        (Kind::Pr, _) if item.draft => (
            IconName::GitPullRequestDraft,
            colors.fg.ink(Ink::Secondary),
            "Draft",
        ),
        (Kind::Pr, _) => (IconName::GitPullRequest, colors.success, "Open"),
        (Kind::Issue, "CLOSED") if item.state_reason.eq_ignore_ascii_case("completed") => {
            (IconName::CircleCheck, colors.accent, "Closed")
        }
        (Kind::Issue, "CLOSED") => (IconName::CircleX, colors.danger, "Closed"),
        (Kind::Issue, _) => (IconName::CircleDot, colors.success, "Open"),
    };
    (icon, tint, name.into())
}

/// Where the item lives, as a row's last line: `owner/name`, or
/// `Backlog · PROJ`.
pub fn source_label(item: &WorkItem) -> String {
    match item.provider {
        Provider::GitHub => item.repo.clone(),
        provider => format!("{} · {}", provider.label(), item.repo),
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
    /// The Inbox's state (a model: `inbox_state`).
    pub fn inbox<'a>(&self, cx: &'a gpui::App) -> &'a InboxState {
        self.inbox_state.read(cx)
    }

    /// Changes the Inbox's state; the caller redraws (the app's notify, or
    /// the model's: `inbox_state.update` / `as_mut`, which the regions
    /// showing the Inbox follow).
    pub fn inbox_mut<R>(&self, cx: &mut gpui::App, change: impl FnOnce(&mut InboxState) -> R) -> R {
        self.inbox_state.update(cx, |inbox, _| change(inbox))
    }

    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.open_inbox(true, cx);
    }

    /// Shows the Inbox with its list taking the keyboard; with `pick`, an
    /// Inbox with nothing open opens its first item to read.
    pub(crate) fn open_inbox(&mut self, pick: bool, cx: &mut Context<Self>) {
        self.show_surface(Surface::Inbox, cx);
        self.focus_inbox_list(cx);
        if pick && self.inbox(cx).selected.is_none() {
            self.select_first_inbox_item(cx);
        }
        let stale = self
            .inbox(cx)
            .fetched_at
            .is_none_or(|at| at.elapsed() > FRESH_FOR);
        if stale && !self.inbox(cx).fetching {
            self.refresh_inbox(cx);
        }
    }

    /// The rail's projects that are still folders (MonoCode
    /// `collectRailProjects`), as the last list found them on disk: for
    /// drawing, which asks the disk nothing.
    pub(crate) fn inbox_projects_shown(&self, cx: &gpui::App) -> Vec<String> {
        let missing = &self.inbox(cx).missing_dirs;
        self.inbox_projects_where(|path| !missing.contains(path))
    }

    /// The rail's projects, once each, that `exists` keeps.
    fn inbox_projects_where(&self, exists: impl Fn(&str) -> bool) -> Vec<String> {
        let mut projects: Vec<String> = Vec::new();
        for path in std::iter::once(&self.current_cwd).chain(self.recent_projects.iter()) {
            let real = !matches!(path.trim(), "" | "~") && exists(path);
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
                    if !app.inbox(cx).fetching {
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
        for item in self.inbox(cx).items.clone() {
            changed |= self.inbox_state.as_mut(cx).mark_seen(&item);
        }
        if changed {
            self.save_settings(cx);
            cx.notify();
        }
    }

    /// Any listed item has activity the user has not read.
    pub fn inbox_has_unseen(&self, cx: &gpui::App) -> bool {
        let inbox = self.inbox(cx);
        inbox.items.iter().any(|item| inbox.is_unseen(item))
    }

    /// Any listed item needs the user: the rail's mark. Activity alone
    /// does not: teammates update tickets all day.
    pub fn inbox_needs_you(&self, cx: &gpui::App) -> bool {
        let inbox = self.inbox(cx);
        inbox.items.iter().any(|item| inbox.needs_you(item))
    }

    /// Fetches the list. A `quiet` fetch (the poll) shows no spinner and
    /// keeps what was loaded for items; an explicit one reloads it all.
    fn fetch_inbox(&mut self, quiet: bool, cx: &mut Context<Self>) {
        // Which are still folders is asked off the UI thread, with the
        // other folders the inbox may name.
        let candidates = self.inbox_projects_where(|_| true);
        let named: Vec<String> = self
            .inbox(cx)
            .start_project
            .values()
            .chain(self.backlog.project_folders.values())
            .cloned()
            .collect();
        let assigned = self.inbox(cx).assigned_to_me;
        let hidden = self.backlog.hidden_projects.clone();
        let list_generation = self.inbox_mut(cx, |inbox| {
            inbox.fetching = true;
            inbox.list_generation += 1;
            if !quiet {
                inbox.loading = true;
                inbox.generation += 1;
            }
            inbox.list_generation
        });
        let task = cx.background_executor().spawn(async move {
            let (projects, mut missing): (Vec<String>, Vec<String>) =
                candidates.into_iter().partition(|path| is_dir(path));
            missing.extend(named.into_iter().filter(|path| !is_dir(path)));
            let status = github::status();
            let mut list = match status {
                Status::Ready => github::inbox(&projects, assigned),
                _ => github::InboxList::default(),
            };
            let backlog = backlog::inbox(&hidden, assigned);
            let statuses = backlog.as_ref().map(|found| found.statuses.clone());
            let ticket_projects = (backlog.as_ref())
                .map(|found| found.projects.clone())
                .unwrap_or_default();
            if let Some(found) = backlog {
                list.items.extend(found.items);
                list.errors.extend(found.errors);
                list.items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            }
            let mut item_projects: Vec<&String> = list.items.iter().map(|i| &i.project).collect();
            item_projects.sort();
            item_projects.dedup();
            missing.extend(item_projects.into_iter().filter(|p| !is_dir(p)).cloned());
            (status, list, statuses, ticket_projects, missing)
        });
        cx.spawn(async move |this, cx| {
            let (status, list, backlog_statuses, ticket_projects, missing) = task.await;
            let updated = this.update(cx, |app, cx| {
                if app.inbox(cx).list_generation != list_generation {
                    return;
                }
                // A fresh Backlog connection: its first list counts as read.
                let seed_backlog =
                    std::mem::take(&mut app.backlog.seed_seen) && backlog_statuses.is_some();
                app.backlog.statuses = backlog_statuses.clone().unwrap_or_default();
                let (failed, seeded, first) = app.inbox_state.update(cx, |inbox, cx| {
                    let first = inbox.fetched_at.is_none();
                    inbox.missing_dirs = missing.into_iter().collect();
                    let status_same = inbox.status == Some(status);
                    inbox.status = Some(status);
                    inbox.backlog_connected = backlog_statuses.is_some();
                    // A linked thread's item opened before the list landed
                    // stays, even when the list does not include it.
                    let kept = inbox
                        .selected
                        .as_deref()
                        .and_then(|key| inbox.items.iter().find(|i| i.key() == key))
                        .filter(|item| !list.items.iter().any(|l| l.key() == item.key()))
                        .cloned();
                    let failed = !list.errors.is_empty();
                    let mut items = list.items;
                    items.extend(kept);
                    // The poll's usual answer: nothing new, nothing to redraw.
                    let same = status_same && inbox.items == items && inbox.errors == list.errors;
                    // Kept while Backlog answers with no catalog.
                    if !ticket_projects.is_empty() {
                        inbox.ticket_projects = ticket_projects;
                    }
                    inbox.set_items(items);
                    inbox.errors = list.errors;
                    inbox.loading = false;
                    inbox.fetching = false;
                    inbox.fetched_at = Some(Instant::now());
                    if !quiet {
                        inbox.forget_item_loads();
                    }
                    let mut seeded = inbox.seed_seen();
                    if seed_backlog {
                        for item in inbox.items.clone() {
                            if item.provider == Provider::Backlog {
                                seeded |= inbox.mark_seen(&item);
                            }
                        }
                    }
                    let gone = inbox
                        .selected
                        .as_ref()
                        .is_some_and(|key| inbox.item(key).is_none());
                    if gone {
                        inbox.selected = None;
                    }
                    // What shows the Inbox follows it (`ui/region.rs`).
                    if !(quiet && same && !seeded) {
                        cx.notify();
                    }
                    (failed, seeded, first)
                });
                if seeded {
                    app.save_settings(cx);
                }
                app.announce_inbox_changes(failed, cx);
                if let Some(key) = app.inbox(cx).selected.clone().filter(|_| !quiet) {
                    app.load_inbox_item(&key, cx);
                }
                // What the trackers notified the user of, at their pace.
                app.fetch_inbox_attention(!quiet, cx);
                // What the refresh dropped, the tab beside the chat loads again.
                if !quiet {
                    app.load_work_item_tab(cx);
                }
                // The Inbox opened before it had a list: the first item to
                // read opens now (the list already has the keyboard).
                if first && app.surface_open(Surface::Inbox) && app.inbox(cx).selected.is_none() {
                    app.select_first_inbox_item(cx);
                }
            });
            if let Err(err) = updated {
                log::debug!("inbox fetched after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// The list after every filter but the kind's: the source, statuses,
    /// Unread and the search field ("Mine" is the fetch's).
    fn inbox_matches(&self, cx: &gpui::App) -> Vec<usize> {
        let query = self
            .inbox_search_input
            .read(cx)
            .text()
            .trim()
            .to_lowercase();
        let number = query.trim_start_matches('#');
        let inbox = self.inbox(cx);
        let source = inbox.source_filter();
        inbox
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| source.is_none_or(|only| item.provider == only))
            .filter(|(_, item)| {
                inbox.statuses.is_empty() || inbox.statuses.iter().any(|s| s == status_name(item))
            })
            // The item open stays while it is read.
            .filter(|(_, item)| {
                !inbox.unread_only
                    || inbox.is_unseen(item)
                    || inbox.needs_you(item)
                    || inbox.selected.as_deref() == Some(item.key().as_str())
            })
            .filter(|(_, item)| {
                query.is_empty()
                    || item.title.to_lowercase().contains(&query)
                    || item.repo.to_lowercase().contains(&query)
                    || item.identifier.to_lowercase().contains(&query)
                    || item.number.to_string() == number
                    || item
                        .labels
                        .iter()
                        .any(|l| l.name.to_lowercase().contains(&query))
            })
            .map(|(ix, _)| ix)
            .collect()
    }

    /// The list after every filter.
    fn shown_inbox_items(&self, cx: &gpui::App) -> Vec<usize> {
        let inbox = self.inbox(cx);
        let mut shown = self.inbox_matches(cx);
        shown.retain(|ix| inbox.kind.admits(inbox.items[*ix].kind));
        shown
    }

    /// Backlog's statuses of `items`' projects, each in its workflow's
    /// order, the projects' merged.
    fn inbox_workflow<'a>(&self, items: impl IntoIterator<Item = &'a WorkItem>) -> Vec<String> {
        let mut ordered: Vec<String> = Vec::new();
        let mut projects: Vec<&str> = Vec::new();
        for item in items {
            if item.provider != Provider::Backlog || projects.contains(&item.container_id.as_str())
            {
                continue;
            }
            projects.push(&item.container_id);
            for status in self
                .backlog
                .statuses
                .get(&item.container_id)
                .into_iter()
                .flatten()
            {
                if !ordered.contains(&status.name) {
                    ordered.push(status.name.clone());
                }
            }
        }
        ordered
    }

    /// The list's lines as drawn: shown items, pinned, grouped and sorted.
    fn inbox_entries(&self, cx: &gpui::App) -> Vec<grouping::Entry> {
        let shown = self.shown_inbox_items(cx);
        let inbox = self.inbox(cx);
        let workflow = self.inbox_workflow(shown.iter().map(|ix| &inbox.items[*ix]));
        let how = grouping::Arrange {
            prefs: &inbox.list_prefs,
            workflow: &workflow,
            today: jiff::Zoned::now().date(),
        };
        let pinned = |item: &WorkItem| {
            inbox.needs_you(item) || inbox.kept_pinned.as_deref() == Some(item.key().as_str())
        };
        grouping::arrange(&inbox.items, &shown, pinned, &how)
    }

    /// Opens Inbox item `key` (a linked thread's badge).
    pub fn select_inbox_key(&mut self, key: String, cx: &mut Context<Self>) {
        self.select_inbox_item(key, cx);
    }

    /// MonoCode `onSelect`: the item is read and opens on the right.
    fn select_inbox_item(&mut self, key: String, cx: &mut Context<Self>) {
        if self.inbox(cx).selected.as_deref() != Some(key.as_str()) {
            self.inbox_state.as_mut(cx).reply_to = None;
            self.inbox_state.as_mut(cx).comment_error = None;
            self.inbox_state.as_mut(cx).pr = PrActionUi::default();
            self.inbox_state.as_mut(cx).repair = None;
            self.backlog.issue_error = None;
            // Read, it would leave "Needs you" while it is being read.
            let pinned = (self.inbox(cx).item(&key)).is_some_and(|i| self.inbox(cx).needs_you(i));
            self.inbox_state.as_mut(cx).kept_pinned = pinned.then(|| key.clone());
        }
        self.inbox_state.as_mut(cx).selected = Some(key.clone());
        if let Some(item) = self.inbox(cx).item(&key).cloned()
            && self.inbox_state.as_mut(cx).mark_seen(&item)
        {
            self.save_settings(cx);
        }
        self.load_inbox_item(&key, cx);
        self.apply_inbox_translations(&key, cx);
        cx.notify();
    }

    /// Counts item `key` as read as it stands now.
    pub(crate) fn mark_inbox_key_seen(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(item) = self.inbox(cx).item(key).cloned()
            && self.inbox_state.as_mut(cx).mark_seen(&item)
        {
            self.save_settings(cx);
        }
    }

    /// ↑/↓ (and j/k) in the focused list: the next item opens, headers
    /// skipped, and the list scrolls to keep it (and its group's header)
    /// in view.
    pub fn step_inbox_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let entries = self.inbox_entries(cx);
        let inbox = self.inbox(cx);
        let current = (inbox.selected.as_deref())
            .and_then(|key| inbox.items.iter().position(|item| item.key() == key))
            .and_then(|ix| grouping::line_of(&entries, ix));
        let Some(line) = grouping::step(&entries, current, delta) else {
            return;
        };
        let grouping::Entry::Item(ix) = entries[line] else {
            return;
        };
        let key = inbox.items[ix].key();
        list::reveal_line(&entries, line, &inbox.scroll);
        self.select_inbox_item(key, cx);
    }

    /// MonoCode "Mark all as read" over the listed items.
    fn mark_inbox_read(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<WorkItem> = self
            .shown_inbox_items(cx)
            .into_iter()
            .map(|ix| self.inbox(cx).items[ix].clone())
            .collect();
        let mut changed = false;
        for item in &shown {
            changed |= self.inbox_state.as_mut(cx).mark_seen(item);
        }
        if changed {
            self.save_settings(cx);
        }
        cx.notify();
    }

    /// The body, the conversation and, for a pull request, its checks.
    fn load_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(item) = self.inbox(cx).item(key).cloned() else {
            return;
        };
        load(
            self,
            |inbox| &mut inbox.details,
            Shown::Text,
            key,
            {
                let item = item.clone();
                move || {
                    match item.provider {
                        Provider::GitHub => github::details(
                            std::path::Path::new(&item.project),
                            &item.repo,
                            item.kind,
                            item.number,
                        ),
                        Provider::Backlog => backlog::details(&item.identifier),
                    }
                    .map(Described::from)
                }
            },
            cx,
        );
        self.load_inbox_thread(key, false, cx);
        if item.provider == Provider::Backlog {
            self.load_backlog_field_options(&item.container_id, cx);
        }
        if item.kind == Kind::Pr {
            self.load_pr_checks(key, false, cx);
            if self.inbox(cx).pr_tab == PrTab::Files {
                self.ensure_pr_diff_doc(&item, cx);
            }
        }
    }

    fn set_inbox_pr_tab(&mut self, tab: PrTab, cx: &mut Context<Self>) {
        self.inbox_state.as_mut(cx).pr_tab = tab;
        if tab == PrTab::Files
            && let Some(item) = self
                .inbox(cx)
                .selected
                .as_deref()
                .and_then(|key| self.inbox(cx).item(key))
                .cloned()
        {
            self.ensure_pr_diff_doc(&item, cx);
        }
        cx.notify();
    }

    pub(crate) fn load_inbox_thread(&mut self, key: &str, force: bool, cx: &mut Context<Self>) {
        let Some(item) = self.inbox(cx).item(key).cloned() else {
            return;
        };
        if force {
            self.inbox_state.as_mut(cx).threads.value.remove(key);
        }
        load(
            self,
            |inbox| &mut inbox.threads,
            Shown::Text,
            key,
            move || match item.provider {
                Provider::GitHub => github::thread(
                    std::path::Path::new(&item.project),
                    &item.repo,
                    item.kind,
                    item.number,
                ),
                Provider::Backlog => backlog::thread(&item.identifier),
            },
            cx,
        );
    }

    /// MonoCode `onStartInboxItem`: a thread in the chosen project, its
    /// composer holding the issue card, linked to the item from the start.
    /// It is titled with the item's title as shown (its translation while
    /// that shows), not its key: the navigator's badge names the item. A
    /// Backlog issue's card carries the description too: the agent cannot
    /// open its URL. A pull request's card asks for a review, or for its
    /// review to be addressed (`start.task`), and its title says which.
    /// `start` presets a new worktree or Plan mode. The item opens beside
    /// the thread, to read while composing.
    pub(crate) fn start_inbox_item(&mut self, key: &str, start: StartWith, cx: &mut Context<Self>) {
        let Some(item) = self.inbox(cx).item(key).cloned() else {
            return;
        };
        let Some(card) = self.inbox_start_card(&item, start.task, cx) else {
            log::warn!("inbox: {key} cannot be started as {:?}", start.task);
            return;
        };
        let title = self.inbox_thread_title(&item, card.task, cx);
        let link = crate::app::work_item_link::link_for(&item);
        let linked = link.clone();
        let cwd = self.inbox_start_project(&item, cx);
        // Read before the switch: the git status shown is this project's.
        let base = start.worktree.then(|| self.new_thread_worktree_base(&cwd));
        if !crate::app::same_project_path(&cwd, &self.current_cwd) {
            self.switch_project(cwd.clone(), cx);
        }
        self.close_surface(cx);
        let id = self.open_thread_with_card(
            &cwd,
            move |session| {
                session.title = title;
                session.linked_work_item = linked;
            },
            ComposerCard::Inbox(card),
            cx,
        );
        // Shown at once; saved apart, as a session save never writes it.
        self.links_changed();
        if link.is_some() {
            self.set_session_link(&id, link, cx);
        }
        if let Some(base) = base {
            self.new_worktrees.insert(id.clone(), base);
        }
        if start.plan {
            self.thread_mut(&id).plan_mode = true;
        }
        self.show_item_beside_thread(key, cx);
    }

    /// The card a thread for `item` starts with: an issue's (a Backlog
    /// one's with its description, when loaded), or a pull request's for
    /// `task`; `None` when the item cannot be started so.
    pub(super) fn inbox_start_card(
        &self,
        item: &WorkItem,
        task: CardTask,
        cx: &gpui::App,
    ) -> Option<InboxCard> {
        match (item.kind, task) {
            (Kind::Issue, CardTask::Work) => {
                let body = (item.provider == Provider::Backlog)
                    .then(|| {
                        let described = self.inbox(cx).details.get(&item.key())?;
                        described.as_ref().ok().map(|d| d.details.body.clone())
                    })
                    .flatten();
                Some(InboxCard::from_item_with_body(item, body.as_deref()))
            }
            (Kind::Pr, CardTask::Review | CardTask::AddressReview) => {
                InboxCard::for_pull_request(item, task)
            }
            _ => None,
        }
    }

    /// A thread's title for `item`: its title as shown; a pull request's
    /// says what the thread does with it ("Review #483: …").
    pub(super) fn inbox_thread_title(
        &self,
        item: &WorkItem,
        task: CardTask,
        cx: &gpui::App,
    ) -> String {
        let title = match self.inbox(cx).translations.shown(&item.key()) {
            Some(t) => t.title.to_string(),
            None => item.title.clone(),
        };
        let title = title.trim();
        match task {
            CardTask::Work => title.to_string(),
            task => format!("{} #{}: {title}", task.label(), item.number),
        }
    }

    /// The item's main action: its newest thread when it has one, else
    /// Send to agent, or a review for a pull request
    /// (`item_threads::primary_action`); the item opens beside the thread
    /// either way.
    pub(crate) fn open_or_start_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(kind) = self.inbox(cx).item(key).map(|item| item.kind) else {
            return;
        };
        match primary_action(&self.item_threads(key)) {
            PrimaryAction::Open(id) => {
                self.select_live_agent(&id, cx);
                self.show_item_beside_thread(key, cx);
            }
            PrimaryAction::Send => self.start_inbox_item(key, StartWith::main(kind), cx),
        }
    }

    /// The project "Send to agent" (and CI repair) starts in.
    pub(crate) fn inbox_start_project(&self, item: &WorkItem, cx: &gpui::App) -> String {
        self.start_project_where(item, is_dir, cx)
    }

    /// `inbox_start_project` as the last list found the folders on disk:
    /// for drawing, which asks the disk nothing.
    pub(crate) fn inbox_start_project_shown(&self, item: &WorkItem, cx: &gpui::App) -> String {
        let missing = &self.inbox(cx).missing_dirs;
        self.start_project_where(item, |path| !missing.contains(path), cx)
    }

    fn start_project_where(
        &self,
        item: &WorkItem,
        exists: impl Fn(&str) -> bool,
        cx: &gpui::App,
    ) -> String {
        self.inbox(cx)
            .start_project
            .get(&item.key())
            .cloned()
            .filter(|p| exists(p))
            // A Backlog project's threads start where the last one did.
            .or_else(|| {
                (item.provider == Provider::Backlog)
                    .then(|| self.backlog.project_folders.get(&item.repo).cloned())
                    .flatten()
                    .filter(|p| exists(p))
            })
            .or_else(|| Some(item.project.clone()).filter(|p| exists(p)))
            .unwrap_or_else(|| self.current_cwd.clone())
    }

    pub(crate) fn render_inbox_body(
        &mut self,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_backlog_hours(cx);
        let detail_width = self.inbox_detail_width(window, cx);
        let colors = &cx.theme().colors;
        let width = self.inbox(cx).list_width;
        let resizing = self.inbox(cx).resizing.is_some();
        let handle_hover = colors.fg.fill(Fill::Strong);
        let handle_drag = colors.fg.fill(Fill::Bold);
        div()
            .id("inbox")
            .size_full()
            .flex()
            // MonoCode's resize handle: the drag follows the pointer
            // anywhere over the Inbox.
            .when(resizing, |el| {
                el.cursor_col_resize()
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some((start_x, start_width)) = this.inbox(cx).resizing {
                            let x = crate::ui::scale::logical(event.position.x);
                            this.inbox_state.as_mut(cx).list_width =
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
                    .children(self.render_inbox_pick_bar(cx))
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
                                        this.inbox_state.as_mut(cx).list_width = DEFAULT_LIST_WIDTH;
                                        this.save_settings(cx);
                                    } else {
                                        this.inbox_state.as_mut(cx).resizing = Some((
                                            crate::ui::scale::logical(event.position.x),
                                            this.inbox(cx).list_width,
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
                    .child(self.render_inbox_detail(detail_width, cx)),
            )
            .into_any_element()
    }

    /// The detail's width (at 100%) from the window's layout: the window
    /// less the navigator and the list. Known before anything is laid
    /// out, so the detail picks its columns in the frame it is drawn, and
    /// the region is drawn again whenever the window's size changes.
    fn inbox_detail_width(&self, window: &gpui::Window, cx: &gpui::App) -> f32 {
        let navigator = if self.is_rail_open {
            self.rail_width()
        } else if self.compact_rail_active() {
            crate::ui::rail::COMPACT_RAIL_WIDTH
        } else {
            0.0
        };
        crate::ui::scale::logical(window.viewport_size().width)
            - navigator
            - self.inbox(cx).list_width
    }

    fn end_inbox_resize(&mut self, cx: &mut Context<Self>) {
        if self.inbox_state.as_mut(cx).resizing.take().is_some() {
            self.save_settings(cx);
            cx.notify();
        }
    }
}

/// A Refresh button's place while its load runs: the spinner, in the
/// button's footprint.
pub(super) fn refreshing(id: &'static str, cx: &gpui::App) -> AnyElement {
    let theme = cx.theme();
    div()
        .size(theme.control_height(ControlSize::Sm))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(loading_icon(id, theme.colors.fg.ink(Ink::Secondary)))
        .into_any_element()
}

/// MonoCode's `animate-spin` loader at the Inbox's small size.
pub(super) fn loading_icon(id: &'static str, color: gpui::Hsla) -> AnyElement {
    crate::ui::git_changes_panel::spinning_icon(
        id.into(),
        IconName::LoaderCircle,
        IconSize::Xs,
        color,
    )
}

/// Whether `path` is a folder on disk: for actions and background work,
/// not for drawing.
fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

/// Loads `fetch` into `slot(inbox)[key]` off the UI thread, once at a time.
fn load<T: Send + 'static>(
    app: &mut BenCodeApp,
    slot: fn(&mut InboxState) -> &mut Loaded<T>,
    text: Shown,
    key: &str,
    fetch: impl FnOnce() -> Result<T, String> + Send + 'static,
    cx: &mut Context<BenCodeApp>,
) {
    let started = app.inbox_mut(cx, |inbox| {
        let loaded = slot(inbox);
        !loaded.value.contains_key(key) && loaded.loading.insert(key.to_string())
    });
    if !started {
        return;
    }
    let generation = app.inbox(cx).generation;
    let key = key.to_string();
    let task = cx.background_executor().spawn(async move { fetch() });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        if let Err(err) = &result {
            log::warn!("inbox: could not load {key}: {err}");
        }
        let updated = this.update(cx, |app, cx| {
            // What shows the Inbox follows it (`ui/region.rs`).
            app.inbox_state.update(cx, |inbox, cx| {
                slot(inbox).loading.remove(&key);
                // A refresh started since; its own load replaces this one.
                if inbox.generation == generation {
                    slot(inbox).value.insert(key.clone(), result);
                }
                cx.notify();
            });
            if text == Shown::Text {
                app.forget_inbox_selection(&key, cx);
                app.apply_inbox_translations(&key, cx);
            }
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

    #[test]
    fn statuses_are_named_per_tracker_and_follow_the_workflow() {
        let github = |kind, state: &str, draft| WorkItem {
            kind,
            state: state.into(),
            draft,
            ..Default::default()
        };
        let backlog = |state: &str| WorkItem {
            provider: Provider::Backlog,
            state: state.into(),
            ..Default::default()
        };
        assert_eq!(status_name(&github(Kind::Pr, "OPEN", true)), "Draft");
        assert_eq!(status_name(&github(Kind::Issue, "OPEN", false)), "Open");
        assert_eq!(status_name(&github(Kind::Pr, "MERGED", false)), "Merged");
        assert_eq!(status_name(&backlog("In Progress")), "In Progress");

        let items = [
            backlog("Resolved"),
            github(Kind::Pr, "OPEN", true),
            backlog("Open"),
            backlog("Resolved"),
        ];
        let refs: Vec<&WorkItem> = items.iter().collect();
        let workflow = [
            "Open".to_string(),
            "In Progress".to_string(),
            "Resolved".to_string(),
        ];
        assert_eq!(
            status_choices(&refs, &workflow),
            ["Open", "Resolved", "Draft"]
        );
    }

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
            ..Default::default()
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

    #[test]
    fn read_items_can_be_unread_again() {
        let mut inbox = InboxState::default();
        let it = item("2026-01-01T00:00:00Z");
        inbox.items = vec![it.clone()];
        inbox.seed_seen();
        assert!(inbox.toggle_seen(&it));
        assert!(inbox.is_unseen(&it), "marked unread");
        assert!(!inbox.mark_unseen(&it), "already unread");
        assert!(inbox.toggle_seen(&it));
        assert!(!inbox.is_unseen(&it), "read again");
    }
}
