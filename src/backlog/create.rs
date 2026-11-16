//! A new Backlog issue (`POST /issues`): a project, a subject, a type and a
//! priority (Backlog requires all four), with an optional description and
//! any of the fields `Change` sets. A new issue starts in the project's
//! first status, unresolved.

use super::fields::{Change, IssueFields, Named, fields, parse_named};
use super::{IssueRow, NORMAL_PRIORITY, call, require_config, work_item};
use crate::work_items::WorkItem;

/// What a new issue is made of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewIssue {
    pub project_id: String,
    pub summary: String,
    pub description: Option<String>,
    pub issue_type: i64,
    /// Assignee, priority, milestones, dates and hours; Backlog's Normal
    /// priority unless one is named. Status and resolution are refused.
    pub fields: Vec<Change>,
}

impl NewIssue {
    /// The form `POST /issues` takes; a field set to nothing is left out.
    fn form(&self) -> Result<Vec<(&'static str, String)>, String> {
        let summary = self.summary.trim();
        if summary.is_empty() {
            return Err("A new issue needs a subject".into());
        }
        let mut form = vec![
            ("projectId", self.project_id.clone()),
            ("summary", summary.to_string()),
            ("issueTypeId", self.issue_type.to_string()),
        ];
        if let Some(description) = (self.description.as_deref()).filter(|d| !d.trim().is_empty()) {
            form.push(("description", description.to_string()));
        }
        for change in &self.fields {
            if matches!(change, Change::Status(_) | Change::Resolution(_)) {
                return Err(
                    "A new issue starts open and unresolved; change its status or \
                    resolution once it exists"
                        .into(),
                );
            }
            form.extend(
                change
                    .form()
                    .into_iter()
                    .filter(|(_, value)| !value.is_empty()),
            );
        }
        if !form.iter().any(|(name, _)| *name == "priorityId") {
            form.push(("priorityId", NORMAL_PRIORITY.to_string()));
        }
        Ok(form)
    }
}

/// The issue types `project_id`'s issues can take, in the project's order.
pub fn issue_types(project_id: &str) -> Result<Vec<Named>, String> {
    let config = require_config()?;
    let data = call(
        &config,
        None,
        &format!("/projects/{project_id}/issueTypes"),
        &[],
        &[],
    )?;
    Ok(parse_named(&data, |_| false))
}

/// Creates the issue as the connected user; returns it as Backlog made it.
pub fn create(issue: &NewIssue) -> Result<(WorkItem, IssueFields), String> {
    let config = require_config()?;
    let form = issue.form()?;
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let data = call(&config, None, "/issues", &[], &form)?;
    let row: IssueRow =
        serde_json::from_value(data).map_err(|_| "Backlog sent an unreadable issue".to_string())?;
    Ok((work_item(&row, &config.space), fields(&row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(fields: Vec<Change>) -> NewIssue {
        NewIssue {
            project_id: "12".into(),
            summary: " Fix the login ".into(),
            description: Some("It fails.".into()),
            issue_type: 5,
            fields,
        }
    }

    #[test]
    fn the_form_names_what_backlog_requires_and_defaults_to_normal() {
        let form = issue(vec![
            Change::Assignee(None),
            Change::DueDate(Some("2026-11-20".into())),
        ])
        .form()
        .unwrap();
        assert_eq!(
            form,
            [
                ("projectId", "12".to_string()),
                ("summary", "Fix the login".into()),
                ("issueTypeId", "5".into()),
                ("description", "It fails.".into()),
                ("dueDate", "2026-11-20".into()),
                ("priorityId", NORMAL_PRIORITY.to_string()),
            ]
        );
        let high = issue(vec![Change::Priority(2), Change::Milestones(vec![8, 9])])
            .form()
            .unwrap();
        assert_eq!(
            &high[4..],
            [
                ("priorityId", "2".to_string()),
                ("milestoneId[]", "8".into()),
                ("milestoneId[]", "9".into()),
            ]
        );
    }

    #[test]
    fn a_new_issue_needs_a_subject_and_takes_no_status() {
        let mut blank = issue(Vec::new());
        blank.summary = "  ".into();
        assert!(blank.form().is_err());
        assert!(issue(vec![Change::Status(2)]).form().is_err());
        assert!(issue(vec![Change::Resolution(Some(0))]).form().is_err());
    }
}
