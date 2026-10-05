//! MonoCode's sidebar session list, as pure functions: the filters
//! (`sessionFilters.ts`), search (`filterSessionsByQuery`), card labels
//! (`sessionDisplayTitle`, `formatGitLabel`, `formatRelative`), the list's
//! shape (`buildSessionList`) and multi-selection (`sessionSelection.ts`).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::app::session_folders::SessionFolder;
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::ui::quick_open::fuzzy_match;

/// MonoCode `NO_BRANCH_LABEL`: a thread whose worktree was deleted.
pub const NO_BRANCH_LABEL: &str = "No branch selected";

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

/// MonoCode `SessionTimeFilter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFilter {
    #[default]
    #[serde(rename = "all")]
    All,
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl TimeFilter {
    pub const ALL: [(TimeFilter, &'static str); 4] = [
        (TimeFilter::All, "All time"),
        (TimeFilter::Today, "Today"),
        (TimeFilter::Week, "Last 7 days"),
        (TimeFilter::Month, "Last 30 days"),
    ];
}

/// MonoCode `SessionStatusFilter`: rows in any checked state; none checked
/// means every row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatusFilter {
    pub working: bool,
    pub needs_approval: bool,
    pub done: bool,
}

impl StatusFilter {
    fn any(&self) -> bool {
        self.working || self.needs_approval || self.done
    }
}

/// MonoCode `SessionSidebarFilters` (`monocode.sessionSidebarFilters`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionFilters {
    /// Archived is a view of its own: only archived rows show.
    pub show_archived: bool,
    pub hidden_harnesses: Vec<String>,
    pub time: TimeFilter,
    pub status: StatusFilter,
}

impl SessionFilters {
    /// MonoCode `hasActiveSessionFilters`.
    pub fn is_active(&self) -> bool {
        self.show_archived
            || !self.hidden_harnesses.is_empty()
            || self.time != TimeFilter::All
            || self.status.any()
    }
}

/// What a card shows besides its age (MonoCode's busy / approval / done ids).
#[derive(Default)]
pub struct LiveStates {
    pub busy: HashSet<String>,
    pub approval: HashSet<String>,
    pub done: HashSet<String>,
}

/// MonoCode `timeFilterStart`; "Today" starts at local midnight.
pub fn time_filter_start(time: TimeFilter, now: i64) -> i64 {
    match time {
        TimeFilter::All => 0,
        TimeFilter::Today => local_midnight(now).unwrap_or(now - now.rem_euclid(DAY_MS)),
        TimeFilter::Week => now - 7 * DAY_MS,
        TimeFilter::Month => now - 30 * DAY_MS,
    }
}

fn local_midnight(now: i64) -> Option<i64> {
    let zoned = jiff::Timestamp::from_millisecond(now)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::system());
    let start = zoned.start_of_day().ok()?;
    Some(start.timestamp().as_millisecond())
}

/// The filters MonoCode applies before search: archive view, providers,
/// time window and status.
pub fn passes_filters(
    session: &SessionRow,
    filters: &SessionFilters,
    states: &LiveStates,
    now: i64,
) -> bool {
    if session.archived != filters.show_archived {
        return false;
    }
    if filters.hidden_harnesses.iter().any(|h| h == &session.harness) {
        return false;
    }
    if filters.time != TimeFilter::All && session.updated_at < time_filter_start(filters.time, now)
    {
        return false;
    }
    let status = filters.status;
    if !status.any() {
        return true;
    }
    (status.working && states.busy.contains(&session.id))
        || (status.needs_approval && states.approval.contains(&session.id))
        || (status.done && states.done.contains(&session.id))
}

/// MonoCode `HARNESS_LABEL`: the harness id, used in stored titles.
fn harness_label(harness: &str) -> &str {
    harness
}

/// MonoCode `sessionDisplayTitle`: without the stored `claude · ` prefix,
/// and a bare harness name reads "New session". Blank titles do too.
pub fn display_title(title: &str, harness: &str) -> String {
    let prefix = format!("{} · ", harness_label(harness));
    if let Some(rest) = title.strip_prefix(&prefix) {
        return rest.to_string();
    }
    let bare = title == harness_label(harness)
        || HarnessKind::from_id(harness).is_some_and(|kind| title == kind.label());
    if bare || title.trim().is_empty() {
        return "New session".to_string();
    }
    title.to_string()
}

/// MonoCode `formatGitLabel`: `repo/branch`, else whichever is known.
pub fn git_label(repo: Option<&str>, branch: Option<&str>) -> String {
    let repo = repo.filter(|r| !r.is_empty());
    let branch = branch.filter(|b| !b.is_empty());
    match (repo, branch) {
        (Some(repo), Some(branch)) => format!("{repo}/{branch}"),
        (None, Some(branch)) => branch.to_string(),
        (Some(repo), None) => repo.to_string(),
        (None, None) => String::new(),
    }
}

/// MonoCode `formatRelative`: now, 5m, 2h 15m, 3d, then "Oct 3".
pub fn format_relative(updated_at: i64, now: i64) -> String {
    if updated_at <= 0 {
        return String::new();
    }
    let diff = (now - updated_at).max(0);
    if diff < MINUTE_MS {
        return "now".into();
    }
    if diff < HOUR_MS {
        return format!("{}m", diff / MINUTE_MS);
    }
    if diff < DAY_MS {
        let hours = diff / HOUR_MS;
        let minutes = (diff % HOUR_MS) / MINUTE_MS;
        return if minutes > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{hours}h")
        };
    }
    if diff < 7 * DAY_MS {
        return format!("{}d", diff / DAY_MS);
    }
    jiff::Timestamp::from_millisecond(updated_at)
        .map(|ts| {
            ts.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%b %-d")
                .to_string()
        })
        .unwrap_or_default()
}

/// MonoCode `sessionSearchHit`: a fuzzy hit on the title (shown or
/// stored), the model, the harness or `repo/branch`.
pub fn matches_query(session: &SessionRow, query: &str, repo: Option<&str>) -> bool {
    let needle = query.trim();
    if needle.is_empty() {
        return true;
    }
    let git = git_label(repo, session.branch.as_deref());
    let title = display_title(&session.title, &session.harness);
    [
        title.as_str(),
        session.title.as_str(),
        session.model.as_str(),
        session.harness.as_str(),
        git.as_str(),
    ]
    .iter()
    .any(|field| !field.is_empty() && fuzzy_match(needle, field).is_some())
}

/// MonoCode `compareSessionSummaries`: pinned first, newest, then by id.
pub fn compare_sessions(a: &SessionRow, b: &SessionRow) -> std::cmp::Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| b.updated_at.cmp(&a.updated_at))
        .then_with(|| a.id.cmp(&b.id))
}

/// One row of the list (MonoCode `SessionListEntry`).
pub enum ListEntry<'a> {
    /// MonoCode's "Reminders" group, soonest first.
    Reminders {
        collapsed: bool,
        sessions: Vec<&'a SessionRow>,
    },
    Folder {
        folder: &'a SessionFolder,
        sessions: Vec<&'a SessionRow>,
    },
    Pinned {
        collapsed: bool,
        sessions: Vec<&'a SessionRow>,
    },
    Session(&'a SessionRow),
}

/// How the list is grouped besides folders.
#[derive(Clone, Copy, Default)]
pub struct Groups<'a> {
    pub pinned_collapsed: bool,
    pub reminders_collapsed: bool,
    /// Threads with a reminder, soonest first.
    pub reminder_ids: &'a [String],
}

/// MonoCode `buildSessionList`: reminders first, then folders (members
/// sorted), the pinned group, and loose threads. A thread with a reminder
/// shows only there. `visible` is already sorted.
pub fn build_list<'a>(
    visible: &[&'a SessionRow],
    folders: &'a [SessionFolder],
    groups: Groups<'_>,
) -> Vec<ListEntry<'a>> {
    let mut entries = Vec::new();
    let reminded: Vec<&SessionRow> = groups
        .reminder_ids
        .iter()
        .filter_map(|id| visible.iter().copied().find(|s| &s.id == id))
        .collect();
    let mut grouped: HashSet<&str> = reminded.iter().map(|s| s.id.as_str()).collect();
    if !reminded.is_empty() {
        entries.push(ListEntry::Reminders {
            collapsed: groups.reminders_collapsed,
            sessions: reminded,
        });
    }
    let in_reminders = grouped.clone();
    for folder in folders {
        for id in &folder.session_ids {
            grouped.insert(id.as_str());
        }
        let mut members: Vec<&SessionRow> = folder
            .session_ids
            .iter()
            .filter(|id| !in_reminders.contains(id.as_str()))
            .filter_map(|id| visible.iter().copied().find(|s| &s.id == id))
            .collect();
        if members.is_empty() {
            continue;
        }
        members.sort_by(|a, b| compare_sessions(a, b));
        entries.push(ListEntry::Folder {
            folder,
            sessions: members,
        });
    }
    let loose: Vec<&SessionRow> = visible
        .iter()
        .copied()
        .filter(|s| !grouped.contains(s.id.as_str()))
        .collect();
    let pinned: Vec<&SessionRow> = loose.iter().copied().filter(|s| s.pinned).collect();
    if !pinned.is_empty() {
        entries.push(ListEntry::Pinned {
            collapsed: groups.pinned_collapsed,
            sessions: pinned,
        });
    }
    entries.extend(loose.into_iter().filter(|s| !s.pinned).map(ListEntry::Session));
    entries
}

/// MonoCode `sessionListNavigationIds`: the order cards read in, folded
/// groups skipped unless a search opens them.
pub fn navigation_ids(entries: &[ListEntry<'_>], expand_collapsed: bool) -> Vec<String> {
    let mut ids = Vec::new();
    for entry in entries {
        match entry {
            ListEntry::Session(session) => ids.push(session.id.clone()),
            ListEntry::Pinned {
                collapsed,
                sessions,
            }
            | ListEntry::Reminders {
                collapsed,
                sessions,
            } => {
                if !collapsed || expand_collapsed {
                    ids.extend(sessions.iter().map(|s| s.id.clone()));
                }
            }
            ListEntry::Folder { folder, sessions } => {
                if !folder.collapsed || expand_collapsed {
                    ids.extend(sessions.iter().map(|s| s.id.clone()));
                }
            }
        }
    }
    ids
}

/// The cards picked with ⌘/⇧-click and the anchor a ⇧-click ranges from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub ids: HashSet<String>,
    pub anchor: Option<String>,
    /// Set by a right-click on an unpicked card: the menu's lone target,
    /// dropped again when the menu closes.
    pub from_menu: bool,
}

impl Selection {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// MonoCode `toggleSessionSelection` (⌘-click).
    pub fn toggle(&mut self, id: &str) {
        self.from_menu = false;
        self.anchor = Some(id.to_string());
        if !self.ids.remove(id) {
            self.ids.insert(id.to_string());
        }
        if self.ids.is_empty() {
            self.anchor = None;
        }
    }

    /// MonoCode's ⇧-click: the cards from the anchor (else the open
    /// thread) to `id`, added to the selection when ⌘ is held too.
    pub fn select_range(&mut self, id: &str, order: &[String], active: Option<&str>, add: bool) {
        self.from_menu = false;
        if self.anchor.as_deref().is_some_and(|a| !order.iter().any(|o| o == a)) {
            self.anchor = None;
        }
        let anchor = self
            .anchor
            .clone()
            .or_else(|| active.map(str::to_string))
            .unwrap_or_else(|| id.to_string());
        let start = order.iter().position(|o| *o == anchor);
        let end = order.iter().position(|o| o == id);
        let range: Vec<String> = match (start, end) {
            (Some(s), Some(e)) => order[s.min(e)..=s.max(e)].to_vec(),
            _ => vec![id.to_string()],
        };
        self.anchor = Some(if start.is_some() { anchor } else { id.to_string() });
        if !add {
            self.ids.clear();
        }
        self.ids.extend(range);
    }

    /// MonoCode `pruneSessionSelection`.
    pub fn prune(&mut self, available: &[String]) {
        self.ids.retain(|id| available.iter().any(|a| a == id));
        if self.anchor.as_deref().is_some_and(|a| !available.iter().any(|o| o == a)) {
            self.anchor = None;
        }
    }

    /// MonoCode `orderedSessionActionIds`: what a menu opened on `clicked`
    /// acts on, in list order.
    pub fn action_ids(&self, clicked: &str, order: &[String]) -> Vec<String> {
        if !self.ids.contains(clicked) || self.ids.len() <= 1 {
            return vec![clicked.to_string()];
        }
        let ordered: Vec<String> = order
            .iter()
            .filter(|id| self.ids.contains(*id))
            .cloned()
            .collect();
        if ordered.is_empty() {
            vec![clicked.to_string()]
        } else {
            ordered
        }
    }
}

/// MonoCode `harnessesInSessions`: the providers present, in harness order.
pub fn harnesses_in(sessions: &[&SessionRow]) -> Vec<HarnessKind> {
    crate::harness::ALL_HARNESSES
        .iter()
        .copied()
        .filter(|kind| sessions.iter().any(|s| s.harness == kind.id()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, updated_at: i64, pinned: bool) -> SessionRow {
        SessionRow {
            id: id.into(),
            updated_at,
            pinned,
            harness: "claude".into(),
            ..Default::default()
        }
    }

    #[test]
    fn display_titles_drop_the_harness_prefix() {
        assert_eq!(display_title("claude · Fix login", "claude"), "Fix login");
        assert_eq!(display_title("claude", "claude"), "New session");
        assert_eq!(display_title("Claude Code", "claude"), "New session");
        assert_eq!(display_title("  ", "codex"), "New session");
        assert_eq!(display_title("codex · x", "claude"), "codex · x");
    }

    #[test]
    fn git_labels_join_what_is_known() {
        assert_eq!(git_label(Some("repo"), Some("main")), "repo/main");
        assert_eq!(git_label(None, Some("main")), "main");
        assert_eq!(git_label(Some("repo"), Some("")), "repo");
        assert_eq!(git_label(None, None), "");
    }

    #[test]
    fn relative_time_matches_monocode() {
        let now = 100 * DAY_MS;
        assert_eq!(format_relative(0, now), "");
        assert_eq!(format_relative(now - 30_000, now), "now");
        assert_eq!(format_relative(now - 5 * MINUTE_MS, now), "5m");
        assert_eq!(format_relative(now - 2 * HOUR_MS, now), "2h");
        assert_eq!(
            format_relative(now - 2 * HOUR_MS - 15 * MINUTE_MS, now),
            "2h 15m"
        );
        assert_eq!(format_relative(now - 3 * DAY_MS, now), "3d");
        let old = format_relative(now - 10 * DAY_MS, now);
        assert!(!old.ends_with('d') && !old.is_empty(), "{old}");
    }

    #[test]
    fn filters_narrow_by_archive_provider_time_and_status() {
        let now = 40 * DAY_MS;
        let mut filters = SessionFilters::default();
        let states = LiveStates::default();
        let mut archived = row("a", now, false);
        archived.archived = true;
        assert!(!passes_filters(&archived, &filters, &states, now));
        filters.show_archived = true;
        assert!(passes_filters(&archived, &filters, &states, now));
        assert!(!passes_filters(&row("b", now, false), &filters, &states, now));

        let mut filters = SessionFilters::default();
        filters.hidden_harnesses = vec!["claude".into()];
        assert!(!passes_filters(&row("c", now, false), &filters, &states, now));

        let mut filters = SessionFilters::default();
        filters.time = TimeFilter::Week;
        assert!(!passes_filters(&row("d", now - 8 * DAY_MS, false), &filters, &states, now));
        assert!(passes_filters(&row("e", now - DAY_MS, false), &filters, &states, now));

        let mut filters = SessionFilters::default();
        filters.status.working = true;
        let mut busy = LiveStates::default();
        busy.busy.insert("f".into());
        assert!(passes_filters(&row("f", now, false), &filters, &busy, now));
        assert!(!passes_filters(&row("g", now, false), &filters, &busy, now));
        assert!(filters.is_active() && !SessionFilters::default().is_active());
    }

    #[test]
    fn search_is_fuzzy_across_title_model_and_branch() {
        let mut s = row("a", 1, false);
        s.title = "claude · Fix login flow".into();
        s.model = "claude:opus".into();
        s.branch = Some("feat/auth".into());
        assert!(matches_query(&s, "fx lgn", None));
        assert!(matches_query(&s, "opus", None));
        assert!(matches_query(&s, "repo/feat", Some("repo")));
        assert!(!matches_query(&s, "zzz", None));
    }

    #[test]
    fn the_list_puts_folders_then_pins_then_the_rest() {
        let (a, b, c, d) = (
            row("a", 4, false),
            row("b", 3, true),
            row("c", 2, false),
            row("d", 1, false),
        );
        let folders = vec![SessionFolder {
            id: "f".into(),
            name: "F".into(),
            session_ids: vec!["d".into(), "c".into(), "gone".into()],
            collapsed: true,
            ..Default::default()
        }];
        let visible = vec![&a, &b, &c, &d];
        let list = build_list(&visible, &folders, Groups::default());
        assert!(matches!(&list[0], ListEntry::Folder { sessions, .. }
            if sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>() == ["c", "d"]));
        assert!(matches!(&list[1], ListEntry::Pinned { sessions, .. } if sessions[0].id == "b"));
        assert!(matches!(&list[2], ListEntry::Session(s) if s.id == "a"));
        assert_eq!(navigation_ids(&list, false), ["b", "a"]);
        assert_eq!(navigation_ids(&list, true), ["c", "d", "b", "a"]);

        let reminded = ["d".to_string(), "b".to_string()];
        let groups = Groups {
            reminder_ids: &reminded,
            ..Groups::default()
        };
        let list = build_list(&visible, &folders, groups);
        assert!(matches!(&list[0], ListEntry::Reminders { sessions, .. }
            if sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>() == ["d", "b"]));
        assert!(matches!(&list[1], ListEntry::Folder { sessions, .. } if sessions.len() == 1));
        assert!(matches!(&list[2], ListEntry::Session(s) if s.id == "a"), "no pinned group left");
    }

    #[test]
    fn selection_toggles_ranges_and_orders_menu_targets() {
        let order: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let mut sel = Selection::default();
        sel.toggle("b");
        sel.select_range("d", &order, None, false);
        assert_eq!(sel.ids.len(), 3);
        assert_eq!(sel.action_ids("c", &order), ["b", "c", "d"]);
        assert_eq!(sel.action_ids("a", &order), ["a"]);
        sel.prune(&["b".into()]);
        assert_eq!(sel.ids.len(), 1);
        sel.toggle("b");
        assert!(sel.ids.is_empty() && sel.anchor.is_none());
    }
}
