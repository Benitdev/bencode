//! A Backlog issue's editable fields, as its page in Backlog offers them:
//! assignee, priority, milestones, resolution, start and due dates, and
//! estimated and actual hours. `fields` reads them off an issue, `options`
//! lists what a project's issues can be set to, and `update` changes one
//! (`PATCH /issues/:key`, an empty value clearing it).

use serde_json::Value;

use super::{Config, IssueRow, call, require_config, require_issue_key, text, work_item};
use crate::work_items::WorkItem;

/// One choice of a field: Backlog's id and the name it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub id: i64,
    pub name: String,
}

/// An issue's fields as they are now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IssueFields {
    pub assignee: Option<Named>,
    pub priority: Option<Named>,
    pub milestones: Vec<Named>,
    pub resolution: Option<Named>,
    /// `YYYY-MM-DD`.
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub estimated_hours: Option<f64>,
    pub actual_hours: Option<f64>,
}

/// What a project's issues can be set to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldOptions {
    /// The project's members.
    pub users: Vec<Named>,
    /// Its milestones and versions that are not archived.
    pub milestones: Vec<Named>,
    pub priorities: Vec<Named>,
    pub resolutions: Vec<Named>,
    /// The connected user, for "Assign to me".
    pub me: i64,
}

/// One change to an issue.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Status(i64),
    Assignee(Option<i64>),
    Priority(i64),
    Milestones(Vec<i64>),
    Resolution(Option<i64>),
    StartDate(Option<String>),
    DueDate(Option<String>),
    EstimatedHours(Option<f64>),
    ActualHours(Option<f64>),
}

impl Change {
    /// The form `PATCH /issues/:key` takes; an empty value clears.
    fn form(&self) -> Vec<(&'static str, String)> {
        let id = |id: &Option<i64>| id.map(|id| id.to_string()).unwrap_or_default();
        let hours = |hours: &Option<f64>| hours.map(|h| h.to_string()).unwrap_or_default();
        match self {
            Self::Status(id) => vec![("statusId", id.to_string())],
            Self::Assignee(user) => vec![("assigneeId", id(user))],
            Self::Priority(id) => vec![("priorityId", id.to_string())],
            Self::Milestones(ids) if ids.is_empty() => vec![("milestoneId[]", String::new())],
            Self::Milestones(ids) => ids
                .iter()
                .map(|id| ("milestoneId[]", id.to_string()))
                .collect(),
            Self::Resolution(resolution) => vec![("resolutionId", id(resolution))],
            Self::StartDate(date) => vec![("startDate", date.clone().unwrap_or_default())],
            Self::DueDate(date) => vec![("dueDate", date.clone().unwrap_or_default())],
            Self::EstimatedHours(h) => vec![("estimatedHours", hours(h))],
            Self::ActualHours(h) => vec![("actualHours", hours(h))],
        }
    }
}

/// `2024-05-31T00:00:00Z`: the day is all Backlog means.
fn day(date: &Option<String>) -> Option<String> {
    date.as_deref()
        .map(|date| date.chars().take(10).collect::<String>())
        .filter(|date| !date.is_empty())
}

fn named(named: &super::Named) -> Named {
    Named {
        id: named.id,
        name: named.name.clone(),
    }
}

/// The fields of an issue Backlog sent.
pub(super) fn fields(row: &IssueRow) -> IssueFields {
    IssueFields {
        assignee: row.assignee.as_ref().map(named),
        priority: row.priority.as_ref().map(named),
        milestones: row.milestone.iter().map(named).collect(),
        resolution: row.resolution.as_ref().map(named),
        start_date: day(&row.start_date),
        due_date: day(&row.due_date),
        estimated_hours: row.estimated_hours,
        actual_hours: row.actual_hours,
    }
}

/// A list of `{id, name}` rows; `skip` leaves some out.
fn parse_named(data: &Value, skip: impl Fn(&Value) -> bool) -> Vec<Named> {
    (data.as_array().into_iter().flatten())
        .filter(|row| !skip(row))
        .filter_map(|row| {
            Some(Named {
                id: row.get("id")?.as_i64()?,
                name: text(row, "name"),
            })
        })
        .filter(|named| !named.name.trim().is_empty())
        .collect()
}

/// What `project_id`'s issues can be set to: four requests, side by side.
pub fn options(project_id: &str) -> Result<FieldOptions, String> {
    let config = require_config()?;
    let get = |path: String| call(&config, None, &path, &[], &[]);
    let (users, milestones, priorities, resolutions) = std::thread::scope(|scope| {
        let users = scope.spawn(|| get(format!("/projects/{project_id}/users")));
        let milestones = scope.spawn(|| get(format!("/projects/{project_id}/versions")));
        let priorities = scope.spawn(|| get("/priorities".into()));
        let resolutions = scope.spawn(|| get("/resolutions".into()));
        let join = |handle: std::thread::ScopedJoinHandle<'_, Result<Value, String>>| {
            handle
                .join()
                .unwrap_or_else(|_| Err("Backlog request failed".into()))
        };
        (
            join(users),
            join(milestones),
            join(priorities),
            join(resolutions),
        )
    });
    let archived = |row: &Value| row.get("archived").and_then(Value::as_bool) == Some(true);
    Ok(FieldOptions {
        users: parse_named(&users?, |_| false),
        milestones: parse_named(&milestones?, archived),
        priorities: parse_named(&priorities?, |_| false),
        resolutions: parse_named(&resolutions?, |_| false),
        me: config.user_id,
    })
}

/// Applies `change` to the issue; returns it as it is now.
pub fn update(key: &str, change: &Change) -> Result<(WorkItem, IssueFields), String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    patch(&config, key, change)
}

fn patch(config: &Config, key: &str, change: &Change) -> Result<(WorkItem, IssueFields), String> {
    let form = change.form();
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let data = call(config, Some("PATCH"), &format!("/issues/{key}"), &[], &form)?;
    let row: IssueRow =
        serde_json::from_value(data).map_err(|_| "Backlog sent an unreadable issue".to_string())?;
    Ok((work_item(&row, &config.space), fields(&row)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changes_are_sent_as_backlog_names_them_and_empty_clears() {
        assert_eq!(
            Change::Assignee(Some(7)).form(),
            [("assigneeId", "7".to_string())]
        );
        assert_eq!(
            Change::Assignee(None).form(),
            [("assigneeId", String::new())]
        );
        assert_eq!(
            Change::Milestones(vec![1, 2]).form(),
            [("milestoneId[]", "1".into()), ("milestoneId[]", "2".into())]
        );
        assert_eq!(
            Change::Milestones(Vec::new()).form(),
            [("milestoneId[]", String::new())]
        );
        assert_eq!(
            Change::DueDate(Some("2026-10-21".into())).form(),
            [("dueDate", "2026-10-21".into())]
        );
        assert_eq!(
            Change::EstimatedHours(Some(1.5)).form(),
            [("estimatedHours", "1.5".into())]
        );
        assert_eq!(
            Change::ActualHours(None).form(),
            [("actualHours", String::new())]
        );
    }

    #[test]
    fn an_issue_reads_as_its_fields() {
        let row: IssueRow = serde_json::from_value(json!({
            "issueKey": "P-1",
            "assignee": {"id": 3, "name": "Yumeko"},
            "priority": {"id": 3, "name": "Normal"},
            "milestone": [{"id": 9, "name": "Sprint 04"}],
            "resolution": null,
            "startDate": "2026-10-01T00:00:00Z",
            "dueDate": null,
            "estimatedHours": 2.5,
        }))
        .unwrap();
        let fields = fields(&row);
        assert_eq!(fields.assignee.unwrap().name, "Yumeko");
        assert_eq!(fields.milestones[0].id, 9);
        assert_eq!(fields.resolution, None);
        assert_eq!(fields.start_date.as_deref(), Some("2026-10-01"));
        assert_eq!(fields.due_date, None);
        assert_eq!(fields.estimated_hours, Some(2.5));
        assert_eq!(fields.actual_hours, None);
    }

    #[test]
    fn archived_milestones_are_not_offered() {
        let data = json!([
            {"id": 1, "name": "Sprint 01", "archived": true},
            {"id": 2, "name": "Sprint 02", "archived": false},
        ]);
        let archived = |row: &Value| row.get("archived").and_then(Value::as_bool) == Some(true);
        assert_eq!(
            parse_named(&data, archived),
            [Named {
                id: 2,
                name: "Sprint 02".into()
            }]
        );
    }
}
