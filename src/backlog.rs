//! Nulab Backlog as an Inbox source, over its REST API v2 with a personal
//! API key (shaped after MonoCode's Jira source, `src-tauri/src/jira.rs`):
//! the connection, the space's projects and statuses, open issues, one
//! issue's description and comments, a new comment and a status change.
//!
//! The space and key live in `backlog-config.json` (owner-only) in
//! BenCode's data folder. Every call here blocks on `curl`; use the
//! background executor. The key travels on curl's stdin and is never part
//! of an error message.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rate_limits::http;
use crate::work_items::{Comment, Details, Kind, Label, Provider, Thread, WorkItem};

const CONFIG_FILE: &str = "backlog-config.json";
const TIMEOUT: Duration = Duration::from_secs(20);
/// Backlog's largest page.
const PAGE: usize = 100;
/// Backlog's built-in "Closed" status; a project cannot replace it.
const CLOSED_STATUS: i64 = 4;
/// How long the space's projects and statuses are trusted.
const CATALOG_FRESH_FOR: Duration = Duration::from_secs(600);

/// What is saved to connect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    /// The space's host, like `yourspace.backlog.com`.
    space: String,
    api_key: String,
    /// The key's user, for "assigned to me".
    user_id: i64,
    user_name: String,
}

/// The connection as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub space: String,
    pub user_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// One status of a project's workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusOption {
    pub id: i64,
    pub name: String,
    pub color: String,
}

/// A project id's statuses, in the workflow's order.
pub type Statuses = HashMap<String, Vec<StatusOption>>;

#[derive(Clone, Debug, Default)]
struct Catalog {
    projects: Vec<Project>,
    statuses: Statuses,
}

/// What one fetch of the Inbox found.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub items: Vec<WorkItem>,
    pub errors: Vec<String>,
    pub statuses: Statuses,
}

/// The space's projects and statuses, kept between Inbox polls.
static CATALOG: Mutex<Option<(Instant, String, Catalog)>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// The saved connection
// ---------------------------------------------------------------------------

fn config_path() -> Result<PathBuf, String> {
    crate::storage::data_dir()
        .map(|dir| dir.join(CONFIG_FILE))
        .ok_or_else(|| "BenCode has no data folder".to_string())
}

fn read_config() -> Result<Option<Config>, String> {
    match std::fs::read_to_string(config_path()?) {
        Ok(raw) => {
            let config: Config =
                serde_json::from_str(&raw).map_err(|_| "Backlog settings are invalid".to_string())?;
            Ok((!config.api_key.trim().is_empty()).then_some(config))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

fn require_config() -> Result<Config, String> {
    read_config()?.ok_or_else(|| "Connect Backlog in Settings".to_string())
}

/// Written for the owner only: the file holds the API key.
fn write_config(config: &Config) -> Result<(), String> {
    use std::io::Write;
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let json = serde_json::to_string(config).map_err(|err| err.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|err| err.to_string())?;
    file.write_all(json.as_bytes()).map_err(|err| err.to_string())
}

fn forget_catalog() {
    match CATALOG.lock() {
        Ok(mut catalog) => *catalog = None,
        Err(err) => log::warn!("backlog catalog lock poisoned: {err}"),
    }
}

/// The saved connection, if any.
pub fn account() -> Option<Account> {
    match read_config() {
        Ok(config) => config.map(|config| Account {
            space: config.space,
            user_name: config.user_name,
        }),
        Err(err) => {
            log::warn!("could not read the Backlog connection: {err}");
            None
        }
    }
}

/// Checks the key against the space and saves the connection.
pub fn connect(space: &str, api_key: &str) -> Result<Account, String> {
    let space = normalize_space(space)?;
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("Enter your Backlog API key".into());
    }
    let mut config = Config {
        space,
        api_key,
        user_id: 0,
        user_name: String::new(),
    };
    let me = call(&config, None, "/users/myself", &[], &[])?;
    config.user_id = me.get("id").and_then(Value::as_i64).unwrap_or(0);
    if config.user_id == 0 {
        return Err("Backlog did not return the current user".into());
    }
    config.user_name = text(&me, "name");
    write_config(&config)?;
    forget_catalog();
    Ok(Account {
        space: config.space,
        user_name: config.user_name,
    })
}

/// Forgets the connection and its key.
pub fn disconnect() -> Result<(), String> {
    forget_catalog();
    match std::fs::remove_file(config_path()?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// A space's host from what the user typed: a host or its URL.
fn normalize_space(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let host = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = host.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let valid = host.contains('.')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if valid {
        Ok(host)
    } else {
        Err("Enter your space's address, like yourspace.backlog.com".into())
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Percent-encodes a query value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The request's URL. `name[]` parameters keep their brackets encoded.
fn api_url(config: &Config, path: &str, query: &[(&str, String)]) -> String {
    let mut url = format!(
        "https://{}/api/v2{path}?apiKey={}",
        config.space,
        encode(&config.api_key)
    );
    for (name, value) in query {
        url.push('&');
        url.push_str(&encode(name));
        url.push('=');
        url.push_str(&encode(value));
    }
    url
}

fn call(
    config: &Config,
    method: Option<&str>,
    path: &str,
    query: &[(&str, String)],
    form: &[(&str, &str)],
) -> Result<Value, String> {
    let url = api_url(config, path, query);
    let response = http::send(&url, &[], http::Send { method, form }, TIMEOUT)
        // curl names the host at most, never the query.
        .map_err(|err| format!("Could not reach Backlog: {err}"))?;
    if (200..300).contains(&response.status) {
        if response.body.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&response.body)
            .map_err(|_| "Backlog sent an unreadable answer".to_string());
    }
    Err(http_error(response.status, &response.body))
}

/// Backlog's own message when it sent one, else what the status means.
fn http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("errors")?
                .as_array()?
                .first()?
                .get("message")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|message| !message.trim().is_empty());
    match (status, message) {
        (401, _) => "Backlog rejected the API key".to_string(),
        (429, _) => "Backlog's rate limit was reached; try again shortly".to_string(),
        (_, Some(message)) => format!("Backlog: {message}"),
        (403, None) => "Backlog denied access".to_string(),
        (404, None) => "Not found on Backlog".to_string(),
        (status, None) => format!("Backlog answered {status}"),
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// An issue key as Backlog writes it (`PROJ_1-42`), safe in a URL path.
fn require_issue_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    let valid = key.rsplit_once('-').is_some_and(|(project, number)| {
        !project.is_empty()
            && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if valid {
        Ok(key)
    } else {
        Err(format!("Invalid Backlog issue key: {key}"))
    }
}

// ---------------------------------------------------------------------------
// Projects and statuses
// ---------------------------------------------------------------------------

fn parse_projects(data: &Value) -> Vec<Project> {
    let mut projects: Vec<Project> = data
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row.get("id")?.as_i64()?;
            Some(Project {
                id: id.to_string(),
                key: text(row, "projectKey"),
                name: text(row, "name"),
            })
        })
        .collect();
    projects.sort_by_key(|project| project.name.to_lowercase());
    projects
}

fn parse_statuses(data: &Value) -> Vec<StatusOption> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some(StatusOption {
                id: row.get("id")?.as_i64()?,
                name: text(row, "name"),
                color: text(row, "color"),
            })
        })
        .collect()
}

/// The space's active projects and each one's statuses, from the cache
/// while it is fresh.
fn catalog(config: &Config) -> Result<Catalog, String> {
    if let Ok(cached) = CATALOG.lock()
        && let Some((at, space, catalog)) = cached.as_ref()
        && *space == config.space
        && at.elapsed() < CATALOG_FRESH_FOR
    {
        return Ok(catalog.clone());
    }
    let projects = parse_projects(&call(
        config,
        None,
        "/projects",
        &[("archived", "false".to_string())],
        &[],
    )?);
    // One request per project, side by side.
    let statuses: Statuses = std::thread::scope(|scope| {
        let handles: Vec<_> = projects
            .iter()
            .map(|project| {
                scope.spawn(move || {
                    let path = format!("/projects/{}/statuses", project.id);
                    (project.id.clone(), call(config, None, &path, &[], &[]))
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .filter_map(|(id, result)| match result {
                Ok(data) => Some((id, parse_statuses(&data))),
                Err(err) => {
                    log::warn!("backlog: no statuses for project {id}: {err}");
                    None
                }
            })
            .collect()
    });
    let catalog = Catalog { projects, statuses };
    match CATALOG.lock() {
        Ok(mut cached) => *cached = Some((Instant::now(), config.space.clone(), catalog.clone())),
        Err(err) => log::warn!("backlog catalog lock poisoned: {err}"),
    }
    Ok(catalog)
}

/// The space's active projects, for Settings.
pub fn projects() -> Result<Vec<Project>, String> {
    Ok(catalog(&require_config()?)?.projects)
}

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Named {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueRow {
    #[serde(default)]
    project_id: i64,
    issue_key: String,
    #[serde(default)]
    key_id: i64,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    issue_type: Option<Named>,
    #[serde(default)]
    priority: Option<Named>,
    #[serde(default)]
    status: Option<Named>,
    #[serde(default)]
    assignee: Option<Named>,
    #[serde(default)]
    category: Vec<Named>,
    #[serde(default)]
    milestone: Vec<Named>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    created_user: Option<Named>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

fn work_item(row: &IssueRow, space: &str) -> WorkItem {
    let status = row.status.as_ref();
    let plain = |named: &Named| Label {
        name: named.name.clone(),
        color: String::new(),
    };
    let labels = row
        .issue_type
        .iter()
        .map(|kind| Label {
            name: kind.name.clone(),
            color: kind.color.clone().unwrap_or_default(),
        })
        .chain(row.category.iter().map(plain))
        .chain(row.milestone.iter().map(plain))
        .filter(|label| !label.name.trim().is_empty())
        .collect();
    WorkItem {
        provider: Provider::Backlog,
        kind: Kind::Issue,
        identifier: row.issue_key.clone(),
        number: row.key_id,
        title: row.summary.clone(),
        url: format!("https://{space}/view/{}", row.issue_key),
        state: status.map(|s| s.name.clone()).unwrap_or_default(),
        state_color: status.and_then(|s| s.color.clone()).unwrap_or_default(),
        closed: status.is_some_and(|s| s.id == CLOSED_STATUS),
        created_at: row.created.clone().unwrap_or_default(),
        updated_at: row.updated.clone().unwrap_or_default(),
        labels,
        assignees: row
            .assignee
            .iter()
            .map(|a| a.name.clone())
            .filter(|name| !name.is_empty())
            .collect(),
        priority: row.priority.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
        // `2024-05-31T00:00:00Z`: the day is all Backlog means.
        due_date: row
            .due_date
            .as_deref()
            .map(|date| date.chars().take(10).collect())
            .unwrap_or_default(),
        repo: row
            .issue_key
            .rsplit_once('-')
            .map_or_else(|| row.issue_key.clone(), |(project, _)| project.to_string()),
        container_id: row.project_id.to_string(),
        ..Default::default()
    }
}

fn parse_issues(data: Value, space: &str) -> Result<Vec<WorkItem>, String> {
    let rows: Vec<IssueRow> =
        serde_json::from_value(data).map_err(|_| "Backlog sent unreadable issues".to_string())?;
    Ok(rows.iter().map(|row| work_item(row, space)).collect())
}

/// The issue list's query: the shown projects, every status but Closed,
/// most recently updated first.
fn issue_query(
    catalog: &Catalog,
    hidden: &[String],
    assignee: Option<i64>,
) -> Option<Vec<(&'static str, String)>> {
    let shown: Vec<&Project> = catalog
        .projects
        .iter()
        .filter(|project| !hidden.contains(&project.id))
        .collect();
    if shown.is_empty() {
        return None;
    }
    let mut query: Vec<(&'static str, String)> = shown
        .iter()
        .map(|project| ("projectId[]", project.id.clone()))
        .collect();
    let mut open: Vec<i64> = shown
        .iter()
        .filter_map(|project| catalog.statuses.get(&project.id))
        .flatten()
        .map(|status| status.id)
        .filter(|id| *id != CLOSED_STATUS)
        .collect();
    open.sort_unstable();
    open.dedup();
    query.extend(open.into_iter().map(|id| ("statusId[]", id.to_string())));
    if let Some(user) = assignee {
        query.push(("assigneeId[]", user.to_string()));
    }
    query.push(("sort", "updated".to_string()));
    query.push(("order", "desc".to_string()));
    query.push(("count", PAGE.to_string()));
    Some(query)
}

/// The open issues of the projects not in `hidden`; `None` while Backlog
/// is not connected.
pub fn inbox(hidden: &[String], assigned_to_me: bool) -> Option<Listing> {
    let config = match read_config() {
        Ok(config) => config?,
        Err(err) => {
            return Some(Listing {
                errors: vec![err],
                ..Default::default()
            });
        }
    };
    let mut listing = Listing::default();
    let catalog = match catalog(&config) {
        Ok(catalog) => catalog,
        Err(err) => {
            listing.errors.push(err);
            return Some(listing);
        }
    };
    listing.statuses = catalog.statuses.clone();
    let Some(query) = issue_query(&catalog, hidden, assigned_to_me.then_some(config.user_id)) else {
        return Some(listing);
    };
    match call(&config, None, "/issues", &query, &[]).and_then(|data| parse_issues(data, &config.space)) {
        Ok(items) => listing.items = items,
        Err(err) => {
            log::warn!("inbox: could not list Backlog issues: {err}");
            listing.errors.push(err);
        }
    }
    Some(listing)
}

/// The issue's description and who opened it.
pub fn details(key: &str) -> Result<Details, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let data = call(&config, None, &format!("/issues/{key}"), &[], &[])?;
    let row: IssueRow =
        serde_json::from_value(data).map_err(|_| "Backlog sent an unreadable issue".to_string())?;
    Ok(Details {
        body: row.description.unwrap_or_default(),
        author: row.created_user.map(|user| user.name).unwrap_or_default(),
        ..Default::default()
    })
}

/// Moves the issue to `status_id` and returns it as it is now.
pub fn set_status(key: &str, status_id: i64) -> Result<WorkItem, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let status = status_id.to_string();
    let data = call(
        &config,
        Some("PATCH"),
        &format!("/issues/{key}"),
        &[],
        &[("statusId", &status)],
    )?;
    let row: IssueRow =
        serde_json::from_value(data).map_err(|_| "Backlog sent an unreadable issue".to_string())?;
    Ok(work_item(&row, &config.space))
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// What a change-log entry's `field` reads as.
fn change_label(field: &str) -> &str {
    match field {
        "status" => "Status",
        "assigner" => "Assignee",
        "limitDate" => "Due date",
        "startDate" => "Start date",
        "priority" => "Priority",
        "milestone" => "Milestone",
        "version" => "Version",
        "component" => "Category",
        "issueType" => "Issue type",
        "resolution" => "Resolution",
        "summary" => "Subject",
        "description" => "Description",
        "estimatedHours" => "Estimated hours",
        "actualHours" => "Actual hours",
        "attachment" => "Attachment",
        "parentIssue" => "Parent issue",
        other => other,
    }
}

/// One change as a line of Markdown: "**Status**: Open → In Progress".
fn change_line(change: &Value) -> Option<String> {
    let field = change.get("field")?.as_str()?;
    let label = change_label(field);
    if field == "description" {
        return Some(format!("**{label}** changed"));
    }
    let side = |key: &str| {
        change
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some(match (side("originalValue"), side("newValue")) {
        (Some(old), Some(new)) => format!("**{label}**: {old} → {new}"),
        (None, Some(new)) => format!("**{label}**: {new}"),
        (Some(old), None) => format!("**{label}**: {old} removed"),
        (None, None) => return None,
    })
}

fn parse_comment(row: &Value, space: &str, key: &str) -> Option<Comment> {
    let id = row.get("id")?.as_i64()?;
    let content = row.get("content").and_then(Value::as_str).unwrap_or("").trim();
    let changes: Vec<String> = row
        .get("changeLog")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(change_line)
        .collect();
    let body = match (content.is_empty(), changes.is_empty()) {
        // A notification with nothing to read.
        (true, true) => return None,
        (false, true) => content.to_string(),
        // Two trailing spaces: each change keeps its own line.
        (true, false) => changes.join("  \n"),
        (false, false) => format!("{}\n\n{content}", changes.join("  \n")),
    };
    Some(Comment {
        id: id.to_string(),
        kind: "comment".into(),
        author: row
            .get("createdUser")
            .map(|user| text(user, "name"))
            .unwrap_or_default(),
        body,
        created_at: text(row, "created"),
        url: format!("https://{space}/view/{key}#comment-{id}"),
        ..Default::default()
    })
}

/// Comments arrive newest first; the conversation reads oldest first.
fn parse_thread(data: &Value, space: &str, key: &str) -> Thread {
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut comments: Vec<Comment> = rows
        .iter()
        .filter_map(|row| parse_comment(row, space, key))
        .collect();
    comments.reverse();
    Thread {
        comments,
        commits: Vec::new(),
        truncated: rows.len() >= PAGE,
    }
}

/// The issue's latest comments and changes.
pub fn thread(key: &str) -> Result<Thread, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let data = call(
        &config,
        None,
        &format!("/issues/{key}/comments"),
        &[("count", PAGE.to_string()), ("order", "desc".to_string())],
        &[],
    )?;
    Ok(parse_thread(&data, &config.space, key))
}

/// Adds a comment; returns its URL.
pub fn post_comment(key: &str, body: &str) -> Result<String, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    let data = call(
        &config,
        None,
        &format!("/issues/{key}/comments"),
        &[],
        &[("content", body)],
    )?;
    let id = data
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "Could not post the Backlog comment".to_string())?;
    Ok(format!("https://{}/view/{key}#comment-{id}", config.space))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> Config {
        Config {
            space: "acme.backlog.com".into(),
            api_key: "k&y".into(),
            user_id: 9,
            user_name: "Me".into(),
        }
    }

    #[test]
    fn spaces_are_hosts_whatever_was_pasted() {
        assert_eq!(normalize_space(" https://Acme.backlog.com/projects/X ").unwrap(), "acme.backlog.com");
        assert_eq!(normalize_space("acme.backlog.jp").unwrap(), "acme.backlog.jp");
        assert!(normalize_space("acme").is_err());
        assert!(normalize_space("acme.backlog.com:evil@x").is_err());
        assert!(normalize_space("").is_err());
    }

    #[test]
    fn urls_encode_the_key_and_array_parameters() {
        let url = api_url(&config(), "/issues", &[("projectId[]", "12".into()), ("sort", "updated".into())]);
        assert_eq!(
            url,
            "https://acme.backlog.com/api/v2/issues?apiKey=k%26y&projectId%5B%5D=12&sort=updated"
        );
    }

    #[test]
    fn issue_keys_are_checked_before_they_reach_a_path() {
        assert_eq!(require_issue_key(" WEB_APP-12 ").unwrap(), "WEB_APP-12");
        assert!(require_issue_key("WEB-").is_err());
        assert!(require_issue_key("../users-1").is_err());
        assert!(require_issue_key("12").is_err());
    }

    #[test]
    fn errors_prefer_backlogs_message_and_never_echo_the_key() {
        let body = r#"{"errors":[{"message":"No such issue.","code":6,"moreInfo":""}]}"#;
        assert_eq!(http_error(404, body), "Backlog: No such issue.");
        assert_eq!(http_error(401, body), "Backlog rejected the API key");
        assert_eq!(http_error(500, "<html>"), "Backlog answered 500");
    }

    fn issue() -> Value {
        json!({
            "id": 1001,
            "projectId": 12,
            "issueKey": "WEB-42",
            "keyId": 42,
            "issueType": { "id": 2, "name": "Bug", "color": "#990000" },
            "summary": "Login fails",
            "description": "Steps…",
            "priority": { "id": 2, "name": "High" },
            "status": { "id": 2, "name": "In Progress", "color": "#4488c5" },
            "assignee": { "id": 9, "name": "Me" },
            "category": [{ "id": 1, "name": "Auth" }],
            "milestone": [],
            "dueDate": "2026-10-31T00:00:00Z",
            "createdUser": { "id": 3, "name": "Aki" },
            "created": "2026-10-01T02:00:00Z",
            "updated": "2026-10-08T03:00:00Z"
        })
    }

    #[test]
    fn issues_become_work_items() {
        let items = parse_issues(json!([issue()]), "acme.backlog.com").unwrap();
        let item = &items[0];
        assert_eq!(item.provider, Provider::Backlog);
        assert_eq!((item.identifier.as_str(), item.number), ("WEB-42", 42));
        assert_eq!((item.repo.as_str(), item.container_id.as_str()), ("WEB", "12"));
        assert_eq!(item.url, "https://acme.backlog.com/view/WEB-42");
        assert_eq!((item.state.as_str(), item.state_color.as_str()), ("In Progress", "#4488c5"));
        assert!(!item.closed);
        assert_eq!((item.priority.as_str(), item.due_date.as_str()), ("High", "2026-10-31"));
        assert_eq!(item.assignees, ["Me"]);
        let labels: Vec<&str> = item.labels.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(labels, ["Bug", "Auth"]);
        assert_eq!(item.key(), "backlog:web-42");
    }

    #[test]
    fn sparse_issues_still_parse() {
        let items = parse_issues(
            json!([{ "issueKey": "A_B-1", "status": { "id": 4, "name": "Closed" }, "assignee": null, "dueDate": null }]),
            "s.backlog.jp",
        )
        .unwrap();
        assert!(items[0].closed);
        assert_eq!(items[0].repo, "A_B");
        assert!(items[0].assignees.is_empty() && items[0].due_date.is_empty());
    }

    #[test]
    fn the_list_asks_for_shown_projects_and_open_statuses() {
        let project = |id: &str| Project {
            id: id.into(),
            key: id.into(),
            name: id.into(),
        };
        let status = |id: i64| StatusOption {
            id,
            name: String::new(),
            color: String::new(),
        };
        let catalog = Catalog {
            projects: vec![project("1"), project("2")],
            statuses: HashMap::from([
                ("1".to_string(), vec![status(1), status(2), status(4)]),
                ("2".to_string(), vec![status(1), status(77), status(4)]),
            ]),
        };
        let query = issue_query(&catalog, &["2".to_string()], Some(9)).unwrap();
        let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("projectId[]", "1"),
                ("statusId[]", "1"),
                ("statusId[]", "2"),
                ("assigneeId[]", "9"),
                ("sort", "updated"),
                ("order", "desc"),
                ("count", "100"),
            ]
        );
        let everything_hidden = issue_query(&catalog, &["1".to_string(), "2".to_string()], None);
        assert!(everything_hidden.is_none());
    }

    #[test]
    fn threads_read_oldest_first_with_changes_as_text() {
        let data = json!([
            {
                "id": 3, "content": null, "created": "2026-10-03T00:00:00Z",
                "createdUser": { "name": "Aki" },
                "changeLog": [
                    { "field": "status", "originalValue": "Open", "newValue": "In Progress" },
                    { "field": "assigner", "originalValue": null, "newValue": "Me" }
                ]
            },
            { "id": 2, "content": null, "changeLog": [], "createdUser": { "name": "Bot" } },
            { "id": 1, "content": " First ", "created": "2026-10-01T00:00:00Z", "createdUser": { "name": "Me" } }
        ]);
        let thread = parse_thread(&data, "acme.backlog.com", "WEB-42");
        assert!(!thread.truncated);
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[0].body, "First");
        assert_eq!(thread.comments[0].url, "https://acme.backlog.com/view/WEB-42#comment-1");
        assert_eq!(
            thread.comments[1].body,
            "**Status**: Open → In Progress  \n**Assignee**: Me"
        );
        assert_eq!(thread.comments[1].author, "Aki");
    }

    #[test]
    fn projects_sort_by_name_and_statuses_keep_their_order() {
        let projects = parse_projects(&json!([
            { "id": 2, "projectKey": "ZED", "name": "zed" },
            { "id": 1, "projectKey": "APP", "name": "App" }
        ]));
        assert_eq!(projects[0].key, "APP");
        let statuses = parse_statuses(&json!([
            { "id": 1, "name": "Open", "color": "#ed8077" },
            { "id": 4, "name": "Closed", "color": "#b0be3c" }
        ]));
        assert_eq!(statuses[1], StatusOption { id: 4, name: "Closed".into(), color: "#b0be3c".into() });
    }
}
