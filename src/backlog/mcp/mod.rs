//! `bencode --backlog-mcp`: the Backlog space connected in Settings, as an
//! MCP server every harness's agent gets while it is connected
//! (`app/backlog.rs`). Covers the full Backlog REST API v2 surface (space,
//! projects, issues, comments, attachments, related issues, categories,
//! milestones/versions, custom fields, wikis, documents, Git repositories,
//! pull requests, notifications and watchings).
//!
//! It reads the connection from the data folder itself: the API key never
//! reaches argv or a CLI's config. Writes act as the user; the instructions
//! keep them to what the user asked.
//!
//! Optional toolset filtering: `--toolsets=space,project,issue,...` or
//! `BENCODE_BACKLOG_TOOLSETS` (defaults to all toolsets).

mod activity;
mod git_prs;
mod issues;
mod projects;
mod schemas;
mod wiki_docs;

use std::collections::HashSet;

use serde_json::Value;

use super::fields::{FieldOptions, Named as FieldChoice, parse_named};
use super::{Config, Project, StatusOption, call, require_config, text};
use crate::mcp::server::{self, Reply, Server};

const MCP_ARG: &str = "--backlog-mcp";
/// What a search lists unless asked for more.
pub(super) const DEFAULT_LIMIT: usize = 20;

/// Logical tool groups matching the official Backlog MCP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Toolset {
    Space,
    Project,
    Issue,
    Wiki,
    Document,
    Git,
    Notifications,
}

impl Toolset {
    pub(super) const ALL: [Self; 7] = [
        Self::Space,
        Self::Project,
        Self::Issue,
        Self::Wiki,
        Self::Document,
        Self::Git,
        Self::Notifications,
    ];

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "space" => Some(Self::Space),
            "project" | "projects" => Some(Self::Project),
            "issue" | "issues" => Some(Self::Issue),
            "wiki" | "wikis" => Some(Self::Wiki),
            "document" | "documents" | "doc" | "docs" => Some(Self::Document),
            "git" | "pr" | "prs" | "pullrequests" => Some(Self::Git),
            "notification" | "notifications" => Some(Self::Notifications),
            _ => None,
        }
    }
}

/// Which toolsets are active for this server process.
#[derive(Clone, Debug)]
struct EnabledToolsets(HashSet<Toolset>);

impl EnabledToolsets {
    fn all() -> Self {
        Self(Toolset::ALL.into_iter().collect())
    }

    fn from_spec(spec: &str) -> Self {
        let trimmed = spec.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
            return Self::all();
        }
        let parsed: HashSet<Toolset> = trimmed.split(',').filter_map(Toolset::parse).collect();
        if parsed.is_empty() {
            Self::all()
        } else {
            Self(parsed)
        }
    }

    fn contains(&self, toolset: Toolset) -> bool {
        self.0.contains(&toolset)
    }
}

/// Runs the MCP server when `main` was started as one; returns the exit
/// code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(MCP_ARG) {
        return None;
    }
    let mut toolsets_arg: Option<String> = None;
    while let Some(arg) = args.next() {
        if let Some(rest) = arg.strip_prefix("--toolsets=") {
            toolsets_arg = Some(rest.to_string());
        } else if arg == "--toolsets" || arg == "--enable-toolsets" {
            toolsets_arg = args.next();
        } else if let Some(rest) = arg.strip_prefix("--enable-toolsets=") {
            toolsets_arg = Some(rest.to_string());
        }
    }
    let spec = toolsets_arg
        .or_else(|| std::env::var("BENCODE_BACKLOG_TOOLSETS").ok())
        .or_else(|| std::env::var("ENABLE_TOOLSETS").ok())
        .unwrap_or_default();
    let server = Backlog {
        enabled: EnabledToolsets::from_spec(&spec),
    };
    Some(server::run(&server))
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

struct Backlog {
    enabled: EnabledToolsets,
}

impl Server for Backlog {
    fn name(&self) -> &'static str {
        bencode_core::mcp::BACKLOG_SERVER
    }

    fn instructions(&self) -> &'static str {
        "These tools reach the Nulab Backlog space the user connected in BenCode, the app \
        the user is talking to you from, as the user. They cover the space, projects (members, \
        statuses, types, categories, milestones, custom fields), issues, comments, attachments, \
        related issues, wikis, documents, Git repositories, pull requests, notifications and \
        watchings. When the user names a Backlog issue (PROJ-123, or a link to …/view/PROJ-123), \
        read it with backlog_get_issue (comments: true for its discussion) before working on it. \
        Use backlog_get_project to discover a project's members, categories, milestones, issue \
        types and custom fields. Perform write operations (create, update, delete, comment, \
        mark read, watch) only when the user asked you to."
    }

    fn tools(&self) -> Value {
        Value::Array(
            schemas::all_tools()
                .into_iter()
                .filter(|(toolset, _)| self.enabled.contains(*toolset))
                .map(|(_, schema)| schema)
                .collect(),
        )
    }

    fn call(&self, tool: &str, args: &Value) -> Reply {
        if let Some(toolset) = tool_toolset(tool)
            && !self.enabled.contains(toolset)
        {
            return Reply::error(format!("Tool {tool} is disabled by toolset configuration"));
        }
        // Special-case attachment download so raster images return native MCP image content.
        if tool == "backlog_get_attachment" {
            return issues::get_attachment(args);
        }
        let result = match tool {
            // Space & projects
            "backlog_get_space" => projects::get_space(args),
            "backlog_list_projects" => projects::list_projects(args),
            "backlog_get_project" => projects::get_project(args),
            "backlog_create_category" => projects::create_category(args),
            "backlog_manage_milestone" => projects::manage_milestone(args),
            "backlog_manage_project" => projects::manage_project(args),
            // Issues
            "backlog_search_issues" => issues::search_issues(args),
            "backlog_get_issue" => issues::get_issue(args),
            "backlog_create_issue" => issues::create_issue(args),
            "backlog_update_issue" => issues::update_issue(args),
            "backlog_delete_issue" => issues::delete_issue(args),
            "backlog_add_comment" => issues::add_comment(args),
            "backlog_update_comment" => issues::update_comment(args),
            "backlog_manage_related_issue" => issues::manage_related_issue(args),
            // Wikis & documents
            "backlog_list_wikis" => wiki_docs::list_wikis(args),
            "backlog_get_wiki" => wiki_docs::get_wiki(args),
            "backlog_create_wiki" => wiki_docs::create_wiki(args),
            "backlog_update_wiki" => wiki_docs::update_wiki(args),
            "backlog_get_document_tree" => wiki_docs::get_document_tree(args),
            "backlog_list_documents" => wiki_docs::list_documents(args),
            "backlog_get_document" => wiki_docs::get_document(args),
            "backlog_create_document" => wiki_docs::create_document(args),
            // Git & pull requests
            "backlog_list_git_repositories" => git_prs::list_git_repositories(args),
            "backlog_list_pull_requests" => git_prs::list_pull_requests(args),
            "backlog_get_pull_request" => git_prs::get_pull_request(args),
            "backlog_create_pull_request" => git_prs::create_pull_request(args),
            "backlog_update_pull_request" => git_prs::update_pull_request(args),
            "backlog_comment_pull_request" => git_prs::comment_pull_request(args),
            // Notifications & watchings
            "backlog_list_notifications" => activity::list_notifications(args),
            "backlog_mark_notifications_read" => activity::mark_notifications_read(args),
            "backlog_list_watchings" => activity::list_watchings(args),
            "backlog_manage_watching" => activity::manage_watching(args),
            _ => Err(format!("Unknown tool {tool}")),
        };
        result.map_or_else(Reply::error, Reply::text)
    }
}

fn tool_toolset(tool: &str) -> Option<Toolset> {
    Some(match tool {
        "backlog_get_space" => Toolset::Space,
        "backlog_list_projects"
        | "backlog_get_project"
        | "backlog_create_category"
        | "backlog_manage_milestone"
        | "backlog_manage_project" => Toolset::Project,
        "backlog_search_issues"
        | "backlog_get_issue"
        | "backlog_create_issue"
        | "backlog_update_issue"
        | "backlog_delete_issue"
        | "backlog_add_comment"
        | "backlog_update_comment"
        | "backlog_get_attachment"
        | "backlog_manage_related_issue"
        | "backlog_list_watchings"
        | "backlog_manage_watching" => Toolset::Issue,
        "backlog_list_wikis"
        | "backlog_get_wiki"
        | "backlog_create_wiki"
        | "backlog_update_wiki" => Toolset::Wiki,
        "backlog_get_document_tree"
        | "backlog_list_documents"
        | "backlog_get_document"
        | "backlog_create_document" => Toolset::Document,
        "backlog_list_git_repositories"
        | "backlog_list_pull_requests"
        | "backlog_get_pull_request"
        | "backlog_create_pull_request"
        | "backlog_update_pull_request"
        | "backlog_comment_pull_request" => Toolset::Git,
        "backlog_list_notifications" | "backlog_mark_notifications_read" => Toolset::Notifications,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Shared argument parsing
// ---------------------------------------------------------------------------

pub(super) fn string_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn i64_arg(args: &Value, name: &str) -> Option<i64> {
    match args.get(name)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub(super) fn bool_arg(args: &Value, name: &str) -> Option<bool> {
    args.get(name)?.as_bool()
}

/// An issue key as given, or taken from its address
/// (`https://space.backlog.com/view/PROJ-12#comment-3`).
pub(super) fn parse_issue_key(raw: &str) -> Result<String, String> {
    let given = raw.trim();
    let key = match given.split_once("/view/") {
        Some((_, rest)) => rest.split(['#', '?', '/']).next().unwrap_or_default(),
        None => given,
    };
    if key.is_empty() {
        return Err("Name the issue: its key, like PROJ-123".into());
    }
    Ok(key.to_ascii_uppercase())
}

pub(super) fn key_arg(args: &Value) -> Result<String, String> {
    parse_issue_key(args["key"].as_str().unwrap_or_default())
}

/// `null` or an empty string: the field is cleared.
pub(super) fn clears(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty())
}

/// The id of the choice `wanted` names, by name (any case) or id.
pub(super) fn pick(choices: &[FieldChoice], wanted: &Value, field: &str) -> Result<i64, String> {
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
        if names.is_empty() {
            format!("No {field} {wanted} found (none configured on this project)")
        } else {
            format!(
                "No {field} {wanted} on this project; choose one of: {}",
                names.join(", ")
            )
        }
    })
}

/// Resolves a user choice: `"me"`, member name (any case), or user id.
pub(super) fn pick_user(
    users: &[FieldChoice],
    me: i64,
    wanted: &Value,
) -> Result<i64, String> {
    match wanted {
        Value::String(s) if s.trim().eq_ignore_ascii_case("me") => Ok(me),
        other => pick(users, other, "member"),
    }
}

/// Resolves a JSON array (or single value) of choices into ids.
pub(super) fn pick_many(
    choices: &[FieldChoice],
    wanted: &Value,
    field: &str,
) -> Result<Vec<i64>, String> {
    match wanted {
        Value::Array(items) => items
            .iter()
            .map(|item| pick(choices, item, field))
            .collect(),
        Value::Null => Ok(Vec::new()),
        single => Ok(vec![pick(choices, single, field)?]),
    }
}

/// Resolves a JSON array (or single value) of user choices into ids.
pub(super) fn pick_many_users(
    users: &[FieldChoice],
    me: i64,
    wanted: &Value,
) -> Result<Vec<i64>, String> {
    match wanted {
        Value::Array(items) => items.iter().map(|item| pick_user(users, me, item)).collect(),
        Value::Null => Ok(Vec::new()),
        single => Ok(vec![pick_user(users, me, single)?]),
    }
}

pub(super) fn date(value: &Value, field: &str) -> Result<Option<String>, String> {
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

pub(super) fn hours(value: &Value, field: &str) -> Result<Option<f64>, String> {
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

/// Resolves a project by key (any case) or numeric id, falling back to `GET /projects/:idOrKey`
/// for archived projects not in the active catalog.
pub(super) fn resolve_project(raw: &str) -> Result<Project, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Name the project: its key (like PROJ) or numeric id".into());
    }
    if let Ok(project) = super::project(trimmed) {
        return Ok(project);
    }
    let config = require_config()?;
    let seg = super::require_segment(trimmed, "project")?;
    let data = call(&config, None, &format!("/projects/{seg}"), &[], &[])?;
    let id = data
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("No Backlog project {trimmed}"))?;
    Ok(Project {
        id: id.to_string(),
        key: text(&data, "projectKey"),
        name: text(&data, "name"),
    })
}

/// Resolves a numeric issue ID from an integer, numeric string, issue key or Backlog URL.
pub(super) fn resolve_issue_id(config: &Config, wanted: &Value) -> Result<i64, String> {
    match wanted {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| format!("Invalid issue id: {wanted}")),
        Value::String(s) => {
            let trimmed = s.trim();
            if let Ok(id) = trimmed.parse::<i64>() {
                return Ok(id);
            }
            let key = parse_issue_key(trimmed)?;
            let key = super::require_issue_key(&key)?;
            let data = call(config, None, &format!("/issues/{key}"), &[], &[])?;
            data.get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("Backlog returned no id for issue {key}"))
        }
        _ => Err(format!("Expected an issue key, URL or numeric id, got {wanted}")),
    }
}

/// Space-wide user list (`GET /users`), for resolving users when no project is scoped.
pub(super) fn space_users(config: &Config) -> Result<Vec<FieldChoice>, String> {
    let data = call(config, None, "/users", &[], &[])?;
    Ok(parse_named(&data, |_| false))
}

/// Project categories (`GET /projects/:id/categories`).
pub(super) fn project_categories(
    config: &Config,
    project_id: &str,
) -> Result<Vec<FieldChoice>, String> {
    let data = call(
        config,
        None,
        &format!("/projects/{project_id}/categories"),
        &[],
        &[],
    )?;
    Ok(parse_named(&data, |_| false))
}

/// Project custom field definitions (`GET /projects/:id/customFields`).
pub(super) fn project_custom_fields(
    config: &Config,
    project_id: &str,
) -> Result<Vec<Value>, String> {
    let data = call(
        config,
        None,
        &format!("/projects/{project_id}/customFields"),
        &[],
        &[],
    )?;
    Ok(data.as_array().cloned().unwrap_or_default())
}

/// Project members (`GET /projects/:id/users`).
pub(super) fn project_users(
    config: &Config,
    project_id: &str,
) -> Result<Vec<FieldChoice>, String> {
    let data = call(
        config,
        None,
        &format!("/projects/{project_id}/users"),
        &[],
        &[],
    )?;
    Ok(parse_named(&data, |_| false))
}

/// Status choices mapped into `FieldChoice`.
pub(super) fn status_choices(statuses: &[StatusOption]) -> Vec<FieldChoice> {
    statuses
        .iter()
        .map(|s| FieldChoice {
            id: s.id,
            name: s.name.clone(),
        })
        .collect()
}

/// Options plus optional categories/types/custom-fields for rich issue operations.
#[derive(Default)]
pub(super) struct IssueContext {
    pub options: FieldOptions,
    pub statuses: Vec<StatusOption>,
    pub issue_types: Vec<FieldChoice>,
    pub categories: Vec<FieldChoice>,
    pub custom_fields: Vec<Value>,
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

pub(super) fn short_day(iso: &str) -> String {
    iso.get(..10).unwrap_or(iso).to_string()
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Slices `text` by Unicode scalar offset `start` and max length `limit`.
/// Returns `(slice, total_chars, next_start)`.
pub(super) fn slice_chars(text: &str, start: usize, limit: usize) -> (String, usize, Option<usize>) {
    let total = text.chars().count();
    if start >= total {
        return (String::new(), total, None);
    }
    let slice: String = text.chars().skip(start).take(limit).collect();
    let end = start + slice.chars().count();
    let next = (end < total).then_some(end);
    (slice, total, next)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backlog::fields::{Attachment, Change, CustomFieldValue, IssueFields};
    use crate::work_items::{Comment, Details, Thread, WorkItem};

    fn choice(id: i64, name: &str) -> FieldChoice {
        FieldChoice {
            id,
            name: name.into(),
        }
    }

    fn sample_context() -> IssueContext {
        IssueContext {
            options: FieldOptions {
                users: vec![choice(3, "Yumeko"), choice(4, "Ann Lee")],
                milestones: vec![choice(9, "Sprint 04"), choice(10, "v1.2")],
                priorities: vec![choice(2, "High"), choice(3, "Normal")],
                resolutions: vec![choice(0, "Fixed")],
                me: 7,
            },
            statuses: [(1, "Open"), (2, "In Progress"), (4, "Closed")]
                .map(|(id, name)| StatusOption {
                    id,
                    name: name.into(),
                    color: String::new(),
                })
                .into(),
            issue_types: vec![choice(1, "Task"), choice(2, "Bug")],
            categories: vec![choice(5, "Backend"), choice(6, "UI")],
            custom_fields: vec![json!({
                "id": 101,
                "name": "Environment",
                "typeId": 5,
                "items": [
                    { "id": 11, "name": "Staging" },
                    { "id": 12, "name": "Production" }
                ]
            })],
        }
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
        let ctx = sample_context();
        let changes = issues::build_changes(
            &json!({
                "summary": "New title",
                "description": "Details here",
                "issueType": "bug",
                "status": "in progress",
                "assignee": "me",
                "priority": 2,
                "categories": ["backend"],
                "versions": ["v1.2"],
                "milestones": ["sprint 04"],
                "dueDate": "2026-11-20",
                "actualHours": 1.5,
                "notifiedUsers": ["yumeko", "me"],
                "comment": "Moving forward",
                "customFields": [{ "name": "environment", "value": "production" }],
            }),
            &ctx,
            None,
        )
        .unwrap();
        assert_eq!(
            changes,
            [
                Change::Summary("New title".into()),
                Change::Description(Some("Details here".into())),
                Change::IssueType(2),
                Change::Status(2),
                Change::Assignee(Some(7)),
                Change::Priority(2),
                Change::Categories(vec![5]),
                Change::Versions(vec![10]),
                Change::Milestones(vec![9]),
                Change::DueDate(Some("2026-11-20".into())),
                Change::ActualHours(Some(1.5)),
                Change::NotifiedUsers(vec![3, 7]),
                Change::Comment("Moving forward".into()),
                Change::CustomField {
                    id: 101,
                    values: vec!["12".into()],
                    other_value: None,
                },
            ]
        );
    }

    #[test]
    fn null_clears_and_left_out_stays() {
        let ctx = sample_context();
        let changes = issues::build_changes(
            &json!({
                "assignee": null,
                "resolution": "",
                "categories": [],
                "versions": [],
                "milestones": [],
                "startDate": null,
                "description": null,
            }),
            &ctx,
            None,
        )
        .unwrap();
        assert_eq!(
            changes,
            [
                Change::Description(None),
                Change::Assignee(None),
                Change::Categories(Vec::new()),
                Change::Versions(Vec::new()),
                Change::Milestones(Vec::new()),
                Change::Resolution(None),
                Change::StartDate(None),
            ]
        );
        assert!(issues::build_changes(&json!({}), &ctx, None).unwrap().is_empty());
    }

    #[test]
    fn a_name_that_does_not_match_lists_the_choices() {
        let ctx = sample_context();
        let err = issues::build_changes(&json!({ "status": "Done" }), &ctx, None).unwrap_err();
        assert!(err.contains("Open, In Progress, Closed"), "{err}");
        assert!(issues::build_changes(&json!({ "assignee": 99 }), &ctx, None).is_err());
        assert!(issues::build_changes(&json!({ "dueDate": "20/11/2026" }), &ctx, None).is_err());
        assert!(issues::build_changes(&json!({ "estimatedHours": -1 }), &ctx, None).is_err());
        assert!(issues::build_changes(&json!({ "estimatedHours": "2" }), &ctx, None).is_err());
    }

    #[test]
    fn an_issue_reads_as_markdown_with_fields_attachments_and_comments() {
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
                issue_id: 5001,
                assignee: Some(choice(3, "Yumeko")),
                categories: vec![choice(5, "Backend")],
                due_date: Some("2026-11-20".into()),
                attachments: vec![Attachment {
                    id: 77,
                    name: "trace.log".into(),
                    size: 2048,
                }],
                custom_fields: vec![CustomFieldValue {
                    id: 101,
                    name: "Environment".into(),
                    value: "Staging".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let thread = Thread {
            comments: vec![Comment {
                id: "901".into(),
                author: "Yumeko".into(),
                body: "On it.".into(),
                created_at: "2026-11-02T00:00:00Z".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = issues::issue_text(&item, &details, Some(&thread), None);
        assert!(text.starts_with("# PROJ-12 Fix the login\nhttps://acme.backlog.com/view/PROJ-12"));
        assert!(text.contains("ID: 5001"));
        assert!(text.contains("Status: Open"));
        assert!(text.contains("Assignee: Yumeko"));
        assert!(text.contains("Categories: Backend"));
        assert!(text.contains("Due date: 2026-11-20"));
        assert!(text.contains("Environment (#101): Staging"));
        assert!(text.contains("## Attachments"));
        assert!(text.contains("- #77 trace.log (2.0 KB)"));
        assert!(!text.contains("Milestones"));
        assert!(text.contains("Created by: Ann Lee\nCreated: 2026-11-01"));
        assert!(text.contains("## Description\n\nIt fails."));
        assert!(text.contains("### Yumeko · 2026-11-02 · #901\n\nOn it."));
        assert!(!issues::issue_text(&item, &details, None, None).contains("## Comments"));
    }

    #[test]
    fn a_new_issue_needs_a_project_and_a_subject_and_takes_no_status() {
        assert!(issues::create_issue(&json!({ "summary": "Fix it" })).is_err());
        assert!(issues::create_issue(&json!({ "project": "PROJ", "summary": " " })).is_err());
        let err = issues::create_issue(
            &json!({ "project": "PROJ", "summary": "Fix it", "status": "Open" }),
        )
        .unwrap_err();
        assert!(err.contains("backlog_update_issue"), "{err}");
    }

    #[test]
    fn tools_are_listed_with_schemas_and_toolset_filtering_works() {
        let full = Backlog {
            enabled: EnabledToolsets::all(),
        };
        let reply = server::handle(&full, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 30, "expected >= 30 tools, got {}", tools.len());
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert!(
            tools
                .iter()
                .all(|t| t["name"].as_str().unwrap().starts_with("backlog_"))
        );
        // Every schema tool has a matching toolset mapping.
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(
                tool_toolset(name).is_some(),
                "missing tool_toolset mapping for {name}"
            );
        }

        // Filtered server exposes only the selected toolsets and rejects others.
        let wiki_only = Backlog {
            enabled: EnabledToolsets::from_spec("wiki"),
        };
        let filtered = server::handle(
            &wiki_only,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .unwrap();
        let names: Vec<&str> = filtered["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "backlog_list_wikis",
                "backlog_get_wiki",
                "backlog_create_wiki",
                "backlog_update_wiki",
            ]
        );
        let denied = server::handle(
            &wiki_only,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"backlog_list_projects","arguments":{}}}"#,
        )
        .unwrap();
        assert_eq!(denied["result"]["isError"], true);
    }

    #[test]
    fn slice_chars_paginates_unicode_safely() {
        let (part1, total, next1) = slice_chars("héllo世界", 0, 5);
        assert_eq!((part1.as_str(), total, next1), ("héllo", 7, Some(5)));
        let (part2, _, next2) = slice_chars("héllo世界", 5, 5);
        assert_eq!((part2.as_str(), next2), ("世界", None));
    }

    #[test]
    fn document_tree_renders_nested_hierarchy() {
        let data = json!({
            "projectId": 12,
            "activeTree": {
                "id": "Active",
                "children": [
                    {
                        "id": "doc-1",
                        "name": "Architecture",
                        "emoji": "📐",
                        "children": [
                            { "id": "doc-2", "name": "API Spec", "children": [] }
                        ]
                    }
                ]
            }
        });
        let rendered = wiki_docs::format_document_tree(&data, false);
        assert!(rendered.contains("- 📐 Architecture (`doc-1`)"), "{rendered}");
        assert!(rendered.contains("  - API Spec (`doc-2`)"), "{rendered}");
    }

    #[test]
    fn custom_field_filters_encode_backlog_syntax() {
        let mut query = Vec::new();
        issues::append_custom_field_filters(
            &mut query,
            &[
                json!({ "id": 1, "type": "text", "value": "hello" }),
                json!({ "id": 2, "type": "numeric", "min": 1, "max": 5 }),
                json!({ "id": 3, "type": "date", "min": "2026-01-01" }),
                json!({ "id": 4, "type": "list", "value": [10, 20] }),
            ],
        )
        .unwrap();
        assert_eq!(
            query,
            [
                ("customField_1".into(), "hello".into()),
                ("customField_2_min".into(), "1".into()),
                ("customField_2_max".into(), "5".into()),
                ("customField_3_min".into(), "2026-01-01".into()),
                ("customField_4[0]".into(), "10".into()),
                ("customField_4[1]".into(), "20".into()),
            ]
        );
    }

    #[test]
    fn pr_statuses_and_notifications_and_segments_validate() {
        assert_eq!(git_prs::parse_pr_status(&json!("open")).unwrap(), 1);
        assert_eq!(git_prs::parse_pr_status(&json!("Closed")).unwrap(), 2);
        assert_eq!(git_prs::parse_pr_status(&json!(3)).unwrap(), 3);
        assert!(git_prs::parse_pr_status(&json!("draft")).is_err());

        let line = activity::format_notification(&json!({
            "id": 42,
            "resourceAlreadyRead": false,
            "reason": 2,
            "created": "2026-11-20T09:00:00Z",
            "sender": { "name": "Aki" },
            "issue": { "issueKey": "WEB-7", "summary": "Fix cache" },
            "comment": { "content": "Looks good!\nSecond line" }
        }));
        assert_eq!(
            line,
            "- ● #42 [2026-11-20] Aki (commented): WEB-7 Fix cache — \"Looks good!\""
        );

        assert_eq!(super::super::require_segment("my-repo", "repo").unwrap(), "my-repo");
        assert!(super::super::require_segment("../etc", "repo").is_err());
        assert!(super::super::require_segment("a/b", "repo").is_err());
    }
}
