//! Jira Cloud as an Inbox source, over its REST API v3 with an Atlassian
//! email and API token (ported from MonoCode `src-tauri/src/jira.rs`, shaped
//! alongside `backlog.rs`): the connection, the site's projects, open issues
//! via JQL, issue details and comments (with ADF ↔ Markdown conversion in
//! `jira/adf.rs`), workflow status transitions, and the `bencode-jira` MCP
//! server (`jira/mcp.rs`).
//!
//! The site, email and token live in `jira-config.json` (owner-only) in
//! BenCode's data folder. Every call here blocks on `curl` through
//! `rate_limits::http`; use the background executor. The Authorization header
//! travels on curl's stdin and is never part of an error message.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::rate_limits::http;
use crate::work_items::{Comment, Details, Kind, Label, Provider, Thread, WorkItem};

pub mod adf;
pub mod mcp;

const CONFIG_FILE: &str = "jira-config.json";
const TIMEOUT: Duration = Duration::from_secs(20);
/// Issues per Inbox search.
const INBOX_LIMIT: u32 = 100;
/// Latest comments fetched per issue.
const COMMENT_LIMIT: u32 = 50;
/// How long the site's project list is trusted.
const CATALOG_FRESH_FOR: Duration = Duration::from_secs(600);

const ISSUE_FIELDS: [&str; 10] = [
    "summary",
    "status",
    "created",
    "updated",
    "labels",
    "assignee",
    "project",
    "priority",
    "duedate",
    "issuetype",
];

/// What is saved to connect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    /// The site's host (lowercase, optional `:port`, without `https://`),
    /// like `yourteam.atlassian.net`.
    pub site: String,
    pub email: String,
    pub token: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub display_name: String,
}

/// The connection as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub site: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// One workflow transition available for an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub id: String,
    pub name: String,
    /// Target status name (e.g. "In Progress").
    pub to_status: String,
    /// Target `statusCategory.key` (`new`, `indeterminate`, `done`).
    pub category: String,
}

/// What one fetch of the Inbox found.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub items: Vec<WorkItem>,
    pub errors: Vec<String>,
    /// The site's project keys, listed or hidden: pull requests may name
    /// any of their issues (`work_items/tickets.rs`).
    pub projects: Vec<String>,
    /// The connected user's display name, for "Needs you" (`assigned_to_me`).
    pub me: String,
}

/// The site's projects, kept between Inbox polls.
static CATALOG: Mutex<Option<(Instant, String, Vec<Project>)>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// The saved connection
// ---------------------------------------------------------------------------

fn config_path() -> Result<PathBuf, String> {
    crate::storage::data_dir()
        .map(|dir| dir.join(CONFIG_FILE))
        .ok_or_else(|| "BenCode has no data folder".to_string())
}

pub(crate) fn read_config() -> Result<Option<Config>, String> {
    match std::fs::read_to_string(config_path()?) {
        Ok(raw) => {
            let mut config: Config =
                serde_json::from_str(&raw).map_err(|_| "Jira settings are invalid".to_string())?;
            config.site = normalize_site(&config.site)?;
            config.email = config.email.trim().to_string();
            config.token = config.token.trim().to_string();
            if config.token.is_empty() || config.email.is_empty() {
                Ok(None)
            } else {
                Ok(Some(config))
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

pub(crate) fn require_config() -> Result<Config, String> {
    read_config()?.ok_or_else(|| "Connect Jira in Settings".to_string())
}

/// Written for the owner only: the file holds the API token.
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
    file.write_all(json.as_bytes())
        .map_err(|err| err.to_string())
}

fn forget_catalog() {
    match CATALOG.lock() {
        Ok(mut catalog) => *catalog = None,
        Err(err) => log::warn!("jira catalog lock poisoned: {err}"),
    }
}

/// The saved connection, if any.
pub fn account() -> Option<Account> {
    match read_config() {
        Ok(config) => config.map(|config| Account {
            site: config.site,
            email: config.email,
            display_name: config.display_name,
        }),
        Err(err) => {
            log::warn!("could not read the Jira connection: {err}");
            None
        }
    }
}

/// Checks the credentials against `/rest/api/3/myself` and saves the connection.
pub fn connect(site: &str, email: &str, token: &str) -> Result<Account, String> {
    let site = normalize_site(site)?;
    let email = email.trim().to_string();
    if email.is_empty() || !email.contains('@') {
        return Err("Enter the email address of your Atlassian account".into());
    }
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Enter your Jira API token".into());
    }
    let mut config = Config {
        site,
        email,
        token,
        account_id: String::new(),
        display_name: String::new(),
    };
    let me = call_json(&config, None, "/rest/api/3/myself", None)?;
    config.account_id = string_field(&me, "accountId").unwrap_or_default();
    if config.account_id.is_empty() {
        return Err("Jira did not return the current user".into());
    }
    config.display_name = string_field(&me, "displayName").unwrap_or_default();
    write_config(&config)?;
    forget_catalog();
    Ok(Account {
        site: config.site,
        email: config.email,
        display_name: config.display_name,
    })
}

/// Forgets the connection and its token.
pub fn disconnect() -> Result<(), String> {
    forget_catalog();
    match std::fs::remove_file(config_path()?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// Normalizes what the user typed into a lowercase HTTPS host (with optional `:port`).
/// Bare `yourteam` becomes `yourteam.atlassian.net`; `http://` and credentials are rejected.
pub fn normalize_site(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Enter your Jira site, like yourteam.atlassian.net".into());
    }
    let rest = if let Some((scheme, after)) = trimmed.split_once("://") {
        if !scheme.eq_ignore_ascii_case("https") {
            return Err("Jira site must use HTTPS".into());
        }
        after
    } else {
        trimmed
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err("Enter your Jira site, like yourteam.atlassian.net".into());
    }
    let (host_part, port_part) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    let host_lower = host_part.to_ascii_lowercase();
    let host = if host_lower.contains('.') {
        host_lower
    } else {
        format!("{host_lower}.atlassian.net")
    };
    let host_ok = host.contains('.')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && !host.contains("..")
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-'));
    if !host_ok {
        return Err("Enter your Jira site, like yourteam.atlassian.net".into());
    }
    if let Some(port) = port_part {
        let port_ok = !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|p| p > 0);
        if !port_ok {
            return Err("Jira site port is invalid".into());
        }
        Ok(format!("{host}:{port}"))
    } else {
        Ok(host)
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

pub(crate) fn authorization(config: &Config) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!(
        "{}:{}",
        config.email.trim(),
        config.token.trim()
    ));
    format!("Basic {encoded}")
}

/// Percent-encodes a query parameter value.
pub(crate) fn encode(value: &str) -> String {
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

pub(crate) fn call_json(
    config: &Config,
    method: Option<&str>,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let url = format!("https://{}{path}", config.site);
    let auth = authorization(config);
    let payload = body
        .map(serde_json::to_string)
        .transpose()
        .map_err(|err| err.to_string())?;
    let mut headers = vec![
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
    ];
    if payload.is_some() {
        headers.push(("Content-Type", "application/json"));
    }
    let response = http::send(
        &url,
        &headers,
        http::Send {
            method,
            form: &[],
            body: payload.as_deref(),
            follow_redirects: false,
        },
        TIMEOUT,
    )
    .map_err(|err| format!("Could not reach Jira: {err}"))?;
    if (200..300).contains(&response.status) {
        if response.body.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&response.body)
            .map_err(|_| "Jira sent an unreadable answer".to_string());
    }
    Err(http_error(response.status, &response.body))
}

fn http_error(status: u16, body: &str) -> String {
    if status == 401 {
        return "Jira email or API token is invalid".into();
    }
    if status == 429 {
        return "Jira's rate limit was reached; try again shortly".into();
    }
    if let Some(message) = error_message(body) {
        return format!("Jira: {message}");
    }
    match status {
        403 => "Jira denied access. Check the API token's permissions".into(),
        404 => "Not found on Jira".into(),
        _ => format!("Jira answered {status}"),
    }
}

fn error_message(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let from_list = parsed
        .get("errorMessages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.iter().filter_map(Value::as_str).next())
        .map(str::to_string);
    let from_map = || {
        parsed
            .get("errors")
            .and_then(Value::as_object)
            .and_then(|errors| errors.values().filter_map(Value::as_str).next())
            .map(str::to_string)
    };
    from_list
        .or_else(from_map)
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

pub(crate) fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
}

/// Validates a Jira issue key (`ENG-42`, `PROJ_1-7`), safe in a URL path.
pub(crate) fn require_issue_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    let valid = key.len() < 64
        && key.rsplit_once('-').is_some_and(|(project, number)| {
            !project.is_empty()
                && project
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        });
    if valid {
        Ok(key)
    } else {
        Err(format!("Invalid Jira issue key: {key}"))
    }
}

/// Validates a project key or id path segment.
pub(crate) fn require_segment<'a>(raw: &'a str, what: &str) -> Result<&'a str, String> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains("..")
        && !value
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '?' | '#') || c.is_control());
    if valid {
        Ok(value)
    } else {
        Err(format!("Invalid Jira {what}: {value}"))
    }
}

/// Jira sends `+0000` offsets; `jiff::Timestamp` accepts RFC 3339 (`+00:00`).
fn normalize_time(value: &str) -> String {
    let bytes = value.as_bytes();
    let len = bytes.len();
    if len > 5
        && matches!(bytes[len - 5], b'+' | b'-')
        && bytes[len - 4..].iter().all(u8::is_ascii_digit)
    {
        return format!("{}:{}", &value[..len - 2], &value[len - 2..]);
    }
    value.to_string()
}

fn issue_number(key: &str) -> i64 {
    key.rsplit('-')
        .next()
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

fn is_high_priority(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "highest" | "high" | "critical" | "blocker" | "urgent" | "p0" | "p1"
    )
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

fn parse_projects(data: &Value) -> Result<Vec<Project>, String> {
    let values = data
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return projects".to_string())?;
    Ok(values
        .iter()
        .filter_map(|value| {
            let id = string_field(value, "id").filter(|id| !id.is_empty())?;
            let key = string_field(value, "key").unwrap_or_default();
            let name = string_field(value, "name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| key.clone());
            Some(Project { id, key, name })
        })
        .collect())
}

// Follow every page: a partial project list would silently exclude visible
// projects when converting hidden projects into the JQL allowlist.
fn fetch_projects(
    mut fetch: impl FnMut(u64) -> Result<Value, String>,
) -> Result<Vec<Project>, String> {
    let mut start = 0;
    let mut projects = Vec::new();
    loop {
        let data = fetch(start)?;
        let page = parse_projects(&data)?;
        let count = data["values"].as_array().map_or(0, Vec::len) as u64;
        projects.extend(page);
        if data["isLast"].as_bool() == Some(true) {
            break;
        }
        let next = data["startAt"].as_u64().unwrap_or(start) + count;
        if data["total"].as_u64().is_some_and(|total| next >= total) {
            break;
        }
        if count == 0 || next <= start {
            if data["isLast"].as_bool() == Some(false) {
                return Err("Jira project pagination did not advance".into());
            }
            break;
        }
        start = next;
    }
    let mut seen = HashSet::new();
    projects.retain(|project| seen.insert(project.id.clone()));
    projects.sort_by_key(|project| project.name.to_lowercase());
    Ok(projects)
}

fn catalog(config: &Config) -> Result<Vec<Project>, String> {
    if let Ok(cached) = CATALOG.lock()
        && let Some((at, site, projects)) = cached.as_ref()
        && *site == config.site
        && at.elapsed() < CATALOG_FRESH_FOR
    {
        return Ok(projects.clone());
    }
    let projects = fetch_projects(|start| {
        call_json(
            config,
            None,
            &format!("/rest/api/3/project/search?maxResults=100&orderBy=name&startAt={start}"),
            None,
        )
    })?;
    match CATALOG.lock() {
        Ok(mut cached) => *cached = Some((Instant::now(), config.site.clone(), projects.clone())),
        Err(err) => log::warn!("jira catalog lock poisoned: {err}"),
    }
    Ok(projects)
}

/// The site's active projects, for Settings.
pub fn projects() -> Result<Vec<Project>, String> {
    catalog(&require_config()?)
}

/// Forces a fresh project fetch (for Settings refresh).
pub fn refresh_projects() -> Result<Vec<Project>, String> {
    forget_catalog();
    projects()
}

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------

fn parse_issue(node: &Value, site: &str) -> Option<WorkItem> {
    let key = string_field(node, "key").filter(|key| !key.is_empty())?;
    let fields = node.get("fields").unwrap_or(&Value::Null);
    let project = fields.get("project");
    let status = fields.get("status");
    let category = status
        .and_then(|s| s.pointer("/statusCategory/key"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let issue_type = fields
        .get("issuetype")
        .and_then(|it| string_field(it, "name"))
        .filter(|name| !name.is_empty())
        .map(|name| Label {
            name,
            color: String::new(),
        });
    let mut labels: Vec<Label> = fields
        .get("labels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| Label {
                    name: name.to_string(),
                    color: String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(kind) = &issue_type
        && !labels
            .iter()
            .any(|l| l.name.eq_ignore_ascii_case(&kind.name))
    {
        labels.insert(0, kind.clone());
    }
    let assignee = fields
        .get("assignee")
        .and_then(|a| string_field(a, "displayName"))
        .filter(|name| !name.is_empty());
    let priority = fields
        .get("priority")
        .and_then(|p| string_field(p, "name"))
        .unwrap_or_default();
    let high_priority = is_high_priority(&priority);
    let due_date = string_field(fields, "duedate")
        .map(|date| date.chars().take(10).collect())
        .unwrap_or_default();
    let repo = project
        .and_then(|p| string_field(p, "key"))
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| {
            key.rsplit_once('-')
                .map_or_else(|| key.clone(), |(p, _)| p.to_string())
        });
    let container_id = project
        .and_then(|p| string_field(p, "id"))
        .unwrap_or_default();
    Some(WorkItem {
        provider: Provider::Jira,
        kind: Kind::Issue,
        number: issue_number(&key),
        url: format!("https://{site}/browse/{key}"),
        title: string_field(fields, "summary").unwrap_or_default(),
        state: status
            .and_then(|s| string_field(s, "name"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Open".into()),
        // Jira's `statusCategory.key` (`new`, `indeterminate`, `done`):
        // used by `status_mark` to pick a theme token color.
        state_reason: category.clone(),
        closed: category.eq_ignore_ascii_case("done"),
        created_at: normalize_time(&string_field(fields, "created").unwrap_or_default()),
        updated_at: normalize_time(&string_field(fields, "updated").unwrap_or_default()),
        labels,
        issue_type,
        assignees: assignee.into_iter().collect(),
        priority,
        high_priority,
        due_date,
        repo,
        container_id,
        identifier: key,
        ..Default::default()
    })
}

fn parse_issues(data: &Value, site: &str) -> Result<Vec<WorkItem>, String> {
    let issues = data
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return issues".to_string())?;
    Ok(issues
        .iter()
        .filter_map(|node| parse_issue(node, site))
        .collect())
}

/// Builds the Inbox JQL query. `project_ids`: `None` means no project filter;
/// `Some(ids)` filters to those numeric project IDs.
fn inbox_jql(assigned_to_me: bool, project_ids: Option<&[String]>) -> String {
    let mut clauses = Vec::new();
    if assigned_to_me {
        clauses.push("assignee = currentUser()".to_string());
    }
    if let Some(ids) = project_ids {
        let clean: Vec<&str> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()))
            .collect();
        if !clean.is_empty() {
            clauses.push(format!("project in ({})", clean.join(", ")));
        }
    }
    clauses.push("statusCategory != Done".to_string());
    format!("{} ORDER BY updated DESC", clauses.join(" AND "))
}

/// Computes the visible project IDs for JQL: `None` means don't filter by
/// project (either none are hidden or none are known), `Some(vec![])` means
/// every known project is hidden (skip search).
fn visible_project_ids(projects: &[Project], hidden: &[String]) -> Option<Vec<String>> {
    if hidden.is_empty() || projects.is_empty() {
        return None;
    }
    let hidden_set: HashSet<&str> = hidden.iter().map(String::as_str).collect();
    let visible: Vec<String> = projects
        .iter()
        .filter(|p| !hidden_set.contains(p.id.as_str()))
        .map(|p| p.id.clone())
        .collect();
    if visible.len() == projects.len() {
        None
    } else {
        Some(visible)
    }
}

/// Open issues of the projects not in `hidden`; `None` while Jira is not connected.
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
    let mut listing = Listing {
        me: config.display_name.clone(),
        ..Default::default()
    };
    let projects = match catalog(&config) {
        Ok(projects) => projects,
        Err(err) => {
            listing.errors.push(err);
            return Some(listing);
        }
    };
    listing.projects = projects.iter().map(|p| p.key.clone()).collect();
    let project_filter = visible_project_ids(&projects, hidden);
    if matches!(&project_filter, Some(ids) if ids.is_empty()) {
        return Some(listing);
    }
    let jql = inbox_jql(assigned_to_me, project_filter.as_deref());
    let body = json!({
        "jql": jql,
        "maxResults": INBOX_LIMIT,
        "fields": ISSUE_FIELDS,
    });
    match call_json(&config, Some("POST"), "/rest/api/3/search/jql", Some(&body))
        .and_then(|data| parse_issues(&data, &config.site))
    {
        Ok(items) => listing.items = items,
        Err(err) => {
            log::warn!("inbox: could not list Jira issues: {err}");
            listing.errors.push(err);
        }
    }
    Some(listing)
}

fn parse_details(data: &Value) -> Result<Details, String> {
    let fields = data
        .get("fields")
        .ok_or_else(|| "Jira did not return that issue".to_string())?;
    let author = [
        fields.get("reporter"),
        fields.get("creator"),
        fields.get("assignee"),
    ]
    .into_iter()
    .flatten()
    .find_map(|person| string_field(person, "displayName").filter(|n| !n.is_empty()))
    .unwrap_or_default();
    Ok(Details {
        body: adf::rich_text(fields.get("description")),
        author,
        ..Default::default()
    })
}

/// The issue's description and reporter.
pub fn details(key: &str) -> Result<Details, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}?fields=description,reporter,creator,assignee"),
        None,
    )?;
    parse_details(&data)
}

/// One issue as the Inbox lists it, for a thread's link to an issue the
/// list does not hold; only on the connected site (`host`).
pub fn issue(key: &str, host: &str) -> Result<WorkItem, String> {
    let config = require_config()?;
    if !config.site.eq_ignore_ascii_case(host) {
        return Err(format!("{host} is not the connected Jira site"));
    }
    let key = require_issue_key(key)?;
    let fields = ISSUE_FIELDS.join(",");
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}?fields={fields}"),
        None,
    )?;
    parse_issue(&data, &config.site).ok_or_else(|| "Jira sent an unreadable issue".to_string())
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

fn comment_url(site: &str, key: &str, id: &str) -> String {
    format!("https://{site}/browse/{key}?focusedCommentId={id}")
}

fn parse_thread(data: &Value, site: &str, key: &str) -> Result<Thread, String> {
    let comments = data
        .get("comments")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return comments".to_string())?;
    let total = data
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(comments.len() as u64);
    let mut parsed: Vec<Comment> = comments
        .iter()
        .filter_map(|node| {
            let id = string_field(node, "id").filter(|id| !id.is_empty())?;
            let author = node
                .get("author")
                .and_then(|a| string_field(a, "displayName"))
                .unwrap_or_default();
            Some(Comment {
                url: comment_url(site, key, &id),
                kind: "comment".into(),
                author,
                body: adf::rich_text(node.get("body")),
                created_at: normalize_time(&string_field(node, "created").unwrap_or_default()),
                id,
                ..Default::default()
            })
        })
        .collect();
    parsed.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(Thread {
        comments: parsed,
        commits: Vec::new(),
        truncated: total > comments.len() as u64,
    })
}

/// The issue's comments, oldest first (up to the latest `COMMENT_LIMIT`).
pub fn thread(key: &str) -> Result<Thread, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}/comment?maxResults={COMMENT_LIMIT}&orderBy=-created"),
        None,
    )?;
    parse_thread(&data, &config.site, key)
}

/// Posts a plain-text comment (as ADF) on the issue; returns the comment's URL.
pub fn post_comment(key: &str, body: &str) -> Result<String, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    let data = call_json(
        &config,
        Some("POST"),
        &format!("/rest/api/3/issue/{key}/comment"),
        Some(&json!({ "body": adf::text_to_adf(body) })),
    )?;
    let id = string_field(&data, "id").unwrap_or_default();
    if id.is_empty() {
        return Err("Could not post Jira comment".into());
    }
    Ok(comment_url(&config.site, key, &id))
}

// ---------------------------------------------------------------------------
// Workflow transitions
// ---------------------------------------------------------------------------

fn parse_transitions(data: &Value) -> Vec<Transition> {
    data.get("transitions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = string_field(row, "id").filter(|id| !id.is_empty())?;
            let name = string_field(row, "name").unwrap_or_default();
            let to_status = row
                .pointer("/to/name")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.clone());
            let category = row
                .pointer("/to/statusCategory/key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(Transition {
                id,
                name,
                to_status,
                category,
            })
        })
        .collect()
}

/// Available workflow transitions for `key`.
pub fn transitions(key: &str) -> Result<Vec<Transition>, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}/transitions"),
        None,
    )?;
    Ok(parse_transitions(&data))
}

/// Performs workflow transition `transition_id` on `key`, then returns the refreshed issue.
pub fn transition(key: &str, transition_id: &str) -> Result<WorkItem, String> {
    let config = require_config()?;
    let key = require_issue_key(key)?;
    let transition_id = transition_id.trim();
    if transition_id.is_empty() {
        return Err("Missing Jira transition id".into());
    }
    call_json(
        &config,
        Some("POST"),
        &format!("/rest/api/3/issue/{key}/transitions"),
        Some(&json!({ "transition": { "id": transition_id } })),
    )?;
    let fields = ISSUE_FIELDS.join(",");
    let data = call_json(
        &config,
        None,
        &format!("/rest/api/3/issue/{key}?fields={fields}"),
        None,
    )?;
    parse_issue(&data, &config.site).ok_or_else(|| "Jira sent an unreadable issue".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "acme.atlassian.net";

    #[test]
    fn normalizes_jira_sites() {
        assert_eq!(normalize_site("acme").unwrap(), SITE);
        assert_eq!(normalize_site("acme.atlassian.net").unwrap(), SITE);
        assert_eq!(
            normalize_site(" https://Acme.atlassian.net/jira/software/projects ").unwrap(),
            SITE
        );
        assert_eq!(
            normalize_site("jira.example.com:8443").unwrap(),
            "jira.example.com:8443"
        );
        assert!(normalize_site("http://acme.atlassian.net").is_err());
        assert!(normalize_site("https://user@acme.atlassian.net").is_err());
        assert!(normalize_site("jira.example.com:0").is_err());
        assert!(normalize_site("").is_err());
    }

    #[test]
    fn authorization_is_basic_email_and_token() {
        let config = Config {
            site: SITE.into(),
            email: "ada@acme.com".into(),
            token: "secret".into(),
            account_id: String::new(),
            display_name: String::new(),
        };
        assert_eq!(authorization(&config), "Basic YWRhQGFjbWUuY29tOnNlY3JldA==");
    }

    #[test]
    fn fetches_every_project_page() {
        let mut offsets = Vec::new();
        let projects = fetch_projects(|start| {
            offsets.push(start);
            Ok(if start == 0 {
                json!({ "startAt": 0, "isLast": false, "values": [
                    { "id": "10001", "key": "OPS", "name": "Operations" }
                ] })
            } else {
                json!({ "startAt": 1, "isLast": true, "values": [
                    { "id": "10000", "key": "ENG", "name": "Engineering" }
                ] })
            })
        })
        .unwrap();
        assert_eq!(offsets, vec![0, 1]);
        assert_eq!(projects.len(), 2);
        // Sorted by name.
        assert_eq!(projects[0].key, "ENG");
        assert_eq!(projects[1].key, "OPS");
    }

    #[test]
    fn rejects_stalled_project_pagination() {
        assert!(fetch_projects(|_| Ok(json!({ "isLast": false, "values": [] }))).is_err());
    }

    #[test]
    fn inbox_jql_combines_filters() {
        assert_eq!(
            inbox_jql(
                true,
                Some(&[
                    " 10000 ".into(),
                    "".into(),
                    "10001".into(),
                    "x) OR (1".into()
                ])
            ),
            "assignee = currentUser() AND project in (10000, 10001) AND statusCategory != Done ORDER BY updated DESC"
        );
        assert_eq!(
            inbox_jql(false, None),
            "statusCategory != Done ORDER BY updated DESC"
        );
    }

    #[test]
    fn visible_project_ids_handles_none_some_and_all_hidden() {
        let projects = vec![
            Project {
                id: "1".into(),
                key: "ENG".into(),
                name: "Engineering".into(),
            },
            Project {
                id: "2".into(),
                key: "OPS".into(),
                name: "Ops".into(),
            },
        ];
        assert_eq!(visible_project_ids(&projects, &[]), None);
        assert_eq!(
            visible_project_ids(&projects, &["2".into()]),
            Some(vec!["1".into()])
        );
        assert_eq!(
            visible_project_ids(&projects, &["1".into(), "2".into()]),
            Some(vec![])
        );
    }

    #[test]
    fn parse_issues_maps_fields() {
        let data = json!({
            "issues": [{
                "id": "10042",
                "key": "ENG-42",
                "fields": {
                    "summary": "Fix auth",
                    "created": "2026-08-25T08:00:00.000+0000",
                    "updated": "2026-08-27T10:00:00.000+0000",
                    "status": { "name": "In Progress", "statusCategory": { "key": "indeterminate" } },
                    "issuetype": { "name": "Bug" },
                    "priority": { "name": "High" },
                    "duedate": "2026-09-01",
                    "labels": ["backend"],
                    "assignee": { "displayName": "Maya" },
                    "project": { "id": "10000", "key": "ENG", "name": "Engineering" }
                }
            }, { "id": "10043" }]
        });
        let items = parse_issues(&data, SITE).unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.provider, Provider::Jira);
        assert_eq!(item.kind, Kind::Issue);
        assert_eq!(item.identifier, "ENG-42");
        assert_eq!(item.number, 42);
        assert_eq!(item.url, "https://acme.atlassian.net/browse/ENG-42");
        assert_eq!(item.state, "In Progress");
        assert_eq!(item.state_reason, "indeterminate");
        assert!(!item.closed);
        assert_eq!(item.updated_at, "2026-08-27T10:00:00.000+00:00");
        assert_eq!(item.issue_type.as_ref().unwrap().name, "Bug");
        assert_eq!(item.labels.len(), 2);
        assert_eq!(item.labels[0].name, "Bug");
        assert_eq!(item.labels[1].name, "backend");
        assert_eq!(item.assignees, ["Maya"]);
        assert_eq!(item.priority, "High");
        assert!(item.high_priority);
        assert_eq!(item.due_date, "2026-09-01");
        assert_eq!(item.repo, "ENG");
        assert_eq!(item.container_id, "10000");
    }

    #[test]
    fn parse_details_prefers_reporter() {
        let data = json!({
            "fields": {
                "description": {
                    "type": "doc",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Steps" }] }]
                },
                "reporter": { "displayName": "Ada" },
                "creator": { "displayName": "Lin" },
                "assignee": null
            }
        });
        let details = parse_details(&data).unwrap();
        assert_eq!(details.body, "Steps");
        assert_eq!(details.author, "Ada");
    }

    #[test]
    fn parse_thread_sorts_oldest_first() {
        let data = json!({
            "total": 3,
            "comments": [
                {
                    "id": "2",
                    "author": { "displayName": "Maya" },
                    "body": "Plain reply",
                    "created": "2026-08-31T11:00:00.000+0000"
                },
                {
                    "id": "1",
                    "author": { "displayName": "Ada" },
                    "body": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Root" }] }] },
                    "created": "2026-08-31T10:00:00.000+0000"
                }
            ]
        });
        let thread = parse_thread(&data, SITE, "ENG-42").unwrap();
        assert!(thread.truncated);
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[0].body, "Root");
        assert_eq!(
            thread.comments[0].url,
            "https://acme.atlassian.net/browse/ENG-42?focusedCommentId=1"
        );
        assert_eq!(thread.comments[1].author, "Maya");
        assert_eq!(thread.comments[1].body, "Plain reply");
    }

    #[test]
    fn parse_transitions_reads_target_status_and_category() {
        let data = json!({
            "transitions": [
                {
                    "id": "21",
                    "name": "Start Progress",
                    "to": { "name": "In Progress", "statusCategory": { "key": "indeterminate" } }
                },
                {
                    "id": "31",
                    "name": "Done",
                    "to": { "name": "Done", "statusCategory": { "key": "done" } }
                }
            ]
        });
        let list = parse_transitions(&data);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "21");
        assert_eq!(list[0].to_status, "In Progress");
        assert_eq!(list[0].category, "indeterminate");
        assert_eq!(list[1].category, "done");
    }

    #[test]
    fn issue_keys_and_errors_are_handled() {
        assert_eq!(require_issue_key(" ENG-42 ").unwrap(), "ENG-42");
        assert_eq!(require_issue_key("PROJ_1-7").unwrap(), "PROJ_1-7");
        assert!(require_issue_key("").is_err());
        assert!(require_issue_key("ENG-42/../x").is_err());
        assert!(require_issue_key("ENG").is_err());
        assert_eq!(
            error_message(r#"{"errorMessages":["Issue does not exist"],"errors":{}}"#).as_deref(),
            Some("Issue does not exist")
        );
        assert_eq!(
            error_message(r#"{"errorMessages":[],"errors":{"jql":"Bad JQL"}}"#).as_deref(),
            Some("Bad JQL")
        );
        assert_eq!(
            normalize_time("2026-08-27T10:00:00.000+0200"),
            "2026-08-27T10:00:00.000+02:00"
        );
    }
}
