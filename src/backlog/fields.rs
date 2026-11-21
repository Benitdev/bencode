//! A Backlog issue's editable fields, as its page in Backlog offers them:
//! assignee, priority, milestones, resolution, start and due dates, and
//! estimated and actual hours. `fields` reads them off an issue, `options`
//! lists what a project's issues can be set to, and `update` changes one
//! (`PATCH /issues/:key`, an empty value clearing it; `update_all`, several).

use serde_json::Value;

use super::{Config, IssueRow, call, require_config, require_issue_key, text, work_item};
use crate::work_items::WorkItem;

/// One choice of a field: Backlog's id and the name it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub id: i64,
    pub name: String,
}

/// One attachment listed on an issue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    pub id: i64,
    pub name: String,
    pub size: u64,
}

/// One custom field value as Backlog reports it on an issue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomFieldValue {
    pub id: i64,
    pub name: String,
    pub value: String,
}

/// An issue's fields as they are now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IssueFields {
    pub issue_id: i64,
    pub assignee: Option<Named>,
    pub priority: Option<Named>,
    pub categories: Vec<Named>,
    pub versions: Vec<Named>,
    pub milestones: Vec<Named>,
    pub resolution: Option<Named>,
    pub parent_issue_id: Option<i64>,
    /// `YYYY-MM-DD`.
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub estimated_hours: Option<f64>,
    pub actual_hours: Option<f64>,
    pub attachments: Vec<Attachment>,
    pub custom_fields: Vec<CustomFieldValue>,
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
    Summary(String),
    Description(Option<String>),
    IssueType(i64),
    Status(i64),
    Assignee(Option<i64>),
    Priority(i64),
    Categories(Vec<i64>),
    Versions(Vec<i64>),
    Milestones(Vec<i64>),
    Resolution(Option<i64>),
    ParentIssue(Option<i64>),
    StartDate(Option<String>),
    DueDate(Option<String>),
    EstimatedHours(Option<f64>),
    ActualHours(Option<f64>),
    NotifiedUsers(Vec<i64>),
    Comment(String),
    CustomField {
        id: i64,
        /// Empty clears; one value sends `customField_<id>`, several send
        /// `customField_<id>[0]`, `customField_<id>[1]`, … as Backlog expects.
        values: Vec<String>,
        other_value: Option<String>,
    },
}

impl Change {
    /// The form `PATCH /issues/:key` takes; an empty value clears.
    pub(super) fn form(&self) -> Vec<(String, String)> {
        let id = |id: &Option<i64>| id.map(|id| id.to_string()).unwrap_or_default();
        let hours = |hours: &Option<f64>| hours.map(|h| h.to_string()).unwrap_or_default();
        let id_list = |key: &str, ids: &[i64]| -> Vec<(String, String)> {
            if ids.is_empty() {
                vec![(key.to_string(), String::new())]
            } else {
                ids.iter()
                    .map(|id| (key.to_string(), id.to_string()))
                    .collect()
            }
        };
        match self {
            Self::Summary(s) => vec![("summary".into(), s.clone())],
            Self::Description(d) => vec![("description".into(), d.clone().unwrap_or_default())],
            Self::IssueType(id) => vec![("issueTypeId".into(), id.to_string())],
            Self::Status(id) => vec![("statusId".into(), id.to_string())],
            Self::Assignee(user) => vec![("assigneeId".into(), id(user))],
            Self::Priority(id) => vec![("priorityId".into(), id.to_string())],
            Self::Categories(ids) => id_list("categoryId[]", ids),
            Self::Versions(ids) => id_list("versionId[]", ids),
            Self::Milestones(ids) => id_list("milestoneId[]", ids),
            Self::Resolution(resolution) => vec![("resolutionId".into(), id(resolution))],
            Self::ParentIssue(parent) => vec![("parentIssueId".into(), id(parent))],
            Self::StartDate(date) => vec![("startDate".into(), date.clone().unwrap_or_default())],
            Self::DueDate(date) => vec![("dueDate".into(), date.clone().unwrap_or_default())],
            Self::EstimatedHours(h) => vec![("estimatedHours".into(), hours(h))],
            Self::ActualHours(h) => vec![("actualHours".into(), hours(h))],
            Self::NotifiedUsers(ids) => ids
                .iter()
                .map(|id| ("notifiedUserId[]".into(), id.to_string()))
                .collect(),
            Self::Comment(c) => vec![("comment".into(), c.clone())],
            Self::CustomField {
                id,
                values,
                other_value,
            } => {
                let mut pairs = Vec::new();
                match values.as_slice() {
                    [] => pairs.push((format!("customField_{id}"), String::new())),
                    [one] => pairs.push((format!("customField_{id}"), one.clone())),
                    many => {
                        for (index, value) in many.iter().enumerate() {
                            pairs.push((format!("customField_{id}[{index}]"), value.clone()));
                        }
                    }
                }
                if let Some(other) = other_value {
                    pairs.push((format!("customField_{id}_otherValue"), other.clone()));
                }
                pairs
            }
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

/// Formats one custom field entry from Backlog's issue JSON.
fn parse_custom_field(row: &Value) -> Option<CustomFieldValue> {
    let id = row.get("id")?.as_i64()?;
    let name = text(row, "name");
    if name.trim().is_empty() {
        return None;
    }
    let formatted = match row.get("value") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Object(obj)) => obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("name")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    let other = row
        .get("otherValue")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let value = match (formatted.is_empty(), other) {
        (true, Some(other)) => other.to_string(),
        (false, Some(other)) => format!("{formatted} ({other})"),
        (_, None) => formatted,
    };
    (!value.is_empty()).then_some(CustomFieldValue { id, name, value })
}

/// The fields of an issue Backlog sent.
pub(super) fn fields(row: &IssueRow) -> IssueFields {
    IssueFields {
        issue_id: row.id,
        assignee: row.assignee.as_ref().map(named),
        priority: row.priority.as_ref().map(named),
        categories: row.category.iter().map(named).collect(),
        versions: row.versions.iter().map(named).collect(),
        milestones: row.milestone.iter().map(named).collect(),
        resolution: row.resolution.as_ref().map(named),
        parent_issue_id: row.parent_issue_id,
        start_date: day(&row.start_date),
        due_date: day(&row.due_date),
        estimated_hours: row.estimated_hours,
        actual_hours: row.actual_hours,
        attachments: row
            .attachments
            .iter()
            .map(|a| Attachment {
                id: a.id,
                name: a.name.clone(),
                size: a.size,
            })
            .collect(),
        custom_fields: row
            .custom_fields
            .iter()
            .filter_map(parse_custom_field)
            .collect(),
    }
}

/// A list of `{id, name}` rows; `skip` leaves some out.
pub(super) fn parse_named(data: &Value, skip: impl Fn(&Value) -> bool) -> Vec<Named> {
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
    update_all(key, std::slice::from_ref(change))
}

/// Applies `changes` to the issue at once; returns it as it is now.
pub fn update_all(key: &str, changes: &[Change]) -> Result<(WorkItem, IssueFields), String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    patch(&config, key, changes)
}

fn patch(
    config: &Config,
    key: &str,
    changes: &[Change],
) -> Result<(WorkItem, IssueFields), String> {
    let form: Vec<(String, String)> = changes.iter().flat_map(Change::form).collect();
    if form.is_empty() {
        return Err("Nothing to change".into());
    }
    let form: Vec<(&str, &str)> = form
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
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
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(Change::Assignee(Some(7)).form(), [pair("assigneeId", "7")]);
        assert_eq!(Change::Assignee(None).form(), [pair("assigneeId", "")]);
        assert_eq!(
            Change::Milestones(vec![1, 2]).form(),
            [pair("milestoneId[]", "1"), pair("milestoneId[]", "2")]
        );
        assert_eq!(
            Change::Milestones(Vec::new()).form(),
            [pair("milestoneId[]", "")]
        );
        assert_eq!(
            Change::Categories(vec![3]).form(),
            [pair("categoryId[]", "3")]
        );
        assert_eq!(
            Change::Categories(Vec::new()).form(),
            [pair("categoryId[]", "")]
        );
        assert_eq!(
            Change::Versions(vec![4]).form(),
            [pair("versionId[]", "4")]
        );
        assert_eq!(
            Change::DueDate(Some("2026-10-21".into())).form(),
            [pair("dueDate", "2026-10-21")]
        );
        assert_eq!(
            Change::EstimatedHours(Some(1.5)).form(),
            [pair("estimatedHours", "1.5")]
        );
        assert_eq!(Change::ActualHours(None).form(), [pair("actualHours", "")]);
        assert_eq!(
            Change::CustomField {
                id: 10,
                values: vec!["1".into(), "2".into()],
                other_value: Some("extra".into()),
            }
            .form(),
            [
                pair("customField_10[0]", "1"),
                pair("customField_10[1]", "2"),
                pair("customField_10_otherValue", "extra"),
            ]
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
