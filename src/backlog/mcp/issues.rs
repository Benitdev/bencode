//! Issue tools for `bencode --backlog-mcp`: rich search & count, full issue
//! details (custom fields, attachments, related issues, paginated comments),
//! create, update, delete, comments, attachment download, and related issues.

use base64::Engine as _;
use serde_json::Value;

use super::{
    DEFAULT_LIMIT, IssueContext, bool_arg, clears, date, format_bytes, hours, i64_arg, key_arg,
    pick, pick_many, pick_many_users, pick_user, project_categories, project_custom_fields,
    project_users, resolve_issue_id, resolve_project, short_day, space_users, status_choices,
    string_arg,
};
use crate::backlog::fields::{Change, IssueFields, Named as FieldChoice, parse_named};
use crate::backlog::{
    self, CLOSED_STATUS, Config, NewIssue, PAGE, call, call_owned, catalog, images,
    issue_types, options, parse_issues, require_config, require_issue_key, text,
};
use crate::mcp::server::Reply;
use crate::work_items::{Details, Thread, WorkItem};

/// Max raster image size to inline as base64 in MCP image content (4 MiB).
const MAX_INLINE_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Search & count
// ---------------------------------------------------------------------------

pub(super) fn search_issues(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let cat = catalog(&config)?;
    let mut query: Vec<(String, String)> = Vec::new();

    // Project scoping
    let scoped_project = match string_arg(args, "project") {
        Some(key) => Some(resolve_project(&key)?),
        None => None,
    };
    if let Some(project) = &scoped_project {
        query.push(("projectId[]".into(), project.id.clone()));
    } else {
        for project in &cat.projects {
            query.push(("projectId[]".into(), project.id.clone()));
        }
    }

    // Statuses: explicit list overrides the default open-only filter.
    let include_closed = bool_arg(args, "includeClosed").unwrap_or(false);
    if let Some(wanted) = args.get("statuses").filter(|v| !v.is_null()) {
        let choices: Vec<FieldChoice> = match &scoped_project {
            Some(project) => match cat.statuses.get(&project.id) {
                Some(list) => status_choices(list),
                None => {
                    let data = call(
                        &config,
                        None,
                        &format!("/projects/{}/statuses", project.id),
                        &[],
                        &[],
                    )?;
                    parse_named(&data, |_| false)
                }
            },
            None => {
                let mut all: Vec<FieldChoice> = cat
                    .statuses
                    .values()
                    .flatten()
                    .map(|s| FieldChoice {
                        id: s.id,
                        name: s.name.clone(),
                    })
                    .collect();
                all.sort_by_key(|c| c.id);
                all.dedup_by_key(|c| c.id);
                all
            }
        };
        for id in pick_many(&choices, wanted, "status")? {
            query.push(("statusId[]".into(), id.to_string()));
        }
    } else if !include_closed {
        let mut open: Vec<i64> = match &scoped_project {
            Some(project) => cat
                .statuses
                .get(&project.id)
                .into_iter()
                .flatten()
                .map(|s| s.id)
                .filter(|id| *id != CLOSED_STATUS)
                .collect(),
            None => cat
                .statuses
                .values()
                .flatten()
                .map(|s| s.id)
                .filter(|id| *id != CLOSED_STATUS)
                .collect(),
        };
        open.sort_unstable();
        open.dedup();
        for id in open {
            query.push(("statusId[]".into(), id.to_string()));
        }
    }

    // Users (assignee / assignedToMe / createdBy)
    let needs_users = args.get("assignee").is_some() || args.get("createdBy").is_some();
    let users = if needs_users {
        match &scoped_project {
            Some(project) => project_users(&config, &project.id)?,
            None => space_users(&config)?,
        }
    } else {
        Vec::new()
    };
    if let Some(wanted) = args.get("assignee").filter(|v| !v.is_null()) {
        for id in pick_many_users(&users, config.user_id, wanted)? {
            query.push(("assigneeId[]".into(), id.to_string()));
        }
    } else if bool_arg(args, "assignedToMe").unwrap_or(false) {
        query.push(("assigneeId[]".into(), config.user_id.to_string()));
    }
    if let Some(wanted) = args.get("createdBy").filter(|v| !v.is_null()) {
        for id in pick_many_users(&users, config.user_id, wanted)? {
            query.push(("createdUserId[]".into(), id.to_string()));
        }
    }

    // Project-scoped name resolution (issueTypes, categories, milestones, versions)
    if let Some(wanted) = args.get("issueTypes").filter(|v| !v.is_null()) {
        let ids = resolve_scoped_ids(
            wanted,
            "issue type",
            scoped_project.as_ref(),
            |pid| issue_types(pid),
        )?;
        for id in ids {
            query.push(("issueTypeId[]".into(), id.to_string()));
        }
    }
    if let Some(wanted) = args.get("categories").filter(|v| !v.is_null()) {
        let ids = resolve_scoped_ids(wanted, "category", scoped_project.as_ref(), |pid| {
            project_categories(&config, pid)
        })?;
        for id in ids {
            query.push(("categoryId[]".into(), id.to_string()));
        }
    }
    let needs_versions = args.get("milestones").is_some() || args.get("versions").is_some();
    let versions_choices = if needs_versions {
        match &scoped_project {
            Some(project) => {
                let data = call(
                    &config,
                    None,
                    &format!("/projects/{}/versions", project.id),
                    &[],
                    &[],
                )?;
                Some(parse_named(&data, |_| false))
            }
            None => None,
        }
    } else {
        None
    };
    if let Some(wanted) = args.get("milestones").filter(|v| !v.is_null()) {
        let ids = pick_or_numeric(wanted, versions_choices.as_deref(), "milestone")?;
        for id in ids {
            query.push(("milestoneId[]".into(), id.to_string()));
        }
    }
    if let Some(wanted) = args.get("versions").filter(|v| !v.is_null()) {
        let ids = pick_or_numeric(wanted, versions_choices.as_deref(), "version")?;
        for id in ids {
            query.push(("versionId[]".into(), id.to_string()));
        }
    }

    // Space-wide choices (priorities, resolutions)
    if let Some(wanted) = args.get("priorities").filter(|v| !v.is_null()) {
        let data = call(&config, None, "/priorities", &[], &[])?;
        let choices = parse_named(&data, |_| false);
        for id in pick_many(&choices, wanted, "priority")? {
            query.push(("priorityId[]".into(), id.to_string()));
        }
    }
    if let Some(wanted) = args.get("resolutions").filter(|v| !v.is_null()) {
        let data = call(&config, None, "/resolutions", &[], &[])?;
        let choices = parse_named(&data, |_| false);
        for id in pick_many(&choices, wanted, "resolution")? {
            query.push(("resolutionId[]".into(), id.to_string()));
        }
    }

    // Parent / child filters
    if let Some(parent) = args.get("parentIssue").filter(|v| !v.is_null()) {
        let id = resolve_issue_id(&config, parent)?;
        query.push(("parentIssueId[]".into(), id.to_string()));
    }
    if let Some(mode) = string_arg(args, "parentChild") {
        let code = match mode.to_ascii_lowercase().as_str() {
            "all" => "0",
            "not_child" => "1",
            "child" => "2",
            "standalone" => "3",
            "parent" => "4",
            other => {
                return Err(format!(
                    "Unknown parentChild {other}; choose one of: all, not_child, child, standalone, parent"
                ));
            }
        };
        query.push(("parentChild".into(), code.into()));
    }

    // Date range filters
    for field in [
        "dueDateSince",
        "dueDateUntil",
        "startDateSince",
        "startDateUntil",
        "createdSince",
        "createdUntil",
        "updatedSince",
        "updatedUntil",
    ] {
        if let Some(val) = args.get(field).filter(|v| !v.is_null())
            && let Some(day) = date(val, field)?
        {
            query.push((field.into(), day));
        }
    }

    // Keyword
    if let Some(keyword) = string_arg(args, "keyword") {
        query.push(("keyword".into(), keyword));
    }

    // Custom field filters
    if let Some(filters) = args.get("customFields").and_then(Value::as_array) {
        append_custom_field_filters(&mut query, filters)?;
    }

    if bool_arg(args, "countOnly").unwrap_or(false) {
        let data = call_owned(&config, None, "/issues/count", &query, &[])?;
        let count = data.get("count").and_then(Value::as_i64).unwrap_or(0);
        return Ok(format!("Matching issues: {count}"));
    }

    // Sort, order, pagination
    let sort = string_arg(args, "sort").unwrap_or_else(|| "updated".into());
    let order = string_arg(args, "order").unwrap_or_else(|| "desc".into());
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n as usize)
        .clamp(1, PAGE);
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    query.push(("sort".into(), sort));
    query.push(("order".into(), order));
    query.push(("count".into(), limit.to_string()));
    if offset > 0 {
        query.push(("offset".into(), offset.to_string()));
    }

    let data = call_owned(&config, None, "/issues", &query, &[])?;
    let items = parse_issues(data, &config.space)?;
    if items.is_empty() {
        return Ok("No issues match.".into());
    }
    let mut lines: Vec<String> = items.iter().map(issue_line).collect();
    if items.len() >= limit {
        lines.push(format!(
            "\n(Showing {} issues from offset {offset}; pass offset: {} for more, or countOnly: true for the total.)",
            items.len(),
            offset + items.len() as u64
        ));
    }
    Ok(lines.join("\n"))
}

fn resolve_scoped_ids(
    wanted: &Value,
    label: &str,
    project: Option<&backlog::Project>,
    fetch: impl FnOnce(&str) -> Result<Vec<FieldChoice>, String>,
) -> Result<Vec<i64>, String> {
    if let Some(project) = project {
        let choices = fetch(&project.id)?;
        pick_many(&choices, wanted, label)
    } else {
        pick_or_numeric(wanted, None, label)
    }
}

fn pick_or_numeric(
    wanted: &Value,
    choices: Option<&[FieldChoice]>,
    label: &str,
) -> Result<Vec<i64>, String> {
    if let Some(choices) = choices {
        return pick_many(choices, wanted, label);
    }
    let items: Vec<&Value> = match wanted {
        Value::Array(arr) => arr.iter().collect(),
        single => vec![single],
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| format!("Invalid {label} id: {item}")),
            Value::String(s) => s.trim().parse::<i64>().map_err(|_| {
                format!("Filtering by {label} name ({s}) requires specifying project")
            }),
            _ => Err(format!("Invalid {label}: {item}")),
        })
        .collect()
}

pub(super) fn append_custom_field_filters(
    query: &mut Vec<(String, String)>,
    filters: &[Value],
) -> Result<(), String> {
    for filter in filters {
        let id = i64_arg(filter, "id")
            .ok_or("Each customFields filter needs a numeric id")?;
        let kind = string_arg(filter, "type")
            .ok_or("Each customFields filter needs type: text, numeric, date or list")?;
        let base = format!("customField_{id}");
        match kind.to_ascii_lowercase().as_str() {
            "text" => {
                if let Some(val) = string_arg(filter, "value") {
                    query.push((base, val));
                }
            }
            "numeric" | "date" => {
                if let Some(min) = filter.get("min").filter(|v| !v.is_null()) {
                    query.push((format!("{base}_min"), scalar_to_string(min)));
                }
                if let Some(max) = filter.get("max").filter(|v| !v.is_null()) {
                    query.push((format!("{base}_max"), scalar_to_string(max)));
                }
            }
            "list" => {
                // Backlog requires explicit indices (`customField_1[0]=…`) for list filters.
                let vals: Vec<String> = match filter.get("value") {
                    Some(Value::Array(arr)) => arr.iter().map(scalar_to_string).collect(),
                    Some(v) if !v.is_null() => vec![scalar_to_string(v)],
                    _ => Vec::new(),
                };
                for (idx, val) in vals.into_iter().enumerate() {
                    query.push((format!("{base}[{idx}]"), val));
                }
            }
            other => {
                return Err(format!(
                    "Unknown customFields filter type {other}; expected text, numeric, date or list"
                ));
            }
        }
    }
    Ok(())
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Get issue
// ---------------------------------------------------------------------------

pub(super) fn get_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let (item, details) = backlog::read_issue(&key)?;
    let thread = match bool_arg(args, "comments") {
        Some(true) => {
            let count = args
                .get("commentCount")
                .and_then(Value::as_u64)
                .map_or(PAGE, |n| n as usize);
            let order = string_arg(args, "commentOrder").unwrap_or_else(|| "desc".into());
            let min_id = i64_arg(args, "commentMinId");
            let max_id = i64_arg(args, "commentMaxId");
            Some(backlog::thread_with_options(
                &key, count, &order, min_id, max_id,
            )?)
        }
        _ => None,
    };
    let related = match bool_arg(args, "relatedIssues") {
        Some(true) => {
            let config = require_config()?;
            let checked = require_issue_key(&key)?;
            let data = call(
                &config,
                None,
                &format!("/issues/{checked}/relatedIssues"),
                &[],
                &[],
            )?;
            Some(parse_issues(data, &config.space)?)
        }
        _ => None,
    };
    Ok(issue_text(&item, &details, thread.as_ref(), related.as_deref()))
}

// ---------------------------------------------------------------------------
// Create, update, delete
// ---------------------------------------------------------------------------

pub(super) fn create_issue(args: &Value) -> Result<String, String> {
    let project_key = string_arg(args, "project")
        .ok_or("Name the project: its key (like PROJ) or numeric id")?;
    let summary = string_arg(args, "summary").ok_or("Give the issue a summary")?;
    if let Some(field) = ["status", "resolution"]
        .into_iter()
        .find(|f| args.get(*f).is_some())
    {
        return Err(format!(
            "A new issue starts in the project's first status; set its {field} with \
            backlog_update_issue once it exists"
        ));
    }
    let config = require_config()?;
    let project = resolve_project(&project_key)?;
    let types = issue_types(&project.id)?;
    let issue_type = match args.get("issueType") {
        Some(wanted) if !clears(wanted) => pick(&types, wanted, "issue type")?,
        _ => types
            .first()
            .map(|kind| kind.id)
            .ok_or_else(|| format!("Project {} has no issue types", project.key))?,
    };
    let ctx = load_issue_context(&config, &project.id, args, false)?;
    // Summary, description and issueType are set directly on `NewIssue`, not as Changes.
    let mut change_args = args.clone();
    if let Some(obj) = change_args.as_object_mut() {
        obj.remove("summary");
        obj.remove("description");
        obj.remove("issueType");
    }
    let issue = NewIssue {
        project_id: project.id,
        summary,
        description: string_arg(args, "description"),
        issue_type,
        fields: build_changes(&change_args, &ctx, Some(&config))?,
    };
    let (item, fields) = backlog::create(&issue)?;
    Ok(format!(
        "Created {}: {}\n{}",
        item.identifier,
        item.url,
        field_lines(&item, &fields)
    ))
}

pub(super) fn update_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let config = require_config()?;
    let item = backlog::issue(&key, &config.space)?;
    let ctx = load_issue_context(&config, &item.container_id, args, true)?;
    let changes = build_changes(args, &ctx, Some(&config))?;
    if changes.is_empty() {
        return Err("Name at least one field to change".into());
    }
    let (item, fields) = backlog::update_all(&key, &changes)?;
    Ok(format!(
        "Updated {}.\n{}",
        item.identifier,
        field_lines(&item, &fields)
    ))
}

pub(super) fn delete_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let summary = backlog::delete_issue(&key)?;
    if summary.is_empty() {
        Ok(format!("Deleted {key}."))
    } else {
        Ok(format!("Deleted {key}: {summary}"))
    }
}

fn load_issue_context(
    config: &Config,
    project_id: &str,
    args: &Value,
    for_update: bool,
) -> Result<IssueContext, String> {
    let needs_options = [
        "assignee",
        "priority",
        "resolution",
        "milestones",
        "versions",
        "notifiedUsers",
    ]
    .iter()
    .any(|field| args.get(*field).is_some());
    let needs_status = for_update && args.get("status").is_some();
    let needs_type = for_update && args.get("issueType").is_some();
    let needs_categories = args.get("categories").is_some();
    let needs_custom_fields = args.get("customFields").is_some();

    let mut ctx = IssueContext::default();
    if needs_options {
        ctx.options = options(project_id)?;
    }
    if needs_status {
        ctx.statuses = backlog::statuses(project_id)?;
    }
    if needs_type {
        ctx.issue_types = issue_types(project_id)?;
    }
    if needs_categories {
        ctx.categories = project_categories(config, project_id)?;
    }
    if needs_custom_fields {
        ctx.custom_fields = project_custom_fields(config, project_id)?;
    }
    Ok(ctx)
}

// ---------------------------------------------------------------------------
// Comments, attachments, related issues
// ---------------------------------------------------------------------------

pub(super) fn add_comment(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let content = args["content"].as_str().unwrap_or_default();
    let notified = match args.get("notifiedUsers").filter(|v| !v.is_null()) {
        Some(wanted) => {
            let config = require_config()?;
            let item = backlog::issue(&key, &config.space)?;
            let users = project_users(&config, &item.container_id)?;
            pick_many_users(&users, config.user_id, wanted)?
        }
        None => Vec::new(),
    };
    let url = backlog::post_comment_with_notifications(&key, content, &notified)?;
    Ok(format!("Commented on {key}: {url}"))
}

pub(super) fn update_comment(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let comment_id = i64_arg(args, "commentId")
        .ok_or("Give the numeric commentId to edit (from backlog_get_issue with comments: true)")?;
    let content = args["content"].as_str().unwrap_or_default();
    let url = backlog::update_comment(&key, comment_id, content)?;
    Ok(format!("Updated comment #{comment_id} on {key}: {url}"))
}

pub(super) fn get_attachment(args: &Value) -> Reply {
    let key = match key_arg(args) {
        Ok(k) => k,
        Err(err) => return Reply::error(err),
    };
    let Some(attachment_id) = i64_arg(args, "attachmentId") else {
        return Reply::error("Give the numeric attachmentId (from backlog_get_issue)");
    };
    let downloaded = match images::download_attachment(&key, attachment_id) {
        Ok(d) => d,
        Err(err) => return Reply::error(err),
    };
    let summary = format!(
        "Downloaded attachment #{} of {}: {}\nPath: {}\nSize: {}{}",
        downloaded.id,
        key,
        downloaded.name,
        downloaded.path.display(),
        format_bytes(downloaded.size),
        downloaded
            .image_mime
            .map(|mime| format!("\nMIME: {mime}"))
            .unwrap_or_default()
    );
    if let Some(mime) = downloaded.image_mime
        && downloaded.size <= MAX_INLINE_IMAGE_BYTES
        && let Ok(bytes) = std::fs::read(&downloaded.path)
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Reply::text_with_image(summary, encoded, mime);
    }
    Reply::text(summary)
}

pub(super) fn manage_related_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let action = string_arg(args, "action").ok_or("Specify action: add or remove")?;
    let target = args
        .get("target")
        .filter(|v| !v.is_null())
        .ok_or("Specify target: the related issue's key, URL or numeric id")?;
    let config = require_config()?;
    let checked = require_issue_key(&key)?;
    let target_id = resolve_issue_id(&config, target)?;
    match action.to_ascii_lowercase().as_str() {
        "add" => {
            let _ = call(
                &config,
                Some("POST"),
                &format!("/issues/{checked}/relatedIssues"),
                &[],
                &[("targetIssueId", &target_id.to_string())],
            )?;
            Ok(format!("Linked {checked} with issue #{target_id}."))
        }
        "remove" => {
            let _ = call(
                &config,
                Some("DELETE"),
                &format!("/issues/{checked}/relatedIssues/{target_id}"),
                &[],
                &[],
            )?;
            Ok(format!("Unlinked {checked} from issue #{target_id}."))
        }
        other => Err(format!("Unknown action {other}; use add or remove")),
    }
}

// ---------------------------------------------------------------------------
// Change building (pure, tested in mod.rs)
// ---------------------------------------------------------------------------

pub(super) fn build_changes(
    args: &Value,
    ctx: &IssueContext,
    config: Option<&Config>,
) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    if let Some(summary) = args.get("summary") {
        let text = summary.as_str().unwrap_or_default().trim();
        if text.is_empty() {
            return Err("summary cannot be empty".into());
        }
        changes.push(Change::Summary(text.to_string()));
    }
    if let Some(description) = args.get("description") {
        let text = if clears(description) {
            None
        } else {
            Some(description.as_str().unwrap_or_default().to_string())
        };
        changes.push(Change::Description(text));
    }
    if let Some(issue_type) = args.get("issueType") {
        changes.push(Change::IssueType(pick(
            &ctx.issue_types,
            issue_type,
            "issue type",
        )?));
    }
    if let Some(status) = args.get("status") {
        let choices = status_choices(&ctx.statuses);
        changes.push(Change::Status(pick(&choices, status, "status")?));
    }
    if let Some(assignee) = args.get("assignee") {
        let user = if clears(assignee) {
            None
        } else {
            Some(pick_user(&ctx.options.users, ctx.options.me, assignee)?)
        };
        changes.push(Change::Assignee(user));
    }
    if let Some(priority) = args.get("priority") {
        changes.push(Change::Priority(pick(
            &ctx.options.priorities,
            priority,
            "priority",
        )?));
    }
    if let Some(categories) = args.get("categories") {
        changes.push(Change::Categories(pick_many(
            &ctx.categories,
            categories,
            "category",
        )?));
    }
    if let Some(versions) = args.get("versions") {
        changes.push(Change::Versions(pick_many(
            &ctx.options.milestones,
            versions,
            "version",
        )?));
    }
    if let Some(milestones) = args.get("milestones") {
        changes.push(Change::Milestones(pick_many(
            &ctx.options.milestones,
            milestones,
            "milestone",
        )?));
    }
    if let Some(resolution) = args.get("resolution") {
        let id = match clears(resolution) {
            true => None,
            false => Some(pick(&ctx.options.resolutions, resolution, "resolution")?),
        };
        changes.push(Change::Resolution(id));
    }
    if let Some(parent) = args.get("parentIssue") {
        let id = if clears(parent) {
            None
        } else if let Some(n) = parent.as_i64() {
            Some(n)
        } else if let Some(cfg) = config {
            Some(resolve_issue_id(cfg, parent)?)
        } else {
            return Err("parentIssue requires a Backlog connection to resolve issue keys".into());
        };
        changes.push(Change::ParentIssue(id));
    }
    if let Some(value) = args.get("startDate") {
        changes.push(Change::StartDate(date(value, "startDate")?));
    }
    if let Some(value) = args.get("dueDate") {
        changes.push(Change::DueDate(date(value, "dueDate")?));
    }
    if let Some(value) = args.get("estimatedHours") {
        changes.push(Change::EstimatedHours(hours(value, "estimatedHours")?));
    }
    if let Some(value) = args.get("actualHours") {
        changes.push(Change::ActualHours(hours(value, "actualHours")?));
    }
    if let Some(notified) = args.get("notifiedUsers").filter(|v| !v.is_null()) {
        let ids = pick_many_users(&ctx.options.users, ctx.options.me, notified)?;
        if !ids.is_empty() {
            changes.push(Change::NotifiedUsers(ids));
        }
    }
    if let Some(comment) = string_arg(args, "comment") {
        changes.push(Change::Comment(comment));
    }
    if let Some(entries) = args.get("customFields").and_then(Value::as_array) {
        for entry in entries {
            changes.push(parse_custom_field_change(entry, &ctx.custom_fields)?);
        }
    }
    Ok(changes)
}

fn parse_custom_field_change(entry: &Value, defs: &[Value]) -> Result<Change, String> {
    let def = if let Some(id) = i64_arg(entry, "id") {
        defs.iter()
            .find(|d| d.get("id").and_then(Value::as_i64) == Some(id))
    } else if let Some(name) = string_arg(entry, "name") {
        defs.iter()
            .find(|d| text(d, "name").eq_ignore_ascii_case(&name))
    } else {
        return Err("Each customFields entry needs an id or name".into());
    };
    let id = match (i64_arg(entry, "id"), def.and_then(|d| d.get("id")?.as_i64())) {
        (_, Some(id)) => id,
        (Some(id), None) => id,
        (None, None) => {
            let available: Vec<String> = defs.iter().map(|d| text(d, "name")).collect();
            return Err(format!(
                "No custom field matching {entry}; available: {}",
                available.join(", ")
            ));
        }
    };
    // For list/checkbox/radio custom fields, resolve item names to item IDs automatically.
    let items: Vec<FieldChoice> = def
        .and_then(|d| d.get("items"))
        .map(|arr| parse_named(arr, |_| false))
        .unwrap_or_default();
    let resolve_val = |v: &Value| -> Result<String, String> {
        if !items.is_empty() {
            return Ok(pick(&items, v, "custom field item")?.to_string());
        }
        Ok(scalar_to_string(v))
    };
    let values = match entry.get("value") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(resolve_val)
            .collect::<Result<Vec<_>, _>>()?,
        Some(other) => vec![resolve_val(other)?],
    };
    let other_value = entry
        .get("otherValue")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Change::CustomField {
        id,
        values,
        other_value,
    })
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// One issue in a list: "- PROJ-12 [In Progress] Subject · Ann · due …".
pub(super) fn issue_line(item: &WorkItem) -> String {
    let mut line = format!("- {} [{}] {}", item.identifier, item.state, item.title);
    if !item.assignees.is_empty() {
        line.push_str(&format!(" · {}", item.assignees.join(", ")));
    }
    if !item.priority.is_empty() {
        line.push_str(&format!(" · {} priority", item.priority));
    }
    if !item.due_date.is_empty() {
        line.push_str(&format!(" · due {}", item.due_date));
    }
    if let Some(updated) = item.updated_at.get(..10) {
        line.push_str(&format!(" · updated {updated}"));
    }
    line
}

/// The issue's state and fields, one per line, those set only.
pub(super) fn field_lines(item: &WorkItem, fields: &IssueFields) -> String {
    let named = |n: &Option<FieldChoice>| n.as_ref().map(|n| n.name.clone());
    let join_named = |list: &[FieldChoice]| -> Option<String> {
        let joined = list
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        (!joined.is_empty()).then_some(joined)
    };
    let hours = |h: Option<f64>| h.map(|h| h.to_string());
    let mut rows: Vec<(String, Option<String>)> = vec![
        ("ID".into(), (fields.issue_id > 0).then(|| fields.issue_id.to_string())),
        ("Status".into(), Some(item.state.clone())),
        ("Type".into(), item.issue_type.as_ref().map(|t| t.name.clone())),
        ("Assignee".into(), named(&fields.assignee)),
        ("Priority".into(), named(&fields.priority)),
        ("Categories".into(), join_named(&fields.categories)),
        ("Versions".into(), join_named(&fields.versions)),
        ("Milestones".into(), join_named(&fields.milestones)),
        ("Resolution".into(), named(&fields.resolution)),
        (
            "Parent issue ID".into(),
            fields.parent_issue_id.map(|id| id.to_string()),
        ),
        ("Start date".into(), fields.start_date.clone()),
        ("Due date".into(), fields.due_date.clone()),
        ("Estimated hours".into(), hours(fields.estimated_hours)),
        ("Actual hours".into(), hours(fields.actual_hours)),
    ];
    for cf in &fields.custom_fields {
        rows.push((format!("{} (#{})", cf.name, cf.id), Some(cf.value.clone())));
    }
    rows.into_iter()
        .filter_map(|(label, value)| {
            let value = value.filter(|v| !v.trim().is_empty())?;
            Some(format!("{label}: {value}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn issue_text(
    item: &WorkItem,
    details: &Details,
    thread: Option<&Thread>,
    related: Option<&[WorkItem]>,
) -> String {
    let fields = details.backlog.clone().unwrap_or_default();
    let mut out = format!(
        "# {} {}\n{}\n\n{}",
        item.identifier,
        item.title,
        item.url,
        field_lines(item, &fields)
    );
    if !details.author.is_empty() {
        out.push_str(&format!("\nCreated by: {}", details.author));
    }
    if let Some(created) = item.created_at.get(..10) {
        out.push_str(&format!("\nCreated: {created}"));
    }
    if let Some(updated) = item.updated_at.get(..10) {
        out.push_str(&format!("\nUpdated: {updated}"));
    }
    if !fields.attachments.is_empty() {
        out.push_str("\n\n## Attachments\n");
        for att in &fields.attachments {
            out.push_str(&format!(
                "\n- #{} {} ({})",
                att.id,
                att.name,
                format_bytes(att.size)
            ));
        }
    }
    let body = details.body.trim();
    out.push_str("\n\n## Description\n\n");
    out.push_str(if body.is_empty() { "(none)" } else { body });
    if let Some(related) = related {
        out.push_str("\n\n## Related issues\n");
        if related.is_empty() {
            out.push_str("\n(none)");
        } else {
            for rel in related {
                out.push('\n');
                out.push_str(&issue_line(rel));
            }
        }
    }
    if let Some(thread) = thread {
        out.push_str("\n\n## Comments");
        if thread.truncated {
            out.push_str(" (truncated — pass commentMinId/commentMaxId to paginate)");
        }
        if thread.comments.is_empty() {
            out.push_str("\n\n(none)");
        }
        for comment in &thread.comments {
            let when = short_day(&comment.created_at);
            let id_tag = if comment.id.is_empty() {
                String::new()
            } else {
                format!(" · #{}", comment.id)
            };
            out.push_str(&format!(
                "\n\n### {} · {when}{id_tag}\n\n{}",
                comment.author,
                comment.body.trim()
            ));
        }
    }
    out
}
