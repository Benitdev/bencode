//! The rail's saved state: order, width, archive, project groups, each
//! project's label / colour / mascot / logo, and notification mutes.
//! MonoCode keeps these in webview `localStorage`; BenCode keeps them in
//! `settings.json` under the same names (`monocode.projectRailOrder` is
//! `projectRailOrder`, `monocode:tab-group:labels` is `tabGroupLabels`, …)
//! with the same meaning.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::app::normalize_project_path;

/// MonoCode `PROJECT_RAIL_WIDTH_MIN` / `MAX` / `DEFAULT`.
pub const RAIL_WIDTH_MIN: f32 = 180.0;
pub const RAIL_WIDTH_MAX: f32 = 360.0;
pub const RAIL_WIDTH_DEFAULT: f32 = 200.0;
/// MonoCode `useDragResize` `max`: at most 35% of the window.
const RAIL_WIDTH_WINDOW_SHARE: f32 = 0.35;
/// MonoCode `NOTIFICATION_MUTE_HOURS`.
pub const MUTE_HOURS: [i64; 3] = [1, 4, 8];
const HOUR_MS: i64 = 3_600_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RailPrefs {
    /// `monocode.projectRailOrder`: every rail project, in rail order.
    pub project_rail_order: Vec<String>,
    /// `monocode.projectRailWidth`; `None` is the default.
    pub project_rail_width: Option<f32>,
    /// `monocode.archivedProjects`, newest first.
    pub archived_projects: Vec<ArchivedProject>,
    /// `monocode.projectGroups`.
    pub project_groups: Vec<ProjectGroup>,
    /// `monocode.projectGroupAssignments`: project path → group id.
    pub project_group_assignments: BTreeMap<String, String>,
    /// `monocode:tab-group:labels`: project path → shown name.
    pub tab_group_labels: BTreeMap<String, String>,
    /// `monocode:tab-group:colors`: project path → palette index.
    pub tab_group_colors: BTreeMap<String, usize>,
    /// `monocode:tab-group:custom-colors`: project path → `#rrggbb`.
    pub tab_group_custom_colors: BTreeMap<String, String>,
    /// `monocode:tab-group:mascots`: project path → mascot name.
    pub tab_group_mascots: BTreeMap<String, String>,
    /// `monocode:tab-group:logos`: project path → image file.
    pub tab_group_logos: BTreeMap<String, String>,
    /// `monocode.projectNotifications.v1`, by notification project id.
    pub project_notifications: BTreeMap<String, NotificationPreference>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedProject {
    pub path: String,
    #[serde(default)]
    pub archived_at: i64,
}

/// MonoCode `ProjectGroup`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mascot: Option<String>,
}

/// MonoCode `ProjectNotificationPreference`. `muted_until` is absent for
/// no mute, `Some(None)` (JSON `null`) until resumed, else a deadline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPreference {
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub muted_until: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<i64>,
    /// `enabledAfter` and anything newer, kept as written.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A present key becomes `Some`, so `null` survives as `Some(None)`.
fn present_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Option::<i64>::deserialize(d).map(Some)
}

impl NotificationPreference {
    /// MonoCode `isProjectMuted`.
    pub fn is_muted(&self, now: i64) -> bool {
        match self.muted_until {
            Some(None) => true,
            Some(Some(until)) => until > now,
            None => false,
        }
    }
}

/// MonoCode `notificationMuteStatus`: the rail's and menus' label.
pub fn mute_status(pref: Option<&NotificationPreference>, now: i64) -> Option<String> {
    let pref = pref.filter(|p| p.is_muted(now))?;
    Some(match pref.muted_until.flatten() {
        None => "Muted until resumed".to_string(),
        Some(until) => format!("Muted until {}", format_mute_deadline(until)),
    })
}

/// `toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })`.
fn format_mute_deadline(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()))
        .map_or_else(
            |_| String::new(),
            |z| z.strftime("%b %-d, %Y, %-I:%M %p").to_string(),
        )
}

/// MonoCode `localNotificationProject` id: BenCode knows no hosted
/// project catalog, so every project is notified about as a folder.
pub fn notification_id(path: &str) -> String {
    format!("local:{}", path_key(path))
}

/// One row of MonoCode `notificationMuteActions`.
pub struct MuteAction {
    pub id: &'static str,
    pub label: String,
}

/// MonoCode `notificationMuteActions`: 1, 4 and 8 hours (with the clock
/// time they end), until resumed, and a chosen date.
pub fn mute_actions(now: i64) -> Vec<MuteAction> {
    let ids = ["mute:1", "mute:4", "mute:8"];
    let today = local_date(now);
    let mut actions: Vec<MuteAction> = MUTE_HOURS
        .iter()
        .zip(ids)
        .map(|(hours, id)| {
            let until = now + hours * HOUR_MS;
            let day = if local_date(until) == today {
                ""
            } else {
                "Tomorrow, "
            };
            let unit = if *hours == 1 { "hour" } else { "hours" };
            MuteAction {
                id,
                label: format!("{hours} {unit} ({day}{})", clock_time(until)),
            }
        })
        .collect();
    actions.push(MuteAction {
        id: "mute:indefinite",
        label: "Until resumed".into(),
    });
    actions.push(MuteAction {
        id: "mute:custom",
        label: "Choose date and time".into(),
    });
    actions
}

/// MonoCode `notificationMuteDeadline`: `Some(None)` mutes until resumed.
pub fn mute_deadline(id: &str, now: i64) -> Option<Option<i64>> {
    if id == "mute:indefinite" {
        return Some(None);
    }
    let hours: i64 = id.strip_prefix("mute:")?.parse().ok()?;
    MUTE_HOURS
        .contains(&hours)
        .then_some(Some(now + hours * HOUR_MS))
}

fn local_date(ms: i64) -> Option<jiff::civil::Date> {
    let t = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(t.to_zoned(jiff::tz::TimeZone::system()).date())
}

/// `${hours}:${minutes}`, 24-hour, as MonoCode writes it.
fn clock_time(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()))
        .map_or_else(
            |_| String::new(),
            |z| format!("{}:{:02}", z.hour(), z.minute()),
        )
}

impl RailPrefs {
    /// MonoCode `updateNotificationPreferences` for a mute or a resume
    /// (`muted_until: None`), stamping `resumedAt` when a mute is lifted.
    pub fn with_mute(&self, ids: &[String], muted_until: Option<Option<i64>>, now: i64) -> Self {
        let mut notifications = self.project_notifications.clone();
        for id in ids {
            let previous = notifications.get(id).cloned().unwrap_or_default();
            let resuming = muted_until.is_none() && previous.muted_until.is_some();
            notifications.insert(
                id.clone(),
                NotificationPreference {
                    muted_until,
                    resumed_at: if resuming {
                        Some(now)
                    } else {
                        previous.resumed_at
                    },
                    ..previous
                },
            );
        }
        Self {
            project_notifications: notifications,
            ..self.clone()
        }
    }

    pub fn is_archived(&self, path: &str) -> bool {
        self.archived_projects.iter().any(|a| same(&a.path, path))
    }

    /// MonoCode `archiveProject`: off the rail (order and pins aside) and
    /// first in the archive.
    pub fn with_archived(&self, path: &str, now: i64) -> Self {
        let path = normalize_project_path(path);
        let archived = std::iter::once(ArchivedProject {
            path: path.clone(),
            archived_at: now,
        })
        .chain(
            self.archived_projects
                .iter()
                .filter(|a| !same(&a.path, &path))
                .cloned(),
        )
        .collect();
        Self {
            archived_projects: archived,
            project_rail_order: without(&self.project_rail_order, &path),
            ..self.clone()
        }
    }

    /// MonoCode `forgetProject` plus `clearTabGroupSettings` and the
    /// group assignment: nothing about the project is kept.
    pub fn without_project(&self, path: &str) -> Self {
        let key = path_key(path);
        let drop = |map: &BTreeMap<String, String>| {
            map.iter()
                .filter(|(k, _)| **k != key)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        Self {
            archived_projects: self
                .archived_projects
                .iter()
                .filter(|a| !same(&a.path, path))
                .cloned()
                .collect(),
            project_rail_order: without(&self.project_rail_order, path),
            project_group_assignments: drop(&self.project_group_assignments),
            tab_group_labels: drop(&self.tab_group_labels),
            tab_group_colors: self
                .tab_group_colors
                .iter()
                .filter(|(k, _)| **k != key)
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
            tab_group_custom_colors: drop(&self.tab_group_custom_colors),
            tab_group_mascots: drop(&self.tab_group_mascots),
            tab_group_logos: drop(&self.tab_group_logos),
            ..self.clone()
        }
    }

    /// Back on the rail (MonoCode `rememberProject` drops it from the archive).
    pub fn with_restored(&self, path: &str) -> Self {
        Self {
            archived_projects: self
                .archived_projects
                .iter()
                .filter(|a| !same(&a.path, path))
                .cloned()
                .collect(),
            ..self.clone()
        }
    }

    pub fn group_of(&self, path: &str) -> Option<&ProjectGroup> {
        let id = self.project_group_assignments.get(&path_key(path))?;
        self.project_groups.iter().find(|g| &g.id == id)
    }

    /// MonoCode `setProjectGroupAssignment`; an unknown group is ignored.
    pub fn with_assignment(&self, path: &str, group_id: Option<&str>) -> Self {
        let key = path_key(path);
        let mut assignments = self.project_group_assignments.clone();
        match group_id {
            None => {
                assignments.remove(&key);
            }
            Some(id) if self.project_groups.iter().any(|g| g.id == id) => {
                assignments.insert(key, id.to_string());
            }
            Some(_) => return self.clone(),
        }
        Self {
            project_group_assignments: assignments,
            ..self.clone()
        }
    }

    /// MonoCode `createProjectGroup` + `saveProjectGroups`.
    pub fn with_new_group(&self, id: String) -> Self {
        let name = next_group_name(&self.project_groups);
        let groups = self
            .project_groups
            .iter()
            .cloned()
            .chain(std::iter::once(ProjectGroup {
                id,
                name,
                ..Default::default()
            }))
            .collect();
        Self {
            project_groups: groups,
            ..self.clone()
        }
    }

    /// MonoCode `updateProjectGroup`.
    pub fn with_group(&self, id: &str, update: impl Fn(&ProjectGroup) -> ProjectGroup) -> Self {
        Self {
            project_groups: self
                .project_groups
                .iter()
                .map(|g| if g.id == id { update(g) } else { g.clone() })
                .collect(),
            ..self.clone()
        }
    }

    /// MonoCode `deleteProjectGroup`: its projects become ungrouped.
    pub fn without_group(&self, id: &str) -> Self {
        Self {
            project_groups: self
                .project_groups
                .iter()
                .filter(|g| g.id != id)
                .cloned()
                .collect(),
            project_group_assignments: self
                .project_group_assignments
                .iter()
                .filter(|(_, g)| *g != id)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ..self.clone()
        }
    }

    /// MonoCode `saveTabGroupLabel`: blank restores the folder name.
    pub fn with_label(&self, path: &str, label: &str) -> Self {
        let mut labels = self.tab_group_labels.clone();
        match label.trim() {
            "" => labels.remove(&path_key(path)),
            trimmed => labels.insert(path_key(path), trimmed.to_string()),
        };
        Self {
            tab_group_labels: labels,
            ..self.clone()
        }
    }

    /// MonoCode `saveTabGroupColor`: a palette pick clears a custom colour;
    /// `None` restores the hashed one.
    pub fn with_color(&self, path: &str, index: Option<usize>) -> Self {
        let key = path_key(path);
        let mut colors = self.tab_group_colors.clone();
        let mut custom = self.tab_group_custom_colors.clone();
        custom.remove(&key);
        match index {
            Some(index) => colors.insert(key, index),
            None => colors.remove(&key),
        };
        Self {
            tab_group_colors: colors,
            tab_group_custom_colors: custom,
            ..self.clone()
        }
    }

    /// MonoCode `saveTabGroupCustomColor`: clears the palette pick.
    pub fn with_custom_color(&self, path: &str, hex: &str) -> Self {
        let key = path_key(path);
        let mut colors = self.tab_group_colors.clone();
        let mut custom = self.tab_group_custom_colors.clone();
        colors.remove(&key);
        custom.insert(key, hex.to_lowercase());
        Self {
            tab_group_colors: colors,
            tab_group_custom_colors: custom,
            ..self.clone()
        }
    }

    pub fn with_mascot(&self, path: &str, name: Option<&str>) -> Self {
        Self {
            tab_group_mascots: with_entry(&self.tab_group_mascots, &path_key(path), name),
            ..self.clone()
        }
    }

    pub fn with_logo(&self, path: &str, file: Option<&str>) -> Self {
        Self {
            tab_group_logos: with_entry(&self.tab_group_logos, &path_key(path), file),
            ..self.clone()
        }
    }

    /// MonoCode `resolveTabGroupLabel`.
    pub fn label<'a>(&'a self, path: &str, fallback: &'a str) -> &'a str {
        self.tab_group_labels
            .get(&path_key(path))
            .map_or(fallback, String::as_str)
    }

    /// The width shown, clamped like MonoCode `loadProjectRailWidth`.
    pub fn rail_width(&self) -> f32 {
        self.project_rail_width
            .unwrap_or(RAIL_WIDTH_DEFAULT)
            .clamp(RAIL_WIDTH_MIN, RAIL_WIDTH_MAX)
            .round()
    }
}

fn with_entry(
    map: &BTreeMap<String, String>,
    key: &str,
    value: Option<&str>,
) -> BTreeMap<String, String> {
    let mut next = map.clone();
    match value {
        Some(value) => next.insert(key.to_string(), value.to_string()),
        None => next.remove(key),
    };
    next
}

fn same(a: &str, b: &str) -> bool {
    crate::app::same_project_path(a, b)
}

fn without(paths: &[String], path: &str) -> Vec<String> {
    paths.iter().filter(|p| !same(p, path)).cloned().collect()
}

/// MonoCode `pathKey` / `projectKey` (macOS paths keep their case).
pub fn path_key(path: &str) -> String {
    normalize_project_path(path)
}

/// MonoCode `nextProjectGroupName`.
pub fn next_group_name(groups: &[ProjectGroup]) -> String {
    let taken = |name: &str| {
        groups
            .iter()
            .any(|g| g.name.to_lowercase() == name.to_lowercase())
    };
    if !taken("New group") {
        return "New group".into();
    }
    (2..)
        .map(|n| format!("New group {n}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

/// MonoCode `syncProjectRailOrder`: the saved order, then projects it has
/// not seen in `projects` order (newest first), nothing twice.
pub fn sync_rail_order(order: &[String], projects: &[String]) -> Vec<String> {
    let mut next: Vec<String> = Vec::new();
    for path in order {
        if let Some(project) = projects.iter().find(|p| same(p, path))
            && !next.iter().any(|n| same(n, project))
        {
            next.push(project.clone());
        }
    }
    let newcomers: Vec<String> = projects
        .iter()
        .filter(|p| !next.iter().any(|n| same(n, p)))
        .cloned()
        .collect();
    next.extend(newcomers);
    next
}

/// The rail's lists (MonoCode `projectRailSections` and
/// `groupedProjectSections`): pinned in rail order, each group's projects,
/// then the rest.
#[derive(Debug, Default, PartialEq)]
pub struct RailSections {
    pub pinned: Vec<String>,
    pub groups: Vec<(ProjectGroup, Vec<String>)>,
    pub ungrouped: Vec<String>,
}

pub fn rail_sections(order: &[String], pinned: &[String], prefs: &RailPrefs) -> RailSections {
    let is_pinned = |path: &str| pinned.iter().any(|p| same(p, path));
    let mut groups: Vec<(ProjectGroup, Vec<String>)> = prefs
        .project_groups
        .iter()
        .map(|g| (g.clone(), Vec::new()))
        .collect();
    let mut sections = RailSections::default();
    for path in order {
        if is_pinned(path) {
            sections.pinned.push(path.clone());
            continue;
        }
        let group = prefs.group_of(path).map(|g| g.id.clone());
        match group.and_then(|id| groups.iter_mut().find(|(g, _)| g.id == id)) {
            Some((_, items)) => items.push(path.clone()),
            None => sections.ungrouped.push(path.clone()),
        }
    }
    sections.groups = groups;
    sections
}

/// MonoCode `reorderSubset`: `subset_order` takes the slots its members
/// held in `full`, everything else stays put.
pub fn reorder_subset(full: &[String], subset_order: &[String]) -> Vec<String> {
    let mut slots = subset_order.iter();
    full.iter()
        .filter_map(|path| {
            if subset_order.iter().any(|s| same(s, path)) {
                slots.next().cloned()
            } else {
                Some(path.clone())
            }
        })
        .collect()
}

/// MonoCode `moveItem`.
pub fn move_item(ids: &[String], from: usize, to: usize) -> Vec<String> {
    let mut next = ids.to_vec();
    if from < next.len() {
        let moved = next.remove(from);
        next.insert(to.min(next.len()), moved);
    }
    next
}

/// MonoCode `useDragResize`: the drag's width, clamped to the rail's range
/// and 35% of the window.
pub fn resized_rail_width(start_width: f32, delta: f32, window_width: f32) -> f32 {
    let max = RAIL_WIDTH_MAX
        .min((window_width * RAIL_WIDTH_WINDOW_SHARE).floor())
        .max(RAIL_WIDTH_MIN);
    (start_width + delta).round().clamp(RAIL_WIDTH_MIN, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn saved_order_comes_first_then_newcomers() {
        let order = paths(&["/b", "/gone", "/a/"]);
        let projects = paths(&["/c", "/a", "/b"]);
        assert_eq!(
            sync_rail_order(&order, &projects),
            paths(&["/b", "/a", "/c"])
        );
    }

    #[test]
    fn sections_split_pins_groups_and_the_rest() {
        let prefs = RailPrefs::default()
            .with_new_group("g1".into())
            .with_assignment("/b", Some("g1"))
            .with_assignment("/c", Some("missing"));
        let sections = rail_sections(&paths(&["/a", "/b", "/c", "/d"]), &paths(&["/d"]), &prefs);
        assert_eq!(sections.pinned, paths(&["/d"]));
        assert_eq!(sections.groups.len(), 1);
        assert_eq!(sections.groups[0].1, paths(&["/b"]));
        assert_eq!(sections.ungrouped, paths(&["/a", "/c"]));
    }

    #[test]
    fn subset_reorder_keeps_other_slots() {
        let full = paths(&["/a", "/x", "/b", "/c"]);
        let next = reorder_subset(&full, &paths(&["/c", "/a", "/b"]));
        assert_eq!(next, paths(&["/c", "/x", "/a", "/b"]));
        assert_eq!(
            move_item(&paths(&["a", "b", "c"]), 0, 2),
            paths(&["b", "c", "a"])
        );
    }

    #[test]
    fn group_names_count_up_and_deleting_ungroups() {
        let prefs = RailPrefs::default()
            .with_new_group("g1".into())
            .with_new_group("g2".into());
        assert_eq!(prefs.project_groups[1].name, "New group 2");
        let prefs = prefs.with_assignment("/a", Some("g1")).without_group("g1");
        assert!(prefs.group_of("/a").is_none());
        assert!(prefs.project_group_assignments.is_empty());
    }

    #[test]
    fn archive_restore_and_forget() {
        let prefs = RailPrefs {
            project_rail_order: paths(&["/a", "/b"]),
            ..Default::default()
        }
        .with_label("/a", "Alpha")
        .with_archived("/a/", 5);
        assert!(prefs.is_archived("/a"));
        assert_eq!(prefs.project_rail_order, paths(&["/b"]));
        assert!(!prefs.with_restored("/a").is_archived("/a"));
        let forgotten = prefs.without_project("/a");
        assert!(!forgotten.is_archived("/a"));
        assert!(forgotten.tab_group_labels.is_empty());
    }

    #[test]
    fn colours_replace_each_other_and_labels_trim() {
        let prefs = RailPrefs::default().with_custom_color("/a", "#AABBCC");
        assert_eq!(prefs.tab_group_custom_colors["/a"], "#aabbcc");
        let prefs = prefs.with_color("/a", Some(3));
        assert!(prefs.tab_group_custom_colors.is_empty());
        assert_eq!(prefs.tab_group_colors["/a"], 3);
        assert_eq!(prefs.with_label("/a", "  ").label("/a", "a"), "a");
        assert_eq!(prefs.with_label("/a", " Alpha ").label("/a/", "a"), "Alpha");
    }

    #[test]
    fn mutes_round_trip_null_and_resume() {
        let json = r#"{"local:/a":{"disabled":[],"mutedUntil":null,"enabledAfter":{"issues":3}}}"#;
        let parsed: BTreeMap<String, NotificationPreference> = serde_json::from_str(json).unwrap();
        let pref = &parsed["local:/a"];
        assert_eq!(pref.muted_until, Some(None));
        assert!(pref.is_muted(0));
        assert_eq!(
            mute_status(Some(pref), 0).as_deref(),
            Some("Muted until resumed")
        );
        let back = serde_json::to_string(&parsed).unwrap();
        assert!(
            back.contains("\"mutedUntil\":null") && back.contains("enabledAfter"),
            "{back}"
        );

        let prefs = RailPrefs {
            project_notifications: parsed,
            ..Default::default()
        };
        let resumed = prefs.with_mute(&["local:/a".into()], None, 9);
        let pref = &resumed.project_notifications["local:/a"];
        assert!(!pref.is_muted(10));
        assert_eq!(pref.resumed_at, Some(9));
    }

    #[test]
    fn mute_presets_and_deadlines() {
        assert_eq!(mute_deadline("mute:4", 0), Some(Some(4 * HOUR_MS)));
        assert_eq!(mute_deadline("mute:indefinite", 0), Some(None));
        assert_eq!(mute_deadline("mute:3", 0), None);
        let actions = mute_actions(0);
        assert_eq!(actions.len(), 5);
        assert!(
            actions[0].label.starts_with("1 hour ("),
            "{}",
            actions[0].label
        );
        assert_eq!(actions[4].id, "mute:custom");
    }

    #[test]
    fn resize_clamps_to_range_and_window() {
        assert_eq!(resized_rail_width(200.0, 500.0, 2000.0), RAIL_WIDTH_MAX);
        assert_eq!(resized_rail_width(200.0, 500.0, 800.0), 280.0);
        assert_eq!(resized_rail_width(200.0, -100.0, 2000.0), RAIL_WIDTH_MIN);
    }
}
