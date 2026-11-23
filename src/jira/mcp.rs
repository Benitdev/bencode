//! `bencode --jira-mcp`: the Jira site connected in Settings, as an MCP
//! server every harness's agent gets while Jira is connected (`app/jira.rs`).
//! Covers reading projects, searching and inspecting issues, comments,
//! workflow transitions, creating/updating issues, and searching assignable
//! users over Jira REST API v3.
//!
//! It reads the connection from the data folder itself: the API token never
//! reaches argv or a CLI's config. Writes act as the user; the instructions
//! keep them to what the user asked.

use serde_json::{Value, json};

use super::{
    adf, call_json, encode, parse_issue, parse_transitions, projects, require_config,
    require_issue_key, require_segment, string_field,
};
use crate::mcp::server::{self, Reply, Server};

const MCP_ARG: &str = "--jira-mcp";
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

/// Runs the MCP server when `main` was started as one; returns the exit
/// code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(MCP_ARG) {
        return None;
    }
    Some(server::run(&JiraServer))
}

/// How an agent CLI starts this server (this binary with `--jira-mcp`).
pub fn launch() -> anyhow::Result<bencode_core::mcp::McpLaunch> {
    let exe = std::env::current_exe()?;
    Ok(bencode_core::mcp::McpLaunch {
        name: bencode_core::mcp::JIRA_SERVER.to_string(),
        command: exe.to_string_lossy().into_owned(),
        args: vec![MCP_ARG.to_string()],
    })
}

struct JiraServer;

impl Server for JiraServer {
    fn name(&self) -> &'static str {
        bencode_core::mcp::JIRA_SERVER
    }

    fn instructions(&self) -> &'static str {
        "Connected to the user's Jira Cloud site. Use these tools to read projects, search issues \
         (via JQL or filters), inspect issue details, comments and available transitions, or find \
         assignable users. Only create or edit issues, change status transitions, or post comments \
         when the user asked for it."
    }

    fn tools(&self) -> Value {
        schemas()
    }

    fn call(&self, tool: &str, args: &Value) -> Reply {
        let result = match tool {
            "jira_get_myself" => tool_get_myself(),
            "jira_list_projects" => tool_list_projects(),
            "jira_search_issues" => tool_search_issues(args),
            "jira_get_issue" => tool_get_issue(args),
            "jira_list_transitions" => tool_list_transitions(args),
            "jira_transition_issue" => tool_transition_issue(args),
            "jira_add_comment" => tool_add_comment(args),
            "jira_create_issue" => tool_create_issue(args),
            "jira_update_issue" => tool_update_issue(args),
            "jira_search_users" => tool_search_users(args),
            _ => return Reply::error(format!("unknown tool {tool}")),
        };
        match result {
            Ok(text) => Reply::text(text),
            Err(err) => Reply::error(err),
        }
    }
}

/// Accepts an issue key (`ENG-42`) or its Jira browse URL.
fn extract_issue_key(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let candidate = if let Some((_, tail)) = raw.split_once("/browse/") {
        tail.split(['?', '#', '/']).next().unwrap_or("")
    } else {
        raw
    };
    let key = require_issue_key(candidate)?;
    Ok(key.to_ascii_uppercase())
}

fn arg_str<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn require_arg_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    arg_str(args, name).ok_or_else(|| format!("missing `{name}`"))
}

fn tool_get_myself() -> Result<String, String> {
    let config = require_config()?;
    let me = call_json(&config, None, "/rest/api/3/myself", None)?;
    let name = string_field(&me, "displayName").unwrap_or_default();
    let email = string_field(&me, "emailAddress").unwrap_or_else(|| config.email.clone());
    let account_id = string_field(&me, "accountId").unwrap_or_else(|| config.account_id.clone());
    let tz = string_field(&me, "timeZone").unwrap_or_default();
    let mut lines = vec![
        format!("Site: https://{}", config.site),
        format!("User: {name}"),
        format!("Email: {email}"),
        format!("Account ID: {account_id}"),
    ];
    if !tz.is_empty() {
        lines.push(format!("Time zone: {tz}"));
    }
    Ok(lines.join("\n"))
}

fn tool_list_projects() -> Result<String, String> {
    let list = projects()?;
    if list.is_empty() {
        return Ok("No accessible Jira projects.".into());
    }
    let lines: Vec<String> = list
        .iter()
        .map(|p| format!("- {} ({}, id: {})", p.key, p.name, p.id))
        .collect();
    Ok(format!("Projects ({}):\n{}", list.len(), lines.join("\n")))
}

/// Escapes a string literal for JQL double-quoted values.
fn jql_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn build_search_jql(args: &Value) -> Result<String, String> {
    if let Some(jql) = arg_str(args, "jql") {
        return Ok(jql.to_string());
    }
    let mut clauses = Vec::new();
    if let Some(project) = arg_str(args, "project") {
        let project = require_segment(project, "project")?;
        clauses.push(format!("project = {}", jql_quote(project)));
    }
    if let Some(status) = arg_str(args, "status") {
        clauses.push(format!("status = {}", jql_quote(status)));
    } else if !args
        .get("includeDone")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        clauses.push("statusCategory != Done".to_string());
    }
    if let Some(assignee) = arg_str(args, "assignee") {
        if assignee.eq_ignore_ascii_case("me") {
            clauses.push("assignee = currentUser()".to_string());
        } else if assignee.eq_ignore_ascii_case("unassigned") {
            clauses.push("assignee is EMPTY".to_string());
        } else {
            clauses.push(format!("assignee = {}", jql_quote(assignee)));
        }
    }
    if let Some(text) = arg_str(args, "text") {
        clauses.push(format!("text ~ {}", jql_quote(text)));
    }
    if clauses.is_empty() {
        clauses.push("updated >= -90d".to_string());
    }
    Ok(format!("{} ORDER BY updated DESC", clauses.join(" AND ")))
}

fn tool_search_issues(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let jql = build_search_jql(args)?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let mut payload = json!({
        "jql": jql,
        "maxResults": limit,
        "fields": super::ISSUE_FIELDS,
    });
    if let Some(token) = arg_str(args, "nextPageToken") {
        payload["nextPageToken"] = json!(token);
    }
    let data = call_json(
        &config,
        Some("POST"),
        "/rest/api/3/search/jql",
        Some(&payload),
    )?;
    let issues = data
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return issues".to_string())?;
    if issues.is_empty() {
        return Ok(format!("No Jira issues matched: {jql}"));
    }
    let mut lines = Vec::with_capacity(issues.len() + 2);
    lines.push(format!("Found {} issue(s) for `{jql}`:", issues.len()));
    for node in issues {
        if let Some(item) = parse_issue(node, &config.site) {
            let assignee = if item.assignees.is_empty() {
                "Unassigned".to_string()
            } else {
                item.assignees.join(", ")
            };
            let priority = if item.priority.is_empty() {
                String::new()
            } else {
                format!(" · {}", item.priority)
            };
            let due = if item.due_date.is_empty() {
                String::new()
            } else {
                format!(" · due {}", item.due_date)
            };
            lines.push(format!(
                "- {} [{}]{priority}{due} ({assignee}): {}",
                item.identifier, item.state, item.title
            ));
        }
    }
    if let Some(next) = string_field(&data, "nextPageToken").filter(|s| !s.is_empty()) {
        lines.push(format!("nextPageToken: {next}"));
    }
    Ok(lines.join("\n"))
}

fn tool_get_issue(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let key = extract_issue_key(require_arg_str(args, "key")?)?;
    let include_comments = args
        .get("comments")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let data = call_json(
        &config,
        None,
        &format!(
            "/rest/api/3/issue/{key}?fields=summary,description,status,issuetype,priority,assignee,reporter,labels,duedate,created,updated,project,parent,subtasks,issuelinks"
        ),
        None,
    )?;
    let fields = data
        .get("fields")
        .ok_or_else(|| "Jira did not return that issue".to_string())?;
    let summary = string_field(fields, "summary").unwrap_or_default();
    let status = fields
        .pointer("/status/name")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let issue_type = fields
        .pointer("/issuetype/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let priority = fields
        .pointer("/priority/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let assignee = fields
        .pointer("/assignee/displayName")
        .and_then(Value::as_str)
        .unwrap_or("Unassigned");
    let reporter = fields
        .pointer("/reporter/displayName")
        .and_then(Value::as_str)
        .unwrap_or("");
    let due = string_field(fields, "duedate").unwrap_or_default();
    let labels: Vec<&str> = fields
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out = vec![
        format!("# {key}: {summary}"),
        format!("URL: https://{}/browse/{key}", config.site),
        format!("Status: {status}"),
    ];
    if !issue_type.is_empty() {
        out.push(format!("Type: {issue_type}"));
    }
    if !priority.is_empty() {
        out.push(format!("Priority: {priority}"));
    }
    out.push(format!("Assignee: {assignee}"));
    if !reporter.is_empty() {
        out.push(format!("Reporter: {reporter}"));
    }
    if !due.is_empty() {
        out.push(format!("Due: {due}"));
    }
    if !labels.is_empty() {
        out.push(format!("Labels: {}", labels.join(", ")));
    }
    if let Some(parent_key) = fields.pointer("/parent/key").and_then(Value::as_str) {
        let parent_summary = fields
            .pointer("/parent/fields/summary")
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push(format!("Parent: {parent_key} {parent_summary}"));
    }
    let body = adf::rich_text(fields.get("description"));
    if !body.is_empty() {
        out.push(String::new());
        out.push("## Description".into());
        out.push(body);
    }
    if include_comments {
        let thread = super::thread(&key)?;
        out.push(String::new());
        out.push(format!("## Comments ({})", thread.comments.len()));
        if thread.comments.is_empty() {
            out.push("No comments.".into());
        } else {
            for comment in &thread.comments {
                out.push(format!(
                    "\n### #{} {} ({})\n{}",
                    comment.id, comment.author, comment.created_at, comment.body
                ));
            }
            if thread.truncated {
                out.push("\n(Older comments omitted.)".into());
            }
        }
    }
    Ok(out.join("\n"))
}

fn tool_list_transitions(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let key = extract_issue_key(require_arg_str(args, "key")?)?;
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}/transitions"),
        None,
    )?;
    let list = parse_transitions(&data);
    if list.is_empty() {
        return Ok(format!("No available transitions for {key}."));
    }
    let lines: Vec<String> = list
        .iter()
        .map(|t| {
            format!(
                "- id {}: {} -> {} ({})",
                t.id, t.name, t.to_status, t.category
            )
        })
        .collect();
    Ok(format!("Transitions for {key}:\n{}", lines.join("\n")))
}

fn tool_transition_issue(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let key = extract_issue_key(require_arg_str(args, "key")?)?;
    let wanted = arg_str(args, "transitionId")
        .or_else(|| arg_str(args, "status"))
        .ok_or_else(|| "provide `transitionId` or `status`".to_string())?;
    let transition_id = if wanted.chars().all(|c| c.is_ascii_digit()) {
        wanted.to_string()
    } else {
        let data = call_json(
            &config,
            None,
            &format!("/rest/api/3/issue/{key}/transitions"),
            None,
        )?;
        let list = parse_transitions(&data);
        list.iter()
            .find(|t| {
                t.to_status.eq_ignore_ascii_case(wanted) || t.name.eq_ignore_ascii_case(wanted)
            })
            .map(|t| t.id.clone())
            .ok_or_else(|| {
                let choices: Vec<String> = list
                    .iter()
                    .map(|t| format!("{} (id {})", t.to_status, t.id))
                    .collect();
                format!(
                    "No transition matching `{wanted}` for {key}. Available: {}",
                    choices.join(", ")
                )
            })?
    };
    let updated = super::transition(&key, &transition_id)?;
    Ok(format!(
        "Transitioned {} to {} ({})",
        updated.identifier, updated.state, updated.url
    ))
}

fn tool_add_comment(args: &Value) -> Result<String, String> {
    let key = extract_issue_key(require_arg_str(args, "key")?)?;
    let body = require_arg_str(args, "body")?;
    let url = super::post_comment(&key, body)?;
    Ok(format!("Comment posted on {key}: {url}"))
}

fn tool_create_issue(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let project = require_segment(require_arg_str(args, "project")?, "project")?;
    let summary = require_arg_str(args, "summary")?;
    let issue_type = arg_str(args, "issueType").unwrap_or("Task");
    let mut fields = serde_json::Map::new();
    fields.insert("project".into(), json!({ "key": project }));
    fields.insert("summary".into(), json!(summary));
    fields.insert("issuetype".into(), json!({ "name": issue_type }));
    if let Some(description) = arg_str(args, "description") {
        fields.insert("description".into(), adf::text_to_adf(description));
    }
    if let Some(priority) = arg_str(args, "priority") {
        fields.insert("priority".into(), json!({ "name": priority }));
    }
    if let Some(due) = arg_str(args, "dueDate") {
        fields.insert("duedate".into(), json!(due));
    }
    if let Some(parent) = arg_str(args, "parent") {
        let parent_key = extract_issue_key(parent)?;
        fields.insert("parent".into(), json!({ "key": parent_key }));
    }
    if let Some(labels) = args.get("labels").and_then(Value::as_array) {
        let list: Vec<&str> = labels.iter().filter_map(Value::as_str).collect();
        if !list.is_empty() {
            fields.insert("labels".into(), json!(list));
        }
    }
    if let Some(assignee) = arg_str(args, "assigneeAccountId") {
        let account_id = if assignee.eq_ignore_ascii_case("me") {
            config.account_id.as_str()
        } else {
            assignee
        };
        fields.insert("assignee".into(), json!({ "accountId": account_id }));
    }
    let data = call_json(
        &config,
        Some("POST"),
        "/rest/api/3/issue",
        Some(&json!({ "fields": fields })),
    )?;
    let key = string_field(&data, "key").unwrap_or_default();
    if key.is_empty() {
        return Err("Jira did not return the created issue key".into());
    }
    Ok(format!(
        "Created {key}: https://{}/browse/{key}",
        config.site
    ))
}

fn tool_update_issue(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let key = extract_issue_key(require_arg_str(args, "key")?)?;
    let mut fields = serde_json::Map::new();
    if let Some(summary) = arg_str(args, "summary") {
        fields.insert("summary".into(), json!(summary));
    }
    if let Some(description) = args.get("description").and_then(Value::as_str) {
        fields.insert("description".into(), adf::text_to_adf(description));
    }
    if let Some(priority) = arg_str(args, "priority") {
        fields.insert("priority".into(), json!({ "name": priority }));
    }
    if let Some(due) = args.get("dueDate") {
        if due.is_null() || due.as_str() == Some("") {
            fields.insert("duedate".into(), Value::Null);
        } else if let Some(s) = due.as_str() {
            fields.insert("duedate".into(), json!(s.trim()));
        }
    }
    if let Some(labels) = args.get("labels").and_then(Value::as_array) {
        let list: Vec<&str> = labels.iter().filter_map(Value::as_str).collect();
        fields.insert("labels".into(), json!(list));
    }
    if let Some(assignee) = args.get("assigneeAccountId") {
        if assignee.is_null() || assignee.as_str() == Some("") {
            fields.insert("assignee".into(), Value::Null);
        } else if let Some(s) = assignee.as_str() {
            let s = s.trim();
            let account_id = if s.eq_ignore_ascii_case("me") {
                config.account_id.as_str()
            } else {
                s
            };
            fields.insert("assignee".into(), json!({ "accountId": account_id }));
        }
    }
    if fields.is_empty() {
        return Err("No fields provided to update".into());
    }
    call_json(
        &config,
        Some("PUT"),
        &format!("/rest/api/3/issue/{key}"),
        Some(&json!({ "fields": fields })),
    )?;
    Ok(format!(
        "Updated {key}: https://{}/browse/{key}",
        config.site
    ))
}

fn tool_search_users(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let query = arg_str(args, "query").unwrap_or("");
    let path = match arg_str(args, "project") {
        Some(project) => {
            let project = require_segment(project, "project")?;
            format!(
                "/rest/api/3/user/assignable/search?project={}&query={}&maxResults=20",
                encode(project),
                encode(query)
            )
        }
        None => format!(
            "/rest/api/3/user/search?query={}&maxResults=20",
            encode(query)
        ),
    };
    let data = call_json(&config, None, &path, None)?;
    let users = data
        .as_array()
        .ok_or_else(|| "Jira did not return users".to_string())?;
    if users.is_empty() {
        return Ok("No matching Jira users.".into());
    }
    let lines: Vec<String> = users
        .iter()
        .filter_map(|u| {
            let id = string_field(u, "accountId").filter(|s| !s.is_empty())?;
            let name = string_field(u, "displayName").unwrap_or_default();
            let email = string_field(u, "emailAddress").unwrap_or_default();
            if email.is_empty() {
                Some(format!("- {name} (accountId: {id})"))
            } else {
                Some(format!("- {name} <{email}> (accountId: {id})"))
            }
        })
        .collect();
    Ok(lines.join("\n"))
}

fn schemas() -> Value {
    json!([
        {
            "name": "jira_get_myself",
            "description": "Connected Jira site and current user (display name, email, accountId).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "jira_list_projects",
            "description": "List accessible Jira projects with their keys, names and numeric ids.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "jira_search_issues",
            "description": "Search Jira issues using a raw JQL query or simple filters (project, status, assignee, text).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "jql": { "type": "string", "description": "Full JQL query. When set, overrides the shorthand filter fields." },
                    "project": { "type": "string", "description": "Project key (e.g. ENG)." },
                    "status": { "type": "string", "description": "Exact status name (e.g. In Progress)." },
                    "includeDone": { "type": "boolean", "description": "Include Done issues when status is not set (default false)." },
                    "assignee": { "type": "string", "description": "\"me\", \"unassigned\", or an accountId from jira_search_users (Jira Cloud's JQL matches users by accountId, not reliably by name)." },
                    "text": { "type": "string", "description": "Full-text search term." },
                    "limit": { "type": "integer", "description": "Max issues to return (default 20, max 100)." },
                    "nextPageToken": { "type": "string", "description": "Pagination token from a previous search." }
                }
            }
        },
        {
            "name": "jira_get_issue",
            "description": "Read a Jira issue's summary, fields, description (as Markdown), and optionally its comments.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Issue key (e.g. ENG-42) or its Jira browse URL." },
                    "comments": { "type": "boolean", "description": "Include comments (oldest first)." }
                },
                "required": ["key"]
            }
        },
        {
            "name": "jira_list_transitions",
            "description": "List available workflow status transitions for a Jira issue.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Issue key (e.g. ENG-42) or URL." }
                },
                "required": ["key"]
            }
        },
        {
            "name": "jira_transition_issue",
            "description": "Transition a Jira issue to another workflow status. Only when the user asked for it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Issue key (e.g. ENG-42) or URL." },
                    "transitionId": { "type": "string", "description": "Numeric transition id from jira_list_transitions." },
                    "status": { "type": "string", "description": "Target status or transition name (matched case-insensitively)." }
                },
                "required": ["key"]
            }
        },
        {
            "name": "jira_add_comment",
            "description": "Post a comment on a Jira issue as the user. Only when the user asked for it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Issue key (e.g. ENG-42) or URL." },
                    "body": { "type": "string", "description": "Plain text comment body." }
                },
                "required": ["key", "body"]
            }
        },
        {
            "name": "jira_create_issue",
            "description": "Create a new Jira issue in a project. Only when the user asked for it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project key (e.g. ENG)." },
                    "summary": { "type": "string", "description": "Issue summary/title." },
                    "issueType": { "type": "string", "description": "Issue type name (default Task, e.g. Bug, Story, Sub-task)." },
                    "description": { "type": "string", "description": "Issue description (plain text)." },
                    "priority": { "type": "string", "description": "Priority name (e.g. High, Medium)." },
                    "labels": { "type": "array", "items": { "type": "string" } },
                    "dueDate": { "type": "string", "description": "Due date YYYY-MM-DD." },
                    "parent": { "type": "string", "description": "Parent issue key (e.g. ENG-10) for sub-tasks or epics." },
                    "assigneeAccountId": { "type": "string", "description": "\"me\" or a Jira accountId from jira_search_users." }
                },
                "required": ["project", "summary"]
            }
        },
        {
            "name": "jira_update_issue",
            "description": "Update an existing Jira issue's fields. Only when the user asked for it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Issue key (e.g. ENG-42) or URL." },
                    "summary": { "type": "string" },
                    "description": { "type": "string" },
                    "priority": { "type": "string" },
                    "labels": { "type": "array", "items": { "type": "string" } },
                    "dueDate": { "type": ["string", "null"], "description": "YYYY-MM-DD, or null to clear." },
                    "assigneeAccountId": { "type": ["string", "null"], "description": "\"me\", accountId, or null to unassign." }
                },
                "required": ["key"]
            }
        },
        {
            "name": "jira_search_users",
            "description": "Find Jira users (and their accountIds), optionally filtered to users assignable to a project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Name or email substring to search for." },
                    "project": { "type": "string", "description": "Optional project key to limit to assignable users." }
                }
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_issue_key_handles_keys_and_urls() {
        assert_eq!(extract_issue_key("eng-42").unwrap(), "ENG-42");
        assert_eq!(
            extract_issue_key("https://acme.atlassian.net/browse/ENG-42?focusedCommentId=1")
                .unwrap(),
            "ENG-42"
        );
        assert!(extract_issue_key("not-a-key").is_err());
    }

    #[test]
    fn build_search_jql_respects_raw_and_shorthand_filters() {
        assert_eq!(
            build_search_jql(&json!({ "jql": "project = ENG" })).unwrap(),
            "project = ENG"
        );
        assert_eq!(
            build_search_jql(&json!({
                "project": "ENG",
                "assignee": "me",
                "text": "auth \"bug\""
            }))
            .unwrap(),
            "project = \"ENG\" AND statusCategory != Done AND assignee = currentUser() AND text ~ \"auth \\\"bug\\\"\" ORDER BY updated DESC"
        );
        assert_eq!(
            build_search_jql(&json!({ "includeDone": true })).unwrap(),
            "updated >= -90d ORDER BY updated DESC"
        );
    }

    #[test]
    fn tools_list_advertises_valid_schemas() {
        let list = schemas();
        let tools = list.as_array().unwrap();
        assert_eq!(tools.len(), 10);
        for tool in tools {
            assert!(tool["name"].as_str().unwrap().starts_with("jira_"));
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }
}
