//! Notification and watching list tools for `bencode --backlog-mcp`.

use serde_json::Value;

use super::{
    DEFAULT_LIMIT, bool_arg, i64_arg, parse_issue_key, pick_user, short_day, space_users,
    string_arg,
};
use crate::backlog::{PAGE, call, call_owned, require_config, text};

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub(super) fn list_notifications(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n as usize)
        .clamp(1, PAGE);
    let mut query = vec![("count", limit.to_string())];
    if let Some(order) = string_arg(args, "order") {
        query.push(("order", order));
    }
    if let Some(min_id) = i64_arg(args, "minId") {
        query.push(("minId", min_id.to_string()));
    }
    if let Some(max_id) = i64_arg(args, "maxId") {
        query.push(("maxId", max_id.to_string()));
    }

    let (list_res, count_res) = std::thread::scope(|scope| {
        let list = scope.spawn(|| call(&config, None, "/notifications", &query, &[]));
        let count = scope.spawn(|| {
            call(
                &config,
                None,
                "/notifications/count",
                &[
                    ("alreadyRead", "false".to_string()),
                    ("resourceAlreadyRead", "false".to_string()),
                ],
                &[],
            )
        });
        (
            list.join()
                .unwrap_or_else(|_| Err("Notification list failed".into())),
            count
                .join()
                .unwrap_or_else(|_| Err("Notification count failed".into())),
        )
    });

    let list_data = list_res?;
    let rows = list_data.as_array().map(Vec::as_slice).unwrap_or_default();
    let unread = count_res
        .ok()
        .and_then(|v| v.get("count")?.as_i64())
        .unwrap_or(0);
    if rows.is_empty() {
        return Ok(format!("No notifications (unread: {unread})."));
    }
    let mut out = format!("# Notifications (unread: {unread})\n");
    for row in rows {
        out.push('\n');
        out.push_str(&format_notification(row));
    }
    Ok(out)
}

pub(super) fn format_notification(n: &Value) -> String {
    let id = n.get("id").and_then(Value::as_i64).unwrap_or(0);
    let unread = n.get("resourceAlreadyRead").and_then(Value::as_bool) != Some(true);
    let mark = if unread { "●" } else { "○" };
    let sender = n
        .get("sender")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let reason = notification_reason(n.get("reason").and_then(Value::as_i64).unwrap_or(0));
    let when = short_day(&text(n, "created"));

    let target = if let Some(issue) = n.get("issue").filter(|v| v.is_object()) {
        format!("{} {}", text(issue, "issueKey"), text(issue, "summary"))
    } else if let Some(pr) = n.get("pullRequest").filter(|v| v.is_object()) {
        let num = pr.get("number").and_then(Value::as_i64).unwrap_or(0);
        format!("PR #{num} {}", text(pr, "summary"))
    } else if let Some(doc) = n.get("document").filter(|v| v.is_object()) {
        format!("Doc `{}` {}", text(doc, "id"), text(doc, "title"))
    } else {
        n.get("project")
            .map(|p| text(p, "projectKey"))
            .unwrap_or_default()
    };

    let snippet = n
        .get("comment")
        .and_then(|c| c.get("content"))
        .or_else(|| n.get("pullRequestComment").and_then(|c| c.get("content")))
        .or_else(|| n.get("documentComment").and_then(|c| c.get("plain")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let first = s.lines().next().unwrap_or("");
            let cut: String = first.chars().take(100).collect();
            format!(" — \"{cut}\"")
        })
        .unwrap_or_default();

    format!("- {mark} #{id} [{when}] {sender} ({reason}): {target}{snippet}")
}

fn notification_reason(code: i64) -> &'static str {
    match code {
        1 => "assigned",
        2 => "commented",
        3 => "issue created",
        4 => "issue updated",
        5 => "file added",
        6 => "added to project",
        9 => "other",
        10 => "PR assigned",
        11 => "PR commented",
        12 => "PR created",
        13 => "PR updated",
        _ => "notification",
    }
}

pub(super) fn mark_notifications_read(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    if bool_arg(args, "all").unwrap_or(false) {
        let data = call(&config, Some("POST"), "/notifications/markAsRead", &[], &[])?;
        let count = data.get("count").and_then(Value::as_i64).unwrap_or(0);
        return Ok(format!(
            "Reset unread notification count (remaining unread: {count})."
        ));
    }
    let id = i64_arg(args, "id")
        .ok_or("Provide a notification id to mark as read, or pass all: true")?;
    let _ = call(
        &config,
        Some("POST"),
        &format!("/notifications/{id}/markAsRead"),
        &[],
        &[],
    )?;
    Ok(format!("Marked notification #{id} as read."))
}

// ---------------------------------------------------------------------------
// Watchings
// ---------------------------------------------------------------------------

pub(super) fn list_watchings(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let user_id = match args.get("user").filter(|v| !v.is_null()) {
        Some(wanted) => {
            let users = space_users(&config)?;
            pick_user(&users, config.user_id, wanted)?
        }
        None => config.user_id,
    };
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n as usize)
        .clamp(1, PAGE);
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let unread_only = bool_arg(args, "unreadOnly").unwrap_or(false);

    let mut list_query = vec![("count", limit.to_string())];
    if offset > 0 {
        list_query.push(("offset", offset.to_string()));
    }
    if let Some(sort) = string_arg(args, "sort") {
        list_query.push(("sort", sort));
    }
    if let Some(order) = string_arg(args, "order") {
        list_query.push(("order", order));
    }
    if unread_only {
        list_query.push(("resourceAlreadyRead", "false".to_string()));
    }

    let mut count_query = Vec::new();
    if unread_only {
        count_query.push(("resourceAlreadyRead", "false".to_string()));
    }

    let (list_res, count_res) = std::thread::scope(|scope| {
        let list = scope.spawn(|| {
            call(
                &config,
                None,
                &format!("/users/{user_id}/watchings"),
                &list_query,
                &[],
            )
        });
        let count = scope.spawn(|| {
            call(
                &config,
                None,
                &format!("/users/{user_id}/watchings/count"),
                &count_query,
                &[],
            )
        });
        (
            list.join()
                .unwrap_or_else(|_| Err("Watching list failed".into())),
            count
                .join()
                .unwrap_or_else(|_| Err("Watching count failed".into())),
        )
    });

    let data = list_res?;
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    let total = count_res
        .ok()
        .and_then(|v| v.get("count")?.as_i64())
        .unwrap_or(rows.len() as i64);
    if rows.is_empty() {
        return Ok(format!("No watched items (total: {total})."));
    }
    let mut out = format!("# Watched issues ({} shown / {} total)\n", rows.len(), total);
    for w in rows {
        let wid = w.get("id").and_then(Value::as_i64).unwrap_or(0);
        let unread = w.get("resourceAlreadyRead").and_then(Value::as_bool) != Some(true);
        let mark = if unread { "●" } else { "○" };
        let note = text(w, "note");
        let issue = w.get("issue").unwrap_or(&Value::Null);
        let key = text(issue, "issueKey");
        let status = issue
            .get("status")
            .map(|s| text(s, "name"))
            .unwrap_or_default();
        let summary = text(issue, "summary");
        let updated = short_day(&text(w, "lastContentUpdated"));
        let mut line = format!("\n- {mark} watch #{wid}: {key} [{status}] {summary}");
        if !note.is_empty() {
            line.push_str(&format!(" · note: {note}"));
        }
        if !updated.is_empty() {
            line.push_str(&format!(" · updated {updated}"));
        }
        out.push_str(&line);
    }
    Ok(out)
}

pub(super) fn manage_watching(args: &Value) -> Result<String, String> {
    let action = string_arg(args, "action")
        .ok_or("Specify action: add, update, delete or mark_read")?;
    let config = require_config()?;

    match action.to_ascii_lowercase().as_str() {
        "add" => {
            let issue_arg = args
                .get("issue")
                .filter(|v| !v.is_null())
                .ok_or("Specify issue: its key, URL or numeric id")?;
            let issue_id_or_key = match issue_arg {
                Value::Number(n) => n.to_string(),
                Value::String(s) => {
                    let trimmed = s.trim();
                    if trimmed.parse::<i64>().is_ok() {
                        trimmed.to_string()
                    } else {
                        parse_issue_key(trimmed)?
                    }
                }
                _ => return Err("Invalid issue parameter".into()),
            };
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let form = vec![
                ("issueIdOrKey".to_string(), issue_id_or_key.clone()),
                ("note".to_string(), note),
            ];
            let data = call_owned(&config, Some("POST"), "/watchings", &[], &form)?;
            let wid = data.get("id").and_then(Value::as_i64).unwrap_or(0);
            Ok(format!("Watching {issue_id_or_key} (watch #{wid})."))
        }
        "update" => {
            let wid = i64_arg(args, "watchId").ok_or("Give the numeric watchId")?;
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .ok_or("Provide note text to update")?;
            let _ = call(
                &config,
                Some("PATCH"),
                &format!("/watchings/{wid}"),
                &[],
                &[("note", note)],
            )?;
            Ok(format!("Updated note on watch #{wid}."))
        }
        "delete" => {
            let wid = i64_arg(args, "watchId").ok_or("Give the numeric watchId")?;
            let _ = call(&config, Some("DELETE"), &format!("/watchings/{wid}"), &[], &[])?;
            Ok(format!("Removed watch #{wid}."))
        }
        "mark_read" => {
            let wid = i64_arg(args, "watchId").ok_or("Give the numeric watchId")?;
            let _ = call(
                &config,
                Some("POST"),
                &format!("/watchings/{wid}/markAsRead"),
                &[],
                &[],
            )?;
            Ok(format!("Marked watch #{wid} as read."))
        }
        other => Err(format!(
            "Unknown action {other}; use add, update, delete or mark_read"
        )),
    }
}
