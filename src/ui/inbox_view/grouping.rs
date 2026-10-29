//! The Inbox list's order (BenCode's own; MonoCode lists by update time
//! only): what needs the user pinned on top, the rest grouped by project,
//! status or due date, each group sorted and collapsible. Pure: the list
//! (`list.rs`) draws what `arrange` returns, and the keys step through
//! its items, headers skipped.

use std::collections::HashMap;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use super::row::{ROW_HEIGHT, days_until};
use super::{status_name, updated_ms};
use crate::github::WorkItem;
use crate::work_items::Provider;

/// A group header's height in the list.
pub(super) const HEADER_HEIGHT: f32 = 28.0;
/// The pinned group's id, whatever the grouping.
pub(super) const NEEDS_YOU: &str = "needs-you";
/// After the pinned group while nothing else groups: the rest.
const REST: &str = "rest";
/// GitHub's states, after a Backlog workflow's.
const GITHUB_STATUSES: [&str; 4] = ["Open", "Draft", "Merged", "Closed"];
/// Due within this many days (today counts) is "this week".
const WEEK_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    #[default]
    None,
    Project,
    Status,
    Due,
}

impl GroupBy {
    pub(super) const ALL: [(Self, &'static str); 4] = [
        (Self::None, "None"),
        (Self::Project, "Project"),
        (Self::Status, "Status"),
        (Self::Due, "Due date"),
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    #[default]
    Updated,
    Due,
    Priority,
    Created,
}

impl SortBy {
    pub(super) const ALL: [(Self, &'static str); 4] = [
        (Self::Updated, "Updated"),
        (Self::Due, "Due date"),
        (Self::Priority, "Priority"),
        (Self::Created, "Created"),
    ];
}

/// How the list is grouped and sorted, and which groups are folded; saved
/// as `inboxList` in `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListPrefs {
    pub group: GroupBy,
    pub sort: SortBy,
    /// Folded groups' ids (`status:Open`, `needs-you`, …).
    pub collapsed: Vec<String>,
}

impl ListPrefs {
    fn is_collapsed(&self, id: &str) -> bool {
        self.collapsed.iter().any(|c| c == id)
    }

    /// Folds group `id`, or unfolds it.
    pub(super) fn toggle(&mut self, id: &str) {
        match self.collapsed.iter().position(|c| c == id) {
            Some(ix) => {
                self.collapsed.remove(ix);
            }
            None => self.collapsed.push(id.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Header {
    pub id: String,
    pub label: String,
    pub count: usize,
    pub collapsed: bool,
    /// The "Needs you" group.
    pub pinned: bool,
}

/// A line of the list: a group's header, or an item (its index in
/// `InboxState::items`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Header(Header),
    Item(usize),
}

impl Entry {
    pub(super) fn height(&self) -> f32 {
        match self {
            Self::Header(_) => HEADER_HEIGHT,
            Self::Item(_) => ROW_HEIGHT,
        }
    }
}

/// What `arrange` reads besides the items.
pub(super) struct Arrange<'a> {
    pub prefs: &'a ListPrefs,
    /// Backlog's statuses in their workflow's order.
    pub workflow: &'a [String],
    pub today: Date,
}

fn ms(iso: &str) -> i64 {
    iso.parse::<jiff::Timestamp>()
        .map_or(0, |t| t.as_millisecond())
}

/// What `sort` orders by, smallest first; ties go to the newest update.
fn sort_key(item: &WorkItem, sort: SortBy) -> (i64, String, i64) {
    let newest = -updated_ms(item);
    match sort {
        SortBy::Updated => (newest, String::new(), 0),
        SortBy::Created => (-ms(&item.created_at), String::new(), newest),
        // `YYYY-MM-DD` sorts as text; none comes last.
        SortBy::Due if item.due_date.is_empty() => (1, String::new(), newest),
        SortBy::Due => (0, item.due_date.clone(), newest),
        SortBy::Priority => (i64::from(!item.high_priority), String::new(), newest),
    }
}

/// The group `item` falls in: its id and label, and its rank among the
/// groups (`None`: in the order they first appear).
fn group_of(item: &WorkItem, how: &Arrange) -> (String, String, Option<usize>) {
    match how.prefs.group {
        GroupBy::None => (REST.into(), "Everything else".into(), Some(0)),
        GroupBy::Project => {
            let project = match item.provider {
                Provider::GitHub => item.repo.clone(),
                Provider::Backlog => item.repo.to_uppercase(),
            };
            (format!("project:{project}"), project, None)
        }
        GroupBy::Status => {
            let name = status_name(item);
            let rank = (how.workflow.iter().position(|s| s == name))
                .or_else(|| {
                    (GITHUB_STATUSES.iter().position(|s| *s == name))
                        .map(|ix| how.workflow.len() + ix)
                })
                .unwrap_or(usize::MAX);
            (format!("status:{name}"), name.to_string(), Some(rank))
        }
        GroupBy::Due => {
            let (id, label, rank) = match days_until(&item.due_date, how.today) {
                Some(..0) => ("overdue", "Overdue", 0),
                Some(days) if days < WEEK_DAYS => ("week", "Due this week", 1),
                Some(_) => ("later", "Later", 2),
                None => ("none", "No due date", 3),
            };
            (format!("due:{id}"), label.into(), Some(rank))
        }
    }
}

/// The list's lines for items `shown` (indices into `items`): what
/// `needs_you` picks under a pinned "Needs you" header, then the rest in
/// groups, each sorted, a folded group's items left out. With no grouping
/// and nothing pinned, the items alone.
pub(super) fn arrange(
    items: &[WorkItem],
    shown: &[usize],
    needs_you: impl Fn(&WorkItem) -> bool,
    how: &Arrange,
) -> Vec<Entry> {
    let mut sorted: Vec<usize> = shown.to_vec();
    sorted.sort_by_cached_key(|ix| sort_key(&items[*ix], how.prefs.sort));
    let (pinned, rest): (Vec<usize>, Vec<usize>) =
        sorted.into_iter().partition(|ix| needs_you(&items[*ix]));
    let mut entries = Vec::with_capacity(shown.len() + 8);
    let mut push_group = |id: String, label: String, members: &[usize], pinned: bool| {
        if members.is_empty() {
            return;
        }
        let collapsed = how.prefs.is_collapsed(&id);
        entries.push(Entry::Header(Header {
            id,
            label,
            count: members.len(),
            collapsed,
            pinned,
        }));
        if !collapsed {
            entries.extend(members.iter().copied().map(Entry::Item));
        }
    };
    push_group(NEEDS_YOU.into(), "Needs you".into(), &pinned, true);
    if how.prefs.group == GroupBy::None && pinned.is_empty() {
        entries.extend(rest.into_iter().map(Entry::Item));
        return entries;
    }
    struct Group {
        id: String,
        label: String,
        /// Its rank, then the order its first item came in.
        order: (usize, usize),
        members: Vec<usize>,
    }
    // Groups in their rank's order, else as their first item comes.
    let mut groups: Vec<Group> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for ix in rest {
        let (id, label, rank) = group_of(&items[ix], how);
        let slot = *at.entry(id.clone()).or_insert_with(|| {
            let came = groups.len();
            groups.push(Group {
                id,
                label,
                order: (rank.unwrap_or(came), came),
                members: Vec::new(),
            });
            came
        });
        groups[slot].members.push(ix);
    }
    groups.sort_by_key(|group| group.order);
    for group in groups {
        push_group(group.id, group.label, &group.members, false);
    }
    entries
}

/// Each line's height, for the virtual list.
pub(super) fn heights(entries: &[Entry]) -> Vec<Option<f32>> {
    entries.iter().map(|entry| Some(entry.height())).collect()
}

/// The line `delta` items from line `current`, headers skipped; from no
/// item, the first (or, going up, the last).
pub(super) fn step(entries: &[Entry], current: Option<usize>, delta: isize) -> Option<usize> {
    let items: Vec<usize> = (entries.iter().enumerate())
        .filter(|(_, entry)| matches!(entry, Entry::Item(_)))
        .map(|(line, _)| line)
        .collect();
    let last = items.len().checked_sub(1)?;
    let at = match current.and_then(|line| items.iter().position(|l| *l == line)) {
        Some(at) => at.saturating_add_signed(delta).min(last),
        None if delta < 0 => last,
        None => 0,
    };
    Some(items[at])
}

/// The line item `ix` is drawn on.
pub(super) fn line_of(entries: &[Entry], ix: usize) -> Option<usize> {
    entries.iter().position(|entry| *entry == Entry::Item(ix))
}

/// The line to bring into view with line `line`: its group's header when
/// it is the group's first item.
pub(super) fn with_header(entries: &[Entry], line: usize) -> usize {
    match line.checked_sub(1).map(|before| &entries[before]) {
        Some(Entry::Header(_)) => line - 1,
        _ => line,
    }
}

/// Where line `line` starts, below the list's `lead`.
pub(super) fn top_of(entries: &[Entry], line: usize, lead: f32) -> f32 {
    lead + entries[..line].iter().map(Entry::height).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Kind;

    fn backlog(key: &str, state: &str, due: &str, updated: &str) -> WorkItem {
        WorkItem {
            provider: Provider::Backlog,
            identifier: key.into(),
            repo: key.rsplit_once('-').unwrap().0.into(),
            state: state.into(),
            due_date: due.into(),
            updated_at: updated.into(),
            created_at: updated.into(),
            ..Default::default()
        }
    }

    fn pr(number: i64, draft: bool, updated: &str) -> WorkItem {
        WorkItem {
            kind: Kind::Pr,
            number,
            repo: "o/r".into(),
            state: "OPEN".into(),
            draft,
            updated_at: updated.into(),
            ..Default::default()
        }
    }

    fn items() -> Vec<WorkItem> {
        vec![
            backlog("A-1", "In Progress", "2026-10-27", "2026-10-28T09:00:00Z"),
            backlog("A-2", "Open", "2026-11-01", "2026-10-28T08:00:00Z"),
            pr(7, true, "2026-10-28T07:00:00Z"),
            backlog("B-3", "Open", "", "2026-10-28T06:00:00Z"),
            backlog("B-4", "Resolved", "2026-11-20", "2026-10-28T10:00:00Z"),
            pr(8, false, "2026-10-28T05:00:00Z"),
        ]
    }

    fn lines(entries: &[Entry], items: &[WorkItem]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Header(h) => format!("[{} {}]", h.label, h.count),
                Entry::Item(ix) if items[*ix].provider == Provider::Backlog => {
                    items[*ix].identifier.clone()
                }
                Entry::Item(ix) => format!("#{}", items[*ix].number),
            })
            .collect()
    }

    fn arranged(prefs: &ListPrefs, needs: &[&str]) -> Vec<String> {
        let items = items();
        let shown: Vec<usize> = (0..items.len()).collect();
        let workflow = ["Open", "In Progress", "Resolved"].map(String::from);
        let how = Arrange {
            prefs,
            workflow: &workflow,
            today: Date::new(2026, 10, 28).unwrap(),
        };
        let needs_you = |item: &WorkItem| needs.contains(&item.identifier.as_str());
        lines(&arrange(&items, &shown, needs_you, &how), &items)
    }

    #[test]
    fn ungrouped_lists_by_update_with_nothing_pinned() {
        assert_eq!(
            arranged(&ListPrefs::default(), &[]),
            ["B-4", "A-1", "A-2", "#7", "B-3", "#8"]
        );
    }

    #[test]
    fn what_needs_you_is_pinned_above_the_rest() {
        assert_eq!(
            arranged(&ListPrefs::default(), &["A-2"]),
            [
                "[Needs you 1]",
                "A-2",
                "[Everything else 5]",
                "B-4",
                "A-1",
                "#7",
                "B-3",
                "#8"
            ]
        );
    }

    #[test]
    fn statuses_follow_the_workflow_then_github() {
        let prefs = ListPrefs {
            group: GroupBy::Status,
            ..Default::default()
        };
        assert_eq!(
            arranged(&prefs, &["B-4"]),
            [
                "[Needs you 1]",
                "B-4",
                "[Open 3]",
                "A-2",
                "B-3",
                "#8",
                "[In Progress 1]",
                "A-1",
                "[Draft 1]",
                "#7",
            ]
        );
    }

    #[test]
    fn due_dates_fall_in_buckets_and_folded_groups_keep_their_header() {
        let prefs = ListPrefs {
            group: GroupBy::Due,
            collapsed: vec!["due:none".into()],
            ..Default::default()
        };
        assert_eq!(
            arranged(&prefs, &[]),
            [
                "[Overdue 1]",
                "A-1",
                "[Due this week 1]",
                "A-2",
                "[Later 1]",
                "B-4",
                "[No due date 3]",
            ]
        );
    }

    #[test]
    fn projects_group_in_the_order_their_first_item_comes() {
        let prefs = ListPrefs {
            group: GroupBy::Project,
            sort: SortBy::Due,
            ..Default::default()
        };
        assert_eq!(
            arranged(&prefs, &[]),
            [
                "[A 2]", "A-1", "A-2", "[B 2]", "B-4", "B-3", "[o/r 2]", "#7", "#8"
            ]
        );
    }

    #[test]
    fn sorts_by_priority_and_creation() {
        let first = |list: &[WorkItem], sort| {
            (0..list.len())
                .min_by_key(|ix| sort_key(&list[*ix], sort))
                .unwrap()
        };
        let mut list = items();
        list[3].high_priority = true;
        assert_eq!(first(&list, SortBy::Priority), 3);
        assert_eq!(first(&list, SortBy::Created), 4);
        list[0].created_at = "2026-10-29T00:00:00Z".into();
        assert_eq!(first(&list, SortBy::Created), 0);
    }

    #[test]
    fn keys_step_over_headers() {
        let entries = vec![
            Entry::Header(Header {
                id: NEEDS_YOU.into(),
                label: "Needs you".into(),
                count: 1,
                collapsed: false,
                pinned: true,
            }),
            Entry::Item(4),
            Entry::Header(Header {
                id: REST.into(),
                label: "Everything else".into(),
                count: 2,
                collapsed: false,
                pinned: false,
            }),
            Entry::Item(0),
            Entry::Item(2),
        ];
        assert_eq!(step(&entries, None, 1), Some(1));
        assert_eq!(step(&entries, None, -1), Some(4));
        assert_eq!(step(&entries, Some(1), 1), Some(3), "over the header");
        assert_eq!(step(&entries, Some(3), -1), Some(1));
        assert_eq!(step(&entries, Some(4), 1), Some(4), "stays at the end");
        assert_eq!(step(&entries, Some(1), -1), Some(1));
        assert_eq!(step(&entries[..1], None, 1), None);
        assert_eq!(line_of(&entries, 2), Some(4));
        assert_eq!(with_header(&entries, 3), 2);
        assert_eq!(with_header(&entries, 4), 4);
        assert_eq!(
            top_of(&entries, 3, 6.0),
            6.0 + HEADER_HEIGHT * 2.0 + ROW_HEIGHT
        );
    }

    #[test]
    fn prefs_round_trip_and_fold() {
        let mut prefs = ListPrefs {
            group: GroupBy::Status,
            sort: SortBy::Priority,
            collapsed: vec!["status:Open".into()],
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert_eq!(
            json,
            r#"{"group":"status","sort":"priority","collapsed":["status:Open"]}"#
        );
        assert_eq!(serde_json::from_str::<ListPrefs>(&json).unwrap(), prefs);
        assert_eq!(
            serde_json::from_str::<ListPrefs>("{}").unwrap(),
            ListPrefs::default()
        );
        prefs.toggle("status:Open");
        prefs.toggle(NEEDS_YOU);
        assert_eq!(prefs.collapsed, [NEEDS_YOU]);
    }
}
