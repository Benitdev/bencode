//! Git repository and Pull Request tools for `bencode --backlog-mcp`.

use serde_json::Value;

use super::{
    DEFAULT_LIMIT, bool_arg, clears, i64_arg, pick_many_users, pick_user, project_users,
    resolve_issue_id, resolve_project, short_day, string_arg,
};
use crate::backlog::{PAGE, call, call_owned, require_config, require_segment, text};

fn repo_arg(args: &Value) -> Result<String, String> {
    let raw = match args.get("repo") {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    require_segment(&raw, "repository").map(str::to_string)
}

fn pr_base_path(project_id: &str, repo: &str) -> String {
    format!("/projects/{project_id}/git/repositories/{repo}/pullRequests")
}

// ---------------------------------------------------------------------------
// Git repositories
// ---------------------------------------------------------------------------

pub(super) fn list_git_repositories(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;

    if args.get("repo").is_some_and(|v| !v.is_null()) {
        let repo = repo_arg(args)?;
        let data = call(
            &config,
            None,
            &format!("/projects/{}/git/repositories/{repo}", project.id),
            &[],
            &[],
        )?;
        return Ok(format_repo_detail(&data));
    }

    let data = call(
        &config,
        None,
        &format!("/projects/{}/git/repositories", project.id),
        &[],
        &[],
    )?;
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return Ok(format!("Project {} has no Git repositories.", project.key));
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| {
            let id = r.get("id").and_then(Value::as_i64).unwrap_or(0);
            let name = text(r, "name");
            let desc = text(r, "description");
            let pushed = short_day(&text(r, "pushedAt"));
            let mut line = format!("- {name} (id: {id})");
            if !desc.is_empty() {
                line.push_str(&format!(" — {desc}"));
            }
            if !pushed.is_empty() {
                line.push_str(&format!(" · pushed {pushed}"));
            }
            line
        })
        .collect();
    Ok(format!(
        "# Git repositories in {}\n\n{}",
        project.key,
        lines.join("\n")
    ))
}

fn format_repo_detail(r: &Value) -> String {
    let id = r.get("id").and_then(Value::as_i64).unwrap_or(0);
    let name = text(r, "name");
    let desc = text(r, "description");
    let http = text(r, "httpUrl");
    let ssh = text(r, "sshUrl");
    let pushed = short_day(&text(r, "pushedAt"));
    let mut out = format!("# {name} (id: {id})");
    if !desc.is_empty() {
        out.push_str(&format!("\n{desc}"));
    }
    if !http.is_empty() {
        out.push_str(&format!("\nHTTP: {http}"));
    }
    if !ssh.is_empty() {
        out.push_str(&format!("\nSSH: {ssh}"));
    }
    if !pushed.is_empty() {
        out.push_str(&format!("\nLast pushed: {pushed}"));
    }
    out
}

// ---------------------------------------------------------------------------
// Pull requests
// ---------------------------------------------------------------------------

pub(super) fn list_pull_requests(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let repo = repo_arg(args)?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut query: Vec<(String, String)> = Vec::new();

    if let Some(statuses) = args.get("statuses").and_then(Value::as_array) {
        for s in statuses {
            let id = parse_pr_status(s)?;
            query.push(("statusId[]".into(), id.to_string()));
        }
    }
    let needs_users = args.get("assignee").is_some() || args.get("createdBy").is_some();
    let users = if needs_users {
        project_users(&config, &project.id)?
    } else {
        Vec::new()
    };
    if let Some(wanted) = args.get("assignee").filter(|v| !v.is_null()) {
        for id in pick_many_users(&users, config.user_id, wanted)? {
            query.push(("assigneeId[]".into(), id.to_string()));
        }
    }
    if let Some(wanted) = args.get("createdBy").filter(|v| !v.is_null()) {
        for id in pick_many_users(&users, config.user_id, wanted)? {
            query.push(("createdUserId[]".into(), id.to_string()));
        }
    }
    if let Some(issue) = args.get("issue").filter(|v| !v.is_null()) {
        let id = resolve_issue_id(&config, issue)?;
        query.push(("issueId[]".into(), id.to_string()));
    }

    let base = pr_base_path(&project.id, &repo);
    if bool_arg(args, "countOnly").unwrap_or(false) {
        let data = call_owned(&config, None, &format!("{base}/count"), &query, &[])?;
        let count = data.get("count").and_then(Value::as_i64).unwrap_or(0);
        return Ok(format!("Matching pull requests: {count}"));
    }

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n as usize)
        .clamp(1, PAGE);
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    query.push(("count".into(), limit.to_string()));
    if offset > 0 {
        query.push(("offset".into(), offset.to_string()));
    }

    let data = call_owned(&config, None, &base, &query, &[])?;
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return Ok("No pull requests match.".into());
    }
    let mut lines: Vec<String> = rows.iter().map(pr_line).collect();
    if rows.len() >= limit {
        lines.push(format!(
            "\n(Showing {} pull requests from offset {offset}; pass offset: {} for more.)",
            rows.len(),
            offset + rows.len() as u64
        ));
    }
    Ok(lines.join("\n"))
}

pub(super) fn parse_pr_status(value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(n) if matches!(n.as_i64(), Some(1 | 2 | 3)) => Ok(n.as_i64().unwrap()),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "open" | "1" => Ok(1),
            "closed" | "2" => Ok(2),
            "merged" | "3" => Ok(3),
            other => Err(format!(
                "Unknown pull request status {other}; choose one of: open (1), closed (2), merged (3)"
            )),
        },
        _ => Err(format!("Invalid pull request status: {value}")),
    }
}

fn pr_line(pr: &Value) -> String {
    let number = pr.get("number").and_then(Value::as_i64).unwrap_or(0);
    let status = pr
        .get("status")
        .map(|s| text(s, "name"))
        .unwrap_or_default();
    let summary = text(pr, "summary");
    let branch = text(pr, "branch");
    let base = text(pr, "base");
    let assignee = pr
        .get("assignee")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let updated = short_day(&text(pr, "updated"));
    let mut line = format!("- #{number} [{status}] {summary} ({branch} → {base})");
    if !assignee.is_empty() {
        line.push_str(&format!(" · {assignee}"));
    }
    if !updated.is_empty() {
        line.push_str(&format!(" · updated {updated}"));
    }
    line
}

pub(super) fn get_pull_request(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let repo = repo_arg(args)?;
    let number = i64_arg(args, "number").ok_or("Give the pull request number")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let path = format!("{}/{number}", pr_base_path(&project.id, &repo));
    let pr = call(&config, None, &path, &[], &[])?;

    let status = pr
        .get("status")
        .map(|s| text(s, "name"))
        .unwrap_or_default();
    let summary = text(&pr, "summary");
    let branch = text(&pr, "branch");
    let base = text(&pr, "base");
    let author = pr
        .get("createdUser")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let assignee = pr
        .get("assignee")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let issue_key = pr
        .get("issue")
        .map(|i| text(i, "issueKey"))
        .unwrap_or_default();
    let url = format!(
        "https://{}/git/{}/{}/pullRequests/{number}",
        config.space, project.key, repo
    );
    let mut out = format!(
        "# PR #{number} {summary}\n{url}\n\nStatus: {status}\nBranches: {branch} → {base}\nAuthor: {author}"
    );
    if !assignee.is_empty() {
        out.push_str(&format!("\nAssignee: {assignee}"));
    }
    if !issue_key.is_empty() {
        out.push_str(&format!("\nIssue: {issue_key}"));
    }
    out.push_str(&format!(
        "\nCreated: {} · Updated: {}",
        short_day(&text(&pr, "created")),
        short_day(&text(&pr, "updated"))
    ));
    let desc = text(&pr, "description");
    out.push_str("\n\n## Description\n\n");
    out.push_str(if desc.trim().is_empty() {
        "(none)"
    } else {
        desc.trim()
    });

    if bool_arg(args, "comments").unwrap_or(false) {
        let mut query = Vec::new();
        let count = args
            .get("commentCount")
            .and_then(Value::as_u64)
            .map_or(PAGE, |n| n as usize)
            .clamp(1, PAGE);
        let order = string_arg(args, "commentOrder").unwrap_or_else(|| "asc".into());
        query.push(("count", count.to_string()));
        query.push(("order", order));
        if let Some(min_id) = i64_arg(args, "commentMinId") {
            query.push(("minId", min_id.to_string()));
        }
        if let Some(max_id) = i64_arg(args, "commentMaxId") {
            query.push(("maxId", max_id.to_string()));
        }
        let comments = call(&config, None, &format!("{path}/comments"), &query, &[])?;
        let rows = comments.as_array().map(Vec::as_slice).unwrap_or_default();
        out.push_str("\n\n## Comments");
        if rows.is_empty() {
            out.push_str("\n\n(none)");
        }
        for c in rows {
            let cid = c.get("id").and_then(Value::as_i64).unwrap_or(0);
            let who = c
                .get("createdUser")
                .map(|u| text(u, "name"))
                .unwrap_or_default();
            let when = short_day(&text(c, "created"));
            let body = text(c, "content");
            if body.trim().is_empty() {
                continue;
            }
            out.push_str(&format!("\n\n### {who} · {when} · #{cid}\n\n{}", body.trim()));
        }
    }

    Ok(out)
}

pub(super) fn create_pull_request(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let repo = repo_arg(args)?;
    let summary = string_arg(args, "summary").ok_or("Give the pull request a summary")?;
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .ok_or("Give the pull request a description")?;
    let base = string_arg(args, "base").ok_or("Specify the base branch")?;
    let branch = string_arg(args, "branch").ok_or("Specify the source branch")?;

    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut form = vec![
        ("summary".to_string(), summary),
        ("description".to_string(), description.to_string()),
        ("base".to_string(), base),
        ("branch".to_string(), branch),
    ];
    if let Some(issue) = args.get("issue").filter(|v| !v.is_null()) {
        form.push(("issueId".into(), resolve_issue_id(&config, issue)?.to_string()));
    }
    let needs_users = args.get("assignee").is_some() || args.get("notifiedUsers").is_some();
    if needs_users {
        let users = project_users(&config, &project.id)?;
        if let Some(assignee) = args.get("assignee").filter(|v| !clears(v)) {
            let uid = pick_user(&users, config.user_id, assignee)?;
            form.push(("assigneeId".into(), uid.to_string()));
        }
        if let Some(notified) = args.get("notifiedUsers").filter(|v| !v.is_null()) {
            for uid in pick_many_users(&users, config.user_id, notified)? {
                form.push(("notifiedUserId[]".into(), uid.to_string()));
            }
        }
    }
    let data = call_owned(
        &config,
        Some("POST"),
        &pr_base_path(&project.id, &repo),
        &[],
        &form,
    )?;
    let number = data.get("number").and_then(Value::as_i64).unwrap_or(0);
    let url = format!(
        "https://{}/git/{}/{}/pullRequests/{number}",
        config.space, project.key, repo
    );
    Ok(format!("Created PR #{number}: {url}"))
}

pub(super) fn update_pull_request(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let repo = repo_arg(args)?;
    let number = i64_arg(args, "number").ok_or("Give the pull request number")?;
    // Backlog's Update Pull Request takes no status: one sent would be
    // ignored and the PR reported updated all the same.
    if args.get("status").is_some_and(|v| !v.is_null()) {
        return Err("Backlog's API cannot change a pull request's status; \
            merge or close it in Backlog or with git"
            .into());
    }
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut form: Vec<(String, String)> = Vec::new();

    if let Some(summary) = string_arg(args, "summary") {
        form.push(("summary".into(), summary));
    }
    if let Some(desc) = args.get("description").and_then(Value::as_str) {
        form.push(("description".into(), desc.to_string()));
    }
    if let Some(issue) = args.get("issue") {
        let val = if clears(issue) {
            String::new()
        } else {
            resolve_issue_id(&config, issue)?.to_string()
        };
        form.push(("issueId".into(), val));
    }
    let needs_users = args.get("assignee").is_some() || args.get("notifiedUsers").is_some();
    if needs_users {
        let users = project_users(&config, &project.id)?;
        if let Some(assignee) = args.get("assignee") {
            let val = if clears(assignee) {
                String::new()
            } else {
                pick_user(&users, config.user_id, assignee)?.to_string()
            };
            form.push(("assigneeId".into(), val));
        }
        if let Some(notified) = args.get("notifiedUsers").filter(|v| !v.is_null()) {
            for uid in pick_many_users(&users, config.user_id, notified)? {
                form.push(("notifiedUserId[]".into(), uid.to_string()));
            }
        }
    }
    if let Some(comment) = string_arg(args, "comment") {
        form.push(("comment".into(), comment));
    }
    if form.is_empty() {
        return Err("Name at least one pull request field to update".into());
    }
    let _ = call_owned(
        &config,
        Some("PATCH"),
        &format!("{}/{number}", pr_base_path(&project.id, &repo)),
        &[],
        &form,
    )?;
    Ok(format!("Updated PR #{number} in {}/{repo}.", project.key))
}

pub(super) fn comment_pull_request(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let repo = repo_arg(args)?;
    let number = i64_arg(args, "number").ok_or("Give the pull request number")?;
    let content = string_arg(args, "content").ok_or("Comment cannot be empty")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let base = format!("{}/{number}/comments", pr_base_path(&project.id, &repo));

    if let Some(comment_id) = i64_arg(args, "commentId") {
        let _ = call(
            &config,
            Some("PATCH"),
            &format!("{base}/{comment_id}"),
            &[],
            &[("content", &content)],
        )?;
        return Ok(format!(
            "Updated comment #{comment_id} on PR #{number} in {}/{repo}.",
            project.key
        ));
    }

    let mut form = vec![("content".to_string(), content)];
    if let Some(notified) = args.get("notifiedUsers").filter(|v| !v.is_null()) {
        let users = project_users(&config, &project.id)?;
        for uid in pick_many_users(&users, config.user_id, notified)? {
            form.push(("notifiedUserId[]".into(), uid.to_string()));
        }
    }
    let data = call_owned(&config, Some("POST"), &base, &[], &form)?;
    let cid = data.get("id").and_then(Value::as_i64).unwrap_or(0);
    Ok(format!(
        "Commented (#{cid}) on PR #{number} in {}/{repo}.",
        project.key
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_status_change_is_refused_before_anything_is_sent() {
        let err = update_pull_request(&json!({
            "project": "PROJ", "repo": "app", "number": 3, "status": "merged"
        }))
        .unwrap_err();
        assert!(err.contains("cannot change a pull request's status"), "{err}");
    }
}
