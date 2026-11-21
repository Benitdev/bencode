//! Space and project tools for `bencode --backlog-mcp`: space info, users,
//! activities, project listing, full project metadata (members, statuses,
//! types, categories, milestones/versions, custom fields, priorities,
//! resolutions), category creation, and milestone/project management.

use serde_json::Value;

use super::{
    DEFAULT_LIMIT, bool_arg, date, i64_arg, pick, pick_user, resolve_project, short_day,
    string_arg,
};
use crate::backlog::fields::parse_named;
use crate::backlog::{
    self, PAGE, call, call_owned, forget_catalog, parse_projects, parse_statuses, require_config,
    text,
};

// ---------------------------------------------------------------------------
// Space, users, activities
// ---------------------------------------------------------------------------

pub(super) fn get_space(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let include_users = bool_arg(args, "includeUsers").unwrap_or(false);
    let include_activities = bool_arg(args, "activities").unwrap_or(false)
        || args.get("user").is_some();

    let (space_res, myself_res, users_res) = std::thread::scope(|scope| {
        let space = scope.spawn(|| call(&config, None, "/space", &[], &[]));
        let myself = scope.spawn(|| call(&config, None, "/users/myself", &[], &[]));
        let users = scope.spawn(|| {
            if include_users || args.get("user").is_some() {
                Some(call(&config, None, "/users", &[], &[]))
            } else {
                None
            }
        });
        (
            space.join().unwrap_or_else(|_| Err("Space request failed".into())),
            myself.join().unwrap_or_else(|_| Err("User request failed".into())),
            users.join().unwrap_or(None),
        )
    });

    let space = space_res?;
    let myself = myself_res?;

    let mut out = format!(
        "# {} ({})\nHost: {}\nFormatting: {}\nTimezone: {}\nLanguage: {}\n\n## Connected user\n{} (id: {}, role: {}, email: {})",
        text(&space, "name"),
        text(&space, "spaceKey"),
        config.space,
        text(&space, "textFormattingRule"),
        text(&space, "timezone"),
        text(&space, "lang"),
        text(&myself, "name"),
        myself.get("id").and_then(Value::as_i64).unwrap_or(config.user_id),
        role_name(myself.get("roleType").and_then(Value::as_i64).unwrap_or(0)),
        text(&myself, "mailAddress"),
    );

    let users_data = match users_res {
        Some(res) => Some(res?),
        None => None,
    };

    if include_users
        && let Some(users) = users_data.as_ref().and_then(Value::as_array)
    {
        out.push_str(&format!("\n\n## Users ({})\n", users.len()));
        for u in users {
            let id = u.get("id").and_then(Value::as_i64).unwrap_or(0);
            let name = text(u, "name");
            let role = role_name(u.get("roleType").and_then(Value::as_i64).unwrap_or(0));
            let mail = text(u, "mailAddress");
            out.push_str(&format!("\n- {name} (id: {id}, {role}, {mail})"));
        }
    }

    if include_activities {
        let mut query = Vec::new();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_LIMIT, |n| n as usize)
            .clamp(1, PAGE);
        query.push(("count", limit.to_string()));
        if let Some(order) = string_arg(args, "order") {
            query.push(("order", order));
        }
        if let Some(min_id) = i64_arg(args, "minId") {
            query.push(("minId", min_id.to_string()));
        }
        if let Some(max_id) = i64_arg(args, "maxId") {
            query.push(("maxId", max_id.to_string()));
        }
        let path = if let Some(wanted) = args.get("user").filter(|v| !v.is_null()) {
            let choices = users_data
                .as_ref()
                .map(|d| parse_named(d, |_| false))
                .unwrap_or_default();
            let uid = pick_user(&choices, config.user_id, wanted)?;
            format!("/users/{uid}/activities")
        } else {
            "/space/activities".to_string()
        };
        let activities = call(&config, None, &path, &query, &[])?;
        let rows = activities.as_array().map(Vec::as_slice).unwrap_or_default();
        out.push_str(&format!("\n\n## Recent activities ({})\n", rows.len()));
        if rows.is_empty() {
            out.push_str("\n(none)");
        }
        for act in rows {
            out.push('\n');
            out.push_str(&format_activity(act));
        }
    }

    Ok(out)
}

fn role_name(role_type: i64) -> &'static str {
    match role_type {
        1 => "Admin",
        2 => "Normal",
        3 => "Reporter",
        4 => "Viewer",
        5 => "Guest Reporter",
        6 => "Guest Viewer",
        _ => "Member",
    }
}

fn format_activity(act: &Value) -> String {
    let id = act.get("id").and_then(Value::as_i64).unwrap_or(0);
    let kind = activity_label(act.get("type").and_then(Value::as_i64).unwrap_or(0));
    let project = act
        .get("project")
        .map(|p| text(p, "projectKey"))
        .unwrap_or_default();
    let actor = act
        .get("createdUser")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let created = short_day(&text(act, "created")).to_string();
    let content = act.get("content").unwrap_or(&Value::Null);
    let subject = if let Some(key_id) = content.get("key_id").and_then(Value::as_i64) {
        let summary = text(content, "summary");
        if summary.is_empty() {
            format!("{project}-{key_id}")
        } else {
            format!("{project}-{key_id} {summary}")
        }
    } else if !text(content, "name").is_empty() {
        text(content, "name")
    } else if !text(content, "summary").is_empty() {
        text(content, "summary")
    } else if !text(content, "title").is_empty() {
        text(content, "title")
    } else {
        project.clone()
    };
    format!("- #{id} [{created}] {actor} — {kind}: {subject}")
}

fn activity_label(code: i64) -> &'static str {
    match code {
        1 => "Issue created",
        2 => "Issue updated",
        3 => "Issue commented",
        4 => "Issue deleted",
        5 => "Wiki created",
        6 => "Wiki updated",
        7 => "Wiki deleted",
        8 => "File added",
        9 => "File updated",
        10 => "File deleted",
        11 => "SVN committed",
        12 => "Git pushed",
        13 => "Git repository created",
        14 => "Issues bulk updated",
        15 => "Project member added",
        16 => "Project member removed",
        17 => "Comment notification added",
        18 => "Pull request added",
        19 => "Pull request updated",
        20 => "Pull request commented",
        21 => "Pull request merged",
        22 => "Milestone created",
        23 => "Milestone updated",
        24 => "Milestone deleted",
        _ => "Activity",
    }
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

pub(super) fn list_projects(args: &Value) -> Result<String, String> {
    let include_archived = bool_arg(args, "includeArchived").unwrap_or(false);
    if !include_archived {
        let projects = backlog::projects_with_statuses()?;
        if projects.is_empty() {
            return Ok("The space has no active projects.".into());
        }
        return Ok(projects
            .iter()
            .map(|(project, statuses)| {
                let statuses: Vec<&str> = statuses.iter().map(|s| s.name.as_str()).collect();
                format!(
                    "- {} (id: {}) — {} (statuses: {})",
                    project.key,
                    project.id,
                    project.name,
                    statuses.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"));
    }

    let config = require_config()?;
    let data = call(&config, None, "/projects", &[], &[])?;
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return Ok("The space has no projects.".into());
    }
    let projects = parse_projects(&data);
    let archived_map: std::collections::HashMap<String, bool> = rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_i64()?.to_string();
            let archived = row.get("archived").and_then(Value::as_bool).unwrap_or(false);
            Some((id, archived))
        })
        .collect();
    Ok(projects
        .iter()
        .map(|project| {
            let tag = if archived_map.get(&project.id).copied().unwrap_or(false) {
                " [archived]"
            } else {
                ""
            };
            format!("- {} (id: {}) — {}{tag}", project.key, project.id, project.name)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

pub(super) fn get_project(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key (like PROJ) or id")?;
    let include_archived = bool_arg(args, "includeArchivedMilestones").unwrap_or(false);
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let pid = project.id.as_str();

    let get = |path: String| call(&config, None, &path, &[], &[]);
    let (proj, statuses, types, users, categories, versions, custom_fields, priorities, resolutions) =
        std::thread::scope(|scope| {
            let proj = scope.spawn(|| get(format!("/projects/{pid}")));
            let statuses = scope.spawn(|| get(format!("/projects/{pid}/statuses")));
            let types = scope.spawn(|| get(format!("/projects/{pid}/issueTypes")));
            let users = scope.spawn(|| get(format!("/projects/{pid}/users")));
            let categories = scope.spawn(|| get(format!("/projects/{pid}/categories")));
            let versions = scope.spawn(|| get(format!("/projects/{pid}/versions")));
            let custom_fields = scope.spawn(|| get(format!("/projects/{pid}/customFields")));
            let priorities = scope.spawn(|| get("/priorities".into()));
            let resolutions = scope.spawn(|| get("/resolutions".into()));
            let join = |h: std::thread::ScopedJoinHandle<'_, Result<Value, String>>| {
                h.join().unwrap_or_else(|_| Err("Backlog request failed".into()))
            };
            (
                join(proj),
                join(statuses),
                join(types),
                join(users),
                join(categories),
                join(versions),
                join(custom_fields),
                join(priorities),
                join(resolutions),
            )
        });

    let proj = proj?;
    let mut out = format!(
        "# {} — {} (id: {})\nFormatting: {} · Subtasking: {} · Chart: {} · Wiki: {} · Git: {} · Archived: {}",
        text(&proj, "projectKey"),
        text(&proj, "name"),
        project.id,
        text(&proj, "textFormattingRule"),
        yes_no(&proj, "subtaskingEnabled"),
        yes_no(&proj, "chartEnabled"),
        yes_no(&proj, "useWiki"),
        yes_no(&proj, "useGit"),
        yes_no(&proj, "archived"),
    );

    if let Ok(data) = statuses {
        let items: Vec<String> = parse_statuses(&data)
            .into_iter()
            .map(|s| format!("{} ({})", s.name, s.id))
            .collect();
        out.push_str(&format!("\n\n## Statuses\n{}", join_or_none(&items)));
    }
    if let Ok(data) = types {
        let items: Vec<String> = parse_named(&data, |_| false)
            .into_iter()
            .map(|t| format!("{} ({})", t.name, t.id))
            .collect();
        out.push_str(&format!("\n\n## Issue types\n{}", join_or_none(&items)));
    }
    if let Ok(data) = users {
        let items: Vec<String> = parse_named(&data, |_| false)
            .into_iter()
            .map(|u| format!("{} ({})", u.name, u.id))
            .collect();
        out.push_str(&format!("\n\n## Members\n{}", join_or_none(&items)));
    }
    if let Ok(data) = categories {
        let items: Vec<String> = parse_named(&data, |_| false)
            .into_iter()
            .map(|c| format!("{} ({})", c.name, c.id))
            .collect();
        out.push_str(&format!("\n\n## Categories\n{}", join_or_none(&items)));
    }
    if let Ok(data) = versions {
        let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
        let items: Vec<String> = rows
            .iter()
            .filter(|row| {
                include_archived
                    || row.get("archived").and_then(Value::as_bool) != Some(true)
            })
            .filter_map(|row| {
                let id = row.get("id")?.as_i64()?;
                let name = text(row, "name");
                let mut line = format!("- {name} (id: {id})");
                if let Some(start) = row.get("startDate").and_then(Value::as_str) {
                    line.push_str(&format!(" · start {}", short_day(start)));
                }
                if let Some(due) = row.get("releaseDueDate").and_then(Value::as_str) {
                    line.push_str(&format!(" · release {}", short_day(due)));
                }
                if row.get("archived").and_then(Value::as_bool) == Some(true) {
                    line.push_str(" [archived]");
                }
                Some(line)
            })
            .collect();
        out.push_str("\n\n## Milestones / Versions\n");
        out.push_str(&if items.is_empty() {
            "(none)".to_string()
        } else {
            items.join("\n")
        });
    }
    if let Ok(data) = custom_fields {
        let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
        if !rows.is_empty() {
            out.push_str("\n\n## Custom fields\n");
            for cf in rows {
                let id = cf.get("id").and_then(Value::as_i64).unwrap_or(0);
                let name = text(cf, "name");
                let kind = custom_field_type(cf.get("typeId").and_then(Value::as_i64).unwrap_or(0));
                let req = if cf.get("required").and_then(Value::as_bool) == Some(true) {
                    ", required"
                } else {
                    ""
                };
                let mut line = format!("\n- {name} (id: {id}, {kind}{req})");
                if let Some(items) = cf.get("items").and_then(Value::as_array)
                    && !items.is_empty()
                {
                    let opts: Vec<String> = items
                        .iter()
                        .filter_map(|i| {
                            Some(format!("{} ({})", text(i, "name"), i.get("id")?.as_i64()?))
                        })
                        .collect();
                    line.push_str(&format!(" — options: {}", opts.join(", ")));
                }
                out.push_str(&line);
            }
        }
    }
    if let Ok(data) = priorities {
        let items: Vec<String> = parse_named(&data, |_| false)
            .into_iter()
            .map(|p| format!("{} ({})", p.name, p.id))
            .collect();
        out.push_str(&format!("\n\n## Priorities\n{}", join_or_none(&items)));
    }
    if let Ok(data) = resolutions {
        let items: Vec<String> = parse_named(&data, |_| false)
            .into_iter()
            .map(|r| format!("{} ({})", r.name, r.id))
            .collect();
        out.push_str(&format!("\n\n## Resolutions\n{}", join_or_none(&items)));
    }

    Ok(out)
}

fn yes_no(val: &Value, key: &str) -> &'static str {
    if val.get(key).and_then(Value::as_bool) == Some(true) {
        "yes"
    } else {
        "no"
    }
}

fn join_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".into()
    } else {
        items.join(", ")
    }
}

fn custom_field_type(type_id: i64) -> &'static str {
    match type_id {
        1 => "text",
        2 => "textarea",
        3 => "numeric",
        4 => "date",
        5 => "single-list",
        6 => "multi-list",
        7 => "checkbox",
        8 => "radio",
        _ => "custom",
    }
}

// ---------------------------------------------------------------------------
// Categories, milestones/versions, project management
// ---------------------------------------------------------------------------

pub(super) fn create_category(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let name = string_arg(args, "name").ok_or("Give the category a name")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let data = call(
        &config,
        Some("POST"),
        &format!("/projects/{}/categories", project.id),
        &[],
        &[("name", &name)],
    )?;
    let id = data.get("id").and_then(Value::as_i64).unwrap_or(0);
    Ok(format!(
        "Created category {} (id: {id}) in {}.",
        text(&data, "name"),
        project.key
    ))
}

pub(super) fn manage_milestone(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let action = string_arg(args, "action")
        .ok_or("Specify action: create, update or delete")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;

    match action.to_ascii_lowercase().as_str() {
        "create" => {
            let name = string_arg(args, "name").ok_or("Give the milestone a name")?;
            let mut form = vec![("name".to_string(), name)];
            if let Some(desc) = string_arg(args, "description") {
                form.push(("description".into(), desc));
            }
            if let Some(val) = args.get("startDate")
                && let Some(d) = date(val, "startDate")?
            {
                form.push(("startDate".into(), d));
            }
            if let Some(val) = args.get("releaseDueDate")
                && let Some(d) = date(val, "releaseDueDate")?
            {
                form.push(("releaseDueDate".into(), d));
            }
            let data = call_owned(
                &config,
                Some("POST"),
                &format!("/projects/{}/versions", project.id),
                &[],
                &form,
            )?;
            let id = data.get("id").and_then(Value::as_i64).unwrap_or(0);
            Ok(format!(
                "Created milestone {} (id: {id}) in {}.",
                text(&data, "name"),
                project.key
            ))
        }
        "update" | "delete" => {
            let target = args
                .get("milestone")
                .filter(|v| !v.is_null())
                .ok_or("Specify milestone: its name or numeric id")?;
            let list = call(
                &config,
                None,
                &format!("/projects/{}/versions", project.id),
                &[],
                &[],
            )?;
            let choices = parse_named(&list, |_| false);
            let id = pick(&choices, target, "milestone")?;
            let existing_name = choices
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            if action.eq_ignore_ascii_case("delete") {
                let _ = call(
                    &config,
                    Some("DELETE"),
                    &format!("/projects/{}/versions/{id}", project.id),
                    &[],
                    &[],
                )?;
                return Ok(format!(
                    "Deleted milestone {existing_name} (id: {id}) from {}.",
                    project.key
                ));
            }
            // Backlog requires `name` on PATCH /versions/:id; preserve current name if omitted.
            let name = string_arg(args, "name").unwrap_or(existing_name);
            let mut form = vec![("name".to_string(), name)];
            if let Some(desc) = args.get("description") {
                form.push((
                    "description".into(),
                    desc.as_str().unwrap_or_default().to_string(),
                ));
            }
            if let Some(val) = args.get("startDate") {
                form.push(("startDate".into(), date(val, "startDate")?.unwrap_or_default()));
            }
            if let Some(val) = args.get("releaseDueDate") {
                form.push((
                    "releaseDueDate".into(),
                    date(val, "releaseDueDate")?.unwrap_or_default(),
                ));
            }
            if let Some(archived) = bool_arg(args, "archived") {
                form.push(("archived".into(), archived.to_string()));
            }
            let data = call_owned(
                &config,
                Some("PATCH"),
                &format!("/projects/{}/versions/{id}", project.id),
                &[],
                &form,
            )?;
            Ok(format!(
                "Updated milestone {} (id: {id}) in {}.",
                text(&data, "name"),
                project.key
            ))
        }
        other => Err(format!("Unknown action {other}; use create, update or delete")),
    }
}

pub(super) fn manage_project(args: &Value) -> Result<String, String> {
    let action = string_arg(args, "action").ok_or("Specify action: create or update")?;
    let config = require_config()?;
    let mut form: Vec<(String, String)> = Vec::new();
    let push_bool = |form: &mut Vec<(String, String)>, field: &str| {
        if let Some(v) = bool_arg(args, field) {
            form.push((field.to_string(), v.to_string()));
        }
    };

    match action.to_ascii_lowercase().as_str() {
        "create" => {
            let name = string_arg(args, "name").ok_or("Give the new project a name")?;
            let key = string_arg(args, "key").ok_or("Give the new project a key")?;
            form.push(("name".into(), name));
            form.push(("key".into(), key));
            form.push((
                "chartEnabled".into(),
                bool_arg(args, "chartEnabled").unwrap_or(false).to_string(),
            ));
            form.push((
                "subtaskingEnabled".into(),
                bool_arg(args, "subtaskingEnabled")
                    .unwrap_or(true)
                    .to_string(),
            ));
            form.push((
                "textFormattingRule".into(),
                string_arg(args, "textFormattingRule").unwrap_or_else(|| "markdown".into()),
            ));
            push_bool(&mut form, "grandchildIssueEnabled");
            push_bool(&mut form, "projectLeaderCanEditProjectLeader");
            let data = call_owned(&config, Some("POST"), "/projects", &[], &form)?;
            forget_catalog();
            Ok(format!(
                "Created project {} (id: {}) — {}.",
                text(&data, "projectKey"),
                data.get("id").and_then(Value::as_i64).unwrap_or(0),
                text(&data, "name"),
            ))
        }
        "update" => {
            let raw = string_arg(args, "project")
                .ok_or("Specify project: the existing project's key or id")?;
            let project = resolve_project(&raw)?;
            if let Some(name) = string_arg(args, "name") {
                form.push(("name".into(), name));
            }
            if let Some(key) = string_arg(args, "key") {
                form.push(("key".into(), key));
            }
            if let Some(rule) = string_arg(args, "textFormattingRule") {
                form.push(("textFormattingRule".into(), rule));
            }
            push_bool(&mut form, "chartEnabled");
            push_bool(&mut form, "subtaskingEnabled");
            push_bool(&mut form, "grandchildIssueEnabled");
            push_bool(&mut form, "projectLeaderCanEditProjectLeader");
            push_bool(&mut form, "archived");
            if form.is_empty() {
                return Err("Name at least one project field to update".into());
            }
            let data = call_owned(
                &config,
                Some("PATCH"),
                &format!("/projects/{}", project.id),
                &[],
                &form,
            )?;
            forget_catalog();
            Ok(format!(
                "Updated project {} (id: {}) — {}.",
                text(&data, "projectKey"),
                data.get("id").and_then(Value::as_i64).unwrap_or(0),
                text(&data, "name"),
            ))
        }
        other => Err(format!("Unknown action {other}; use create or update")),
    }
}
