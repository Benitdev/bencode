//! `bencode --backlog-mcp`: the Backlog space connected in Settings, as an
//! MCP server every harness's agent gets while it is connected
//! (`app/backlog.rs`), so a thread reads and works its issues as the Inbox
//! does. It reads the connection from the data folder itself: the API key
//! never reaches argv or a CLI's config. Writes (a comment, a change) act
//! as the user; the instructions keep them to what the user asked.

use serde_json::{Value, json};

use super::{Change, FieldChoice, FieldOptions, IssueFields, Search, StatusOption};
use crate::mcp::server::{self, Reply, Server};
use crate::work_items::{Details, Thread, WorkItem};

const MCP_ARG: &str = "--backlog-mcp";
/// What a search lists unless asked for more.
const DEFAULT_LIMIT: usize = 20;

/// Runs the MCP server when `main` was started as one; returns the exit
/// code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    (args.next().as_deref() == Some(MCP_ARG)).then(|| server::run(&Backlog))
}

/// How an agent CLI starts the server.
pub fn launch() -> anyhow::Result<bencode_core::mcp::McpLaunch> {
    use anyhow::Context as _;
    let exe = std::env::current_exe().context("cannot find BenCode's executable")?;
    Ok(bencode_core::mcp::McpLaunch {
        name: bencode_core::mcp::BACKLOG_SERVER.to_string(),
        command: exe.to_string_lossy().into_owned(),
        args: vec![MCP_ARG.to_string()],
    })
}

struct Backlog;

impl Server for Backlog {
    fn name(&self) -> &'static str {
        bencode_core::mcp::BACKLOG_SERVER
    }

    fn instructions(&self) -> &'static str {
        "These tools reach the Nulab Backlog space the user connected in BenCode, the app \
        the user is talking to you from, as the user. When the user names a Backlog issue \
        (PROJ-123, or a link to …/view/PROJ-123), read it with backlog_get_issue (comments: \
        true for its discussion) before working on it. Comment on or change an issue only \
        when the user asked you to."
    }

    fn tools(&self) -> Value {
        tools()
    }

    fn call(&self, tool: &str, args: &Value) -> Reply {
        let result = match tool {
            "backlog_list_projects" => list_projects(),
            "backlog_search_issues" => search_issues(args),
            "backlog_get_issue" => get_issue(args),
            "backlog_add_comment" => add_comment(args),
            "backlog_update_issue" => update_issue(args),
            _ => Err(format!("Unknown tool {tool}")),
        };
        result.map_or_else(Reply::error, Reply::text)
    }
}

fn tools() -> Value {
    let key = json!({
        "type": "string",
        "description": "The issue's key (PROJ-123) or its Backlog address.",
    });
    let name_or_id = |what: &str| {
        json!({
            "type": ["string", "integer", "null"],
            "description": format!("{what}: its name or id; null clears it."),
        })
    };
    let day = |what: &str| {
        json!({
            "type": ["string", "null"],
            "description": format!("{what}, YYYY-MM-DD; null clears it."),
        })
    };
    let hours = |what: &str| {
        json!({
            "type": ["number", "null"],
            "description": format!("{what}; null clears them."),
        })
    };
    json!([
        {
            "name": "backlog_list_projects",
            "description": "The space's active projects: each key, name and workflow statuses.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "backlog_search_issues",
            "description": "Find issues, most recently updated first: open ones unless includeClosed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "A project key; every project when left out." },
                    "keyword": { "type": "string", "description": "Words in the subject, description or comments." },
                    "assignedToMe": { "type": "boolean" },
                    "includeClosed": { "type": "boolean" },
                    "limit": { "type": "integer", "description": "At most this many (default 20, at most 100)." },
                },
            },
        },
        {
            "name": "backlog_get_issue",
            "description": "An issue's subject, fields and description; its pictures are saved as local files the description links. comments: true adds its comments and changes, oldest first.",
            "inputSchema": {
                "type": "object",
                "properties": { "key": key, "comments": { "type": "boolean" } },
                "required": ["key"],
            },
        },
        {
            "name": "backlog_add_comment",
            "description": "Comment on an issue as the user (Backlog's Markdown or plain text). Only when the user asked for it.",
            "inputSchema": {
                "type": "object",
                "properties": { "key": key, "content": { "type": "string" } },
                "required": ["key", "content"],
            },
        },
        {
            "name": "backlog_update_issue",
            "description": "Change an issue's fields as the user; fields left out stay as they are. Only when the user asked for it. A name that does not match lists the choices.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": key,
                    "status": { "type": ["string", "integer"], "description": "The status: its name or id." },
                    "assignee": {
                        "type": ["string", "integer", "null"],
                        "description": "The assignee: \"me\", a member's name or id; null clears it.",
                    },
                    "priority": { "type": ["string", "integer"], "description": "The priority: its name or id." },
                    "resolution": name_or_id("The resolution"),
                    "milestones": {
                        "type": "array",
                        "items": { "type": ["string", "integer"] },
                        "description": "The milestones, by name or id; [] clears them.",
                    },
                    "startDate": day("The start date"),
                    "dueDate": day("The due date"),
                    "estimatedHours": hours("Estimated hours"),
                    "actualHours": hours("Actual hours"),
                },
                "required": ["key"],
            },
        },
    ])
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn list_projects() -> Result<String, String> {
    let projects = super::projects_with_statuses()?;
    if projects.is_empty() {
        return Ok("The space has no active projects.".into());
    }
    Ok(projects
        .iter()
        .map(|(project, statuses)| {
            let statuses: Vec<&str> = statuses.iter().map(|s| s.name.as_str()).collect();
            format!(
                "- {} — {} (statuses: {})",
                project.key,
                project.name,
                statuses.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn search_issues(args: &Value) -> Result<String, String> {
    let search = Search {
        project: string_arg(args, "project"),
        keyword: string_arg(args, "keyword"),
        assigned_to_me: args["assignedToMe"].as_bool().unwrap_or(false),
        include_closed: args["includeClosed"].as_bool().unwrap_or(false),
        count: (args["limit"].as_u64()).map_or(DEFAULT_LIMIT, |limit| limit as usize),
    };
    let items = super::search(&search)?;
    if items.is_empty() {
        return Ok("No issues match.".into());
    }
    Ok(items.iter().map(issue_line).collect::<Vec<_>>().join("\n"))
}

fn get_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let (item, details) = super::read_issue(&key)?;
    let thread = match args["comments"].as_bool() {
        Some(true) => Some(super::thread(&key)?),
        _ => None,
    };
    Ok(issue_text(&item, &details, thread.as_ref()))
}

fn add_comment(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let content = args["content"].as_str().unwrap_or_default();
    let url = super::post_comment(&key, content)?;
    Ok(format!("Commented on {key}: {url}"))
}

fn update_issue(args: &Value) -> Result<String, String> {
    let key = key_arg(args)?;
    let account = super::account().ok_or("Connect Backlog in Settings")?;
    let item = super::issue(&key, &account.space)?;
    let statuses = match args.get("status") {
        Some(_) => super::statuses(&item.container_id)?,
        None => Vec::new(),
    };
    let needs_options = ["assignee", "priority", "resolution", "milestones"]
        .iter()
        .any(|field| args.get(*field).is_some());
    let options = match needs_options {
        true => super::options(&item.container_id)?,
        false => FieldOptions::default(),
    };
    let changes = changes(args, &statuses, &options)?;
    if changes.is_empty() {
        return Err("Name at least one field to change".into());
    }
    let (item, fields) = super::update_all(&key, &changes)?;
    Ok(format!(
        "Updated {}.\n{}",
        item.identifier,
        field_lines(&item, &fields)
    ))
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

fn string_arg(args: &Value, name: &str) -> Option<String> {
    args[name]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// An issue key as given, or taken from its address
/// (`https://space.backlog.com/view/PROJ-12#comment-3`).
fn key_arg(args: &Value) -> Result<String, String> {
    let given = args["key"].as_str().unwrap_or_default().trim();
    let key = match given.split_once("/view/") {
        Some((_, rest)) => rest.split(['#', '?', '/']).next().unwrap_or_default(),
        None => given,
    };
    if key.is_empty() {
        return Err("Name the issue: its key, like PROJ-123".into());
    }
    Ok(key.to_ascii_uppercase())
}

/// The id of the choice `wanted` names, by name (any case) or id.
fn pick(choices: &[FieldChoice], wanted: &Value, field: &str) -> Result<i64, String> {
    let found = match wanted {
        Value::Number(id) => id.as_i64().filter(|id| choices.iter().any(|c| c.id == *id)),
        Value::String(name) => {
            let name = name.trim();
            (choices.iter())
                .find(|choice| choice.name.eq_ignore_ascii_case(name))
                .map(|choice| choice.id)
                .or_else(|| {
                    let id = name.parse::<i64>().ok()?;
                    choices.iter().any(|c| c.id == id).then_some(id)
                })
        }
        _ => None,
    };
    found.ok_or_else(|| {
        let names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
        format!(
            "No {field} {wanted} on this issue's project; choose one of: {}",
            names.join(", ")
        )
    })
}

/// `null` or an empty string: the field is cleared.
fn clears(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty())
}

fn date(value: &Value, field: &str) -> Result<Option<String>, String> {
    if clears(value) {
        return Ok(None);
    }
    let text = value.as_str().unwrap_or_default().trim();
    let valid = text.len() == 10
        && text.char_indices().all(|(at, c)| match at {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    match valid {
        true => Ok(Some(text.to_string())),
        false => Err(format!("{field} must be YYYY-MM-DD, not {value}")),
    }
}

fn hours(value: &Value, field: &str) -> Result<Option<f64>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .filter(|h| *h >= 0.0)
            .map(Some)
            .ok_or_else(|| format!("{field} must not be negative")),
        _ => Err(format!("{field} must be a number of hours or null")),
    }
}

/// The changes `args` asks for, names resolved against the issue's
/// project.
fn changes(
    args: &Value,
    statuses: &[StatusOption],
    options: &FieldOptions,
) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    if let Some(status) = args.get("status") {
        let choices: Vec<FieldChoice> = (statuses.iter())
            .map(|s| FieldChoice {
                id: s.id,
                name: s.name.clone(),
            })
            .collect();
        changes.push(Change::Status(pick(&choices, status, "status")?));
    }
    if let Some(assignee) = args.get("assignee") {
        let user = match assignee {
            value if clears(value) => None,
            Value::String(me) if me.trim().eq_ignore_ascii_case("me") => Some(options.me),
            value => Some(pick(&options.users, value, "member")?),
        };
        changes.push(Change::Assignee(user));
    }
    if let Some(priority) = args.get("priority") {
        changes.push(Change::Priority(pick(
            &options.priorities,
            priority,
            "priority",
        )?));
    }
    if let Some(resolution) = args.get("resolution") {
        let id = match clears(resolution) {
            true => None,
            false => Some(pick(&options.resolutions, resolution, "resolution")?),
        };
        changes.push(Change::Resolution(id));
    }
    if let Some(milestones) = args.get("milestones") {
        let ids = (milestones.as_array().into_iter().flatten())
            .map(|wanted| pick(&options.milestones, wanted, "milestone"))
            .collect::<Result<Vec<_>, _>>()?;
        changes.push(Change::Milestones(ids));
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
    Ok(changes)
}

// ---------------------------------------------------------------------------
// Answers
// ---------------------------------------------------------------------------

/// One issue in a list: "PROJ-12 [In Progress] Subject · Ann · due …".
fn issue_line(item: &WorkItem) -> String {
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
fn field_lines(item: &WorkItem, fields: &IssueFields) -> String {
    let named = |n: &Option<FieldChoice>| n.as_ref().map(|n| n.name.clone());
    let hours = |h: Option<f64>| h.map(|h| h.to_string());
    let milestones: Vec<&str> = fields.milestones.iter().map(|m| m.name.as_str()).collect();
    let rows = [
        ("Status", Some(item.state.clone())),
        ("Type", item.issue_type.as_ref().map(|t| t.name.clone())),
        ("Assignee", named(&fields.assignee)),
        ("Priority", named(&fields.priority)),
        ("Milestones", Some(milestones.join(", "))),
        ("Resolution", named(&fields.resolution)),
        ("Start date", fields.start_date.clone()),
        ("Due date", fields.due_date.clone()),
        ("Estimated hours", hours(fields.estimated_hours)),
        ("Actual hours", hours(fields.actual_hours)),
    ];
    rows.into_iter()
        .filter_map(|(label, value)| {
            let value = value.filter(|v| !v.trim().is_empty())?;
            Some(format!("{label}: {value}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn issue_text(item: &WorkItem, details: &Details, thread: Option<&Thread>) -> String {
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
    let body = details.body.trim();
    out.push_str("\n\n## Description\n\n");
    out.push_str(if body.is_empty() { "(none)" } else { body });
    if let Some(thread) = thread {
        out.push_str("\n\n## Comments");
        if thread.truncated {
            out.push_str(" (the latest only)");
        }
        if thread.comments.is_empty() {
            out.push_str("\n\n(none)");
        }
        for comment in &thread.comments {
            let when = comment.created_at.get(..10).unwrap_or(&comment.created_at);
            out.push_str(&format!(
                "\n\n### {} · {when}\n\n{}",
                comment.author,
                comment.body.trim()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_items::Comment;

    fn choice(id: i64, name: &str) -> FieldChoice {
        FieldChoice {
            id,
            name: name.into(),
        }
    }

    fn options() -> FieldOptions {
        FieldOptions {
            users: vec![choice(3, "Yumeko"), choice(4, "Ann Lee")],
            milestones: vec![choice(9, "Sprint 04")],
            priorities: vec![choice(2, "High"), choice(3, "Normal")],
            resolutions: vec![choice(0, "Fixed")],
            me: 7,
        }
    }

    fn statuses() -> Vec<StatusOption> {
        [(1, "Open"), (2, "In Progress"), (4, "Closed")]
            .map(|(id, name)| StatusOption {
                id,
                name: name.into(),
                color: String::new(),
            })
            .into()
    }

    #[test]
    fn keys_come_as_typed_or_from_an_address() {
        let key = |given: &str| key_arg(&json!({ "key": given }));
        assert_eq!(key("proj-12").unwrap(), "PROJ-12");
        assert_eq!(
            key("https://acme.backlog.com/view/PROJ_1-42#comment-9").unwrap(),
            "PROJ_1-42"
        );
        assert!(key(" ").is_err());
    }

    #[test]
    fn names_resolve_to_ids_whatever_their_case() {
        let changes = changes(
            &json!({
                "status": "in progress",
                "assignee": "me",
                "priority": 2,
                "milestones": ["sprint 04"],
                "dueDate": "2026-11-20",
                "actualHours": 1.5,
            }),
            &statuses(),
            &options(),
        )
        .unwrap();
        assert_eq!(
            changes,
            [
                Change::Status(2),
                Change::Assignee(Some(7)),
                Change::Priority(2),
                Change::Milestones(vec![9]),
                Change::DueDate(Some("2026-11-20".into())),
                Change::ActualHours(Some(1.5)),
            ]
        );
    }

    #[test]
    fn null_clears_and_left_out_stays() {
        let changes = changes(
            &json!({ "assignee": null, "resolution": "", "milestones": [], "startDate": null }),
            &[],
            &options(),
        )
        .unwrap();
        assert_eq!(
            changes,
            [
                Change::Assignee(None),
                Change::Resolution(None),
                Change::Milestones(Vec::new()),
                Change::StartDate(None),
            ]
        );
        assert!(changes_for(json!({})).unwrap().is_empty());
    }

    fn changes_for(args: Value) -> Result<Vec<Change>, String> {
        changes(&args, &statuses(), &options())
    }

    #[test]
    fn a_name_that_does_not_match_lists_the_choices() {
        let err = changes_for(json!({ "status": "Done" })).unwrap_err();
        assert!(err.contains("Open, In Progress, Closed"), "{err}");
        assert!(changes_for(json!({ "assignee": 99 })).is_err());
        assert!(changes_for(json!({ "dueDate": "20/11/2026" })).is_err());
        assert!(changes_for(json!({ "estimatedHours": -1 })).is_err());
        assert!(changes_for(json!({ "estimatedHours": "2" })).is_err());
    }

    #[test]
    fn an_issue_reads_as_markdown_with_its_comments() {
        let item = WorkItem {
            identifier: "PROJ-12".into(),
            title: "Fix the login".into(),
            url: "https://acme.backlog.com/view/PROJ-12".into(),
            state: "Open".into(),
            created_at: "2026-11-01T02:03:04Z".into(),
            ..Default::default()
        };
        let details = Details {
            body: "It fails.".into(),
            author: "Ann Lee".into(),
            backlog: Some(IssueFields {
                assignee: Some(choice(3, "Yumeko")),
                due_date: Some("2026-11-20".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let thread = Thread {
            comments: vec![Comment {
                author: "Yumeko".into(),
                body: "On it.".into(),
                created_at: "2026-11-02T00:00:00Z".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = issue_text(&item, &details, Some(&thread));
        assert!(text.starts_with("# PROJ-12 Fix the login\nhttps://acme.backlog.com/view/PROJ-12"));
        assert!(text.contains("Status: Open\nAssignee: Yumeko\nDue date: 2026-11-20"));
        assert!(!text.contains("Milestones"));
        assert!(text.contains("Created by: Ann Lee\nCreated: 2026-11-01"));
        assert!(text.contains("## Description\n\nIt fails."));
        assert!(text.contains("### Yumeko · 2026-11-02\n\nOn it."));
        assert!(!issue_text(&item, &details, None).contains("## Comments"));
    }

    #[test]
    fn tools_are_listed_with_schemas() {
        let reply = server::handle(
            &Backlog,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert!(
            tools
                .iter()
                .all(|t| t["name"].as_str().unwrap().starts_with("backlog_"))
        );
    }
}
