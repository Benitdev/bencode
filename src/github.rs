//! GitHub through the `gh` CLI, as MonoCode's Inbox reads it
//! (`src-tauri/src/fs.rs` `git_github_*`): the CLI's status, a working
//! copy's repositories, open issues and pull requests, and one item's body.
//!
//! Every call runs `gh` and blocks; use the background executor.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// MonoCode's default page of a project's items.
const ITEM_LIMIT: &str = "40";
/// Where `gh` lives when the app starts from Finder without a shell PATH.
const GH_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Issue,
    Pr,
}

impl Kind {
    fn arg(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "pr",
        }
    }

    fn fields(self) -> &'static str {
        match self {
            Self::Issue => {
                "number,title,url,state,stateReason,createdAt,updatedAt,labels,assignees"
            }
            Self::Pr => "number,title,url,state,createdAt,updatedAt,labels,assignees,isDraft",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

/// MonoCode `InboxItem` for the GitHub source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkItem {
    pub kind: Kind,
    pub number: i64,
    pub title: String,
    pub url: String,
    /// `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub state_reason: String,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub draft: bool,
    /// `owner/name`.
    pub repo: String,
    /// The project folder it was listed for.
    pub project: String,
}

impl WorkItem {
    /// Unique across repositories and kinds.
    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.repo.to_lowercase(),
            self.kind.arg(),
            self.number
        )
    }
}

/// MonoCode `GithubWorkItemDetails`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub body: String,
    pub author: String,
    pub base_ref: String,
    pub head_ref: String,
    pub review_decision: String,
}

/// Whether `gh` is there and signed in to github.com.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    SignedOut,
    Ready,
}

fn gh_path() -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
    from_path
        .chain(GH_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join("gh"))
        .find(|candidate| candidate.is_file())
}

/// MonoCode `gh_run`: stdout, or the CLI's own error text.
fn gh(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let program = gh_path().ok_or_else(|| "GitHub CLI (`gh`) is not installed.".to_string())?;
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .output()
        .map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("gh {} failed", args.join(" "))
    })
}

/// MonoCode `git_github_status`.
pub fn status() -> Status {
    let Some(program) = gh_path() else {
        return Status::NotInstalled;
    };
    let signed_in = Command::new(program)
        .args(["auth", "status", "--active", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .is_ok_and(|out| out.status.success());
    if signed_in {
        Status::Ready
    } else {
        Status::SignedOut
    }
}

/// MonoCode `git_github_repositories`: the working copy's repository and,
/// for a fork, its parent.
pub fn repositories(cwd: &Path) -> Result<Vec<String>, String> {
    parse_repositories(&gh(
        cwd,
        &["repo", "view", "--json", "nameWithOwner,parent"],
    )?)
}

fn parse_repositories(json: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Owner {
        login: String,
    }
    #[derive(Deserialize)]
    struct Parent {
        name: String,
        owner: Owner,
    }
    #[derive(Deserialize)]
    struct View {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
        #[serde(default)]
        parent: Option<Parent>,
    }
    let view: View = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let mut repos = vec![view.name_with_owner];
    if let Some(parent) = view.parent {
        let parent = format!("{}/{}", parent.owner.login, parent.name);
        if !repos[0].eq_ignore_ascii_case(&parent) {
            repos.push(parent);
        }
    }
    Ok(repos)
}

/// MonoCode `git_github_work_items` for open items.
pub fn work_items(
    cwd: &Path,
    repo: &str,
    kind: Kind,
    assigned_to_me: bool,
) -> Result<Vec<WorkItem>, String> {
    let mut args = vec![
        kind.arg(),
        "list",
        "--state",
        "open",
        "--limit",
        ITEM_LIMIT,
        "--repo",
        repo,
        "--json",
        kind.fields(),
    ];
    if assigned_to_me {
        args.extend(["--assignee", "@me"]);
    }
    let json = gh(cwd, &args)?;
    parse_work_items(&json, kind, repo, &cwd.to_string_lossy())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemRow {
    number: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    state_reason: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    assignees: Vec<Login>,
    #[serde(default)]
    is_draft: bool,
}

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    login: String,
}

fn parse_work_items(
    json: &str,
    kind: Kind,
    repo: &str,
    project: &str,
) -> Result<Vec<WorkItem>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<ItemRow> = serde_json::from_str(json).map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| WorkItem {
            kind,
            number: row.number,
            title: row.title,
            url: row.url,
            state: row.state,
            state_reason: row.state_reason.unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
            labels: row.labels,
            assignees: row.assignees.into_iter().map(|a| a.login).collect(),
            draft: row.is_draft,
            repo: repo.to_string(),
            project: project.to_string(),
        })
        .collect())
}

/// MonoCode `git_github_work_item_details`.
pub fn details(cwd: &Path, repo: &str, kind: Kind, number: i64) -> Result<Details, String> {
    let fields = match kind {
        Kind::Pr => "body,author,baseRefName,headRefName,reviewDecision",
        Kind::Issue => "body,author",
    };
    let number = number.to_string();
    let json = gh(
        cwd,
        &[
            kind.arg(),
            "view",
            &number,
            "--repo",
            repo,
            "--json",
            fields,
        ],
    )?;
    parse_details(&json)
}

fn parse_details(json: &str) -> Result<Details, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        #[serde(default)]
        body: String,
        #[serde(default)]
        author: Option<Login>,
        #[serde(default)]
        base_ref_name: String,
        #[serde(default)]
        head_ref_name: String,
        #[serde(default)]
        review_decision: Option<String>,
    }
    let row: Row = serde_json::from_str(json).map_err(|err| err.to_string())?;
    Ok(Details {
        body: row.body,
        author: row.author.map(|a| a.login).unwrap_or_default(),
        base_ref: row.base_ref_name,
        head_ref: row.head_ref_name,
        review_decision: row.review_decision.unwrap_or_default(),
    })
}

/// MonoCode `repositoriesByPath`: a project's repositories rarely change,
/// so the Inbox's poll asks `gh` once per project per run.
fn cached_repositories(project: &str) -> Result<Vec<String>, String> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let cached = cache
        .lock()
        .map_err(|err| err.to_string())?
        .get(project)
        .cloned();
    if let Some(repos) = cached {
        return Ok(repos);
    }
    let repos = repositories(Path::new(project))?;
    cache
        .lock()
        .map_err(|err| err.to_string())?
        .insert(project.to_string(), repos.clone());
    Ok(repos)
}

/// What one fetch of the Inbox found, and why projects were left out.
#[derive(Clone, Debug, Default)]
pub struct InboxList {
    pub items: Vec<WorkItem>,
    pub errors: Vec<String>,
}

/// MonoCode `fetchInboxItems` for GitHub: every project's repositories
/// (each once), their open issues and pull requests, newest first.
pub fn inbox(projects: &[String], assigned_to_me: bool) -> InboxList {
    let mut list = InboxList::default();
    let mut repos: Vec<(String, String)> = Vec::new();
    for project in projects {
        match cached_repositories(project) {
            Ok(found) => {
                for repo in found {
                    if !repos.iter().any(|(r, _)| r.eq_ignore_ascii_case(&repo)) {
                        repos.push((repo, project.clone()));
                    }
                }
            }
            Err(err) => log::debug!("inbox: {project} has no GitHub repository: {err}"),
        }
    }
    for (repo, project) in &repos {
        for kind in [Kind::Issue, Kind::Pr] {
            match work_items(Path::new(project), repo, kind, assigned_to_me) {
                Ok(items) => list.items.extend(items),
                Err(err) => {
                    log::warn!("inbox: could not list {repo}: {err}");
                    list.errors.push(format!("{repo}: {err}"));
                }
            }
        }
    }
    list.items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    list.errors.dedup();
    list
}

// ---------------------------------------------------------------------------
// One item's conversation (MonoCode `git_github_work_item_thread`).
// ---------------------------------------------------------------------------

const ISSUE_THREAD_QUERY: &str = r#"
query InboxIssueThread($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      comments(last: 40) {
        totalCount
        nodes { id author { login } body createdAt url isMinimized }
      }
    }
  }
}
"#;

const PR_THREAD_QUERY: &str = r#"
query InboxPullRequestThread($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      commits(last: 40) {
        totalCount
        nodes { commit { oid messageHeadline committedDate url author { name user { login } } } }
      }
      comments(last: 40) {
        totalCount
        nodes { id author { login } body createdAt url isMinimized }
      }
      reviews(last: 40) {
        totalCount
        nodes { id author { login } body state submittedAt url }
      }
      reviewThreads(last: 20) {
        totalCount
        nodes {
          id
          isResolved
          path
          comments(first: 8) {
            totalCount
            nodes { id author { login } body createdAt url path line originalLine isMinimized }
          }
        }
      }
    }
  }
}
"#;

const REVIEW_REPLY_MUTATION: &str = r#"
mutation InboxReviewReply($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {
    pullRequestReviewThreadId: $threadId
    body: $body
  }) {
    comment { url }
  }
}
"#;

/// MonoCode `GitHubWorkItemComment`: a comment, a review, or a review
/// thread (its first comment, the rest as `replies`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    /// `comment`, `review` or `review_comment`.
    pub kind: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
    /// A review's `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, …
    pub state: String,
    pub path: String,
    pub line: Option<i64>,
    pub resolved: bool,
    /// The review thread a reply goes to.
    pub thread_id: String,
    pub replies: Vec<Comment>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub headline: String,
    pub author: String,
    pub committed_at: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thread {
    pub comments: Vec<Comment>,
    pub commits: Vec<Commit>,
    /// GitHub has more than the latest page shown.
    pub truncated: bool,
}

fn split_repo(repo: &str) -> Result<(&str, &str), String> {
    let (owner, name) = repo
        .trim()
        .split_once('/')
        .ok_or_else(|| format!("Invalid GitHub repository: {repo}"))?;
    let valid = |part: &str| {
        !part.is_empty()
            && !matches!(part, "." | "..")
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    if valid(owner) && valid(name) {
        Ok((owner, name))
    } else {
        Err(format!("Invalid GitHub repository: {repo}"))
    }
}

pub fn thread(cwd: &Path, repo: &str, kind: Kind, number: i64) -> Result<Thread, String> {
    let (owner, name) = split_repo(repo)?;
    let query = match kind {
        Kind::Pr => PR_THREAD_QUERY,
        Kind::Issue => ISSUE_THREAD_QUERY,
    };
    let args = thread_args(query, owner, name, number);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let json = gh(cwd, &args)?;
    parse_thread(&json, kind)
}

/// `gh api graphql` argv: `-f` sends strings as typed, `-F` would turn a
/// repository named `2048` into a number.
fn thread_args(query: &str, owner: &str, name: &str, number: i64) -> Vec<String> {
    vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={query}"),
        "-f".into(),
        format!("owner={owner}"),
        "-f".into(),
        format!("name={name}"),
        "-F".into(),
        format!("number={number}"),
    ]
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Nodes<T> {
    #[serde(rename = "totalCount")]
    total_count: i64,
    nodes: Vec<T>,
}

impl<T> Nodes<T> {
    fn truncated(&self) -> bool {
        (self.nodes.len() as i64) < self.total_count
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlComment {
    id: String,
    author: Option<Login>,
    body: String,
    created_at: String,
    url: String,
    path: String,
    line: Option<i64>,
    original_line: Option<i64>,
    is_minimized: bool,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlReview {
    id: String,
    author: Option<Login>,
    body: String,
    state: String,
    submitted_at: Option<String>,
    url: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlReviewThread {
    id: String,
    is_resolved: bool,
    path: String,
    comments: Nodes<GqlComment>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlCommitAuthor {
    name: String,
    user: Option<Login>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlCommit {
    oid: String,
    message_headline: String,
    committed_date: String,
    url: String,
    author: Option<GqlCommitAuthor>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlCommitNode {
    commit: GqlCommit,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlPull {
    commits: Nodes<GqlCommitNode>,
    comments: Nodes<GqlComment>,
    reviews: Nodes<GqlReview>,
    review_threads: Nodes<GqlReviewThread>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlIssue {
    comments: Nodes<GqlComment>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GqlRepository {
    issue: Option<GqlIssue>,
    pull_request: Option<GqlPull>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlData {
    repository: Option<GqlRepository>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlError {
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlEnvelope {
    data: Option<GqlData>,
    errors: Vec<GqlError>,
}

fn login(author: Option<Login>) -> String {
    author
        .map(|a| a.login)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// MonoCode `github_mapped_comment`: minimized comments are left out.
fn map_comment(comment: GqlComment, kind: &str, path: &str, resolved: bool) -> Option<Comment> {
    if comment.is_minimized {
        return None;
    }
    let author = login(comment.author);
    let created_at = comment.created_at.trim().to_string();
    let id = match comment.id.trim() {
        "" => format!("{kind}:{author}:{created_at}"),
        id => id.to_string(),
    };
    Some(Comment {
        id,
        kind: kind.to_string(),
        author,
        body: comment.body,
        created_at,
        url: comment.url,
        state: String::new(),
        path: match comment.path.trim() {
            "" => path.trim().to_string(),
            own => own.to_string(),
        },
        line: comment.line.or(comment.original_line),
        resolved,
        thread_id: String::new(),
        replies: Vec::new(),
    })
}

/// MonoCode `github_review_comment`: pending and empty "commented"
/// reviews are left out.
fn map_review(review: GqlReview) -> Option<Comment> {
    let state = review.state.trim().to_uppercase();
    if state.is_empty()
        || state == "PENDING"
        || (state == "COMMENTED" && review.body.trim().is_empty())
    {
        return None;
    }
    let created_at = review.submitted_at.unwrap_or_default().trim().to_string();
    if created_at.is_empty() {
        return None;
    }
    let author = login(review.author);
    Some(Comment {
        id: match review.id.trim() {
            "" => format!("review:{author}:{created_at}"),
            id => id.to_string(),
        },
        kind: "review".into(),
        author,
        body: review.body,
        created_at,
        url: review.url,
        state,
        ..Comment::default()
    })
}

fn map_review_thread(thread: GqlReviewThread) -> Option<Comment> {
    let thread_id = thread.id.trim().to_string();
    let (path, resolved) = (thread.path, thread.is_resolved);
    let mut mapped = thread
        .comments
        .nodes
        .into_iter()
        .filter_map(|c| map_comment(c, "review_comment", &path, resolved))
        .map(|mut c| {
            c.thread_id = thread_id.clone();
            c
        });
    let mut first = mapped.next()?;
    first.replies = mapped.collect();
    Some(first)
}

fn parse_thread(json: &str, kind: Kind) -> Result<Thread, String> {
    let envelope: GqlEnvelope = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let error = envelope
        .errors
        .iter()
        .map(|e| e.message.trim())
        .find(|m| !m.is_empty())
        .map(str::to_string);
    let repository = envelope.data.and_then(|d| d.repository).ok_or_else(|| {
        error
            .clone()
            .unwrap_or_else(|| "GitHub item not found".into())
    })?;
    let mut thread = Thread::default();
    match kind {
        Kind::Pr => {
            let pull = repository
                .pull_request
                .ok_or_else(|| error.unwrap_or_else(|| "GitHub pull request not found".into()))?;
            thread.truncated = pull.commits.truncated()
                || pull.comments.truncated()
                || pull.reviews.truncated()
                || pull.review_threads.truncated();
            thread.commits = pull
                .commits
                .nodes
                .into_iter()
                .map(|n| n.commit)
                .filter(|c| !c.oid.trim().is_empty() && !c.committed_date.trim().is_empty())
                .map(|c| Commit {
                    author: c
                        .author
                        .map(|a| {
                            a.user
                                .map(|u| u.login)
                                .filter(|l| !l.trim().is_empty())
                                .unwrap_or(a.name)
                        })
                        .unwrap_or_default(),
                    oid: c.oid,
                    headline: c.message_headline,
                    committed_at: c.committed_date,
                    url: c.url,
                })
                .collect();
            thread.comments.extend(
                pull.comments
                    .nodes
                    .into_iter()
                    .filter_map(|c| map_comment(c, "comment", "", false)),
            );
            thread
                .comments
                .extend(pull.reviews.nodes.into_iter().filter_map(map_review));
            thread.comments.extend(
                pull.review_threads
                    .nodes
                    .into_iter()
                    .filter_map(map_review_thread),
            );
        }
        Kind::Issue => {
            let issue = repository
                .issue
                .ok_or_else(|| error.unwrap_or_else(|| "GitHub issue not found".into()))?;
            thread.truncated = issue.comments.truncated();
            thread.comments.extend(
                issue
                    .comments
                    .nodes
                    .into_iter()
                    .filter_map(|c| map_comment(c, "comment", "", false)),
            );
        }
    }
    thread.comments.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(thread)
}

/// MonoCode `with_temp_markdown`: a body goes to `gh` as a file.
fn with_temp_body(
    body: &str,
    run: impl FnOnce(&str) -> Result<String, String>,
) -> Result<String, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!("bencode-comment-{stamp}.md"));
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    let result = run(&path.to_string_lossy());
    if let Err(err) = std::fs::remove_file(&path) {
        log::warn!("could not remove {}: {err}", path.display());
    }
    result
}

/// MonoCode `valid_github_node_id`.
fn valid_node_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id.len() < 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '='))
}

/// MonoCode `git_github_work_item_comment`: a new comment, or with
/// `reply_thread` a reply in that review thread. Returns its URL (may be
/// empty when GitHub does not print one).
pub fn post_comment(
    cwd: &Path,
    repo: &str,
    kind: Kind,
    number: i64,
    body: &str,
    reply_thread: Option<&str>,
) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    split_repo(repo)?;
    if let Some(thread) = reply_thread.map(str::trim).filter(|t| !t.is_empty()) {
        if !valid_node_id(thread) {
            return Err("Invalid review thread".into());
        }
        return with_temp_body(body, |path| {
            gh(
                cwd,
                &[
                    "api",
                    "graphql",
                    "-f",
                    &format!("query={REVIEW_REPLY_MUTATION}"),
                    "-f",
                    &format!("threadId={thread}"),
                    "-F",
                    &format!("body=@{path}"),
                ],
            )
        });
    }
    let number = number.to_string();
    with_temp_body(body, |path| {
        gh(
            cwd,
            &[
                kind.arg(),
                "comment",
                &number,
                "--repo",
                repo,
                "--body-file",
                path,
            ],
        )
    })
}

// ---------------------------------------------------------------------------
// Pull request checks (MonoCode `git_github_pr_checks`, `_check_details`).
// ---------------------------------------------------------------------------

/// MonoCode `GithubPrCheckState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckState {
    Fail,
    Pending,
    Cancel,
    Unknown,
    Pass,
    Skipping,
}

impl CheckState {
    /// MonoCode `CHECK_STATES` order.
    pub const ORDER: [Self; 6] = [
        Self::Fail,
        Self::Pending,
        Self::Cancel,
        Self::Unknown,
        Self::Pass,
        Self::Skipping,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "passed",
            Self::Fail => "failed",
            Self::Pending => "running",
            Self::Skipping => "skipped",
            Self::Cancel => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// MonoCode `github_check_conclusion_state`.
    fn from_conclusion(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "SUCCESS" => Self::Pass,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => Self::Fail,
            "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" | "REQUESTED" => Self::Pending,
            "NEUTRAL" | "SKIPPED" => Self::Skipping,
            "CANCELLED" => Self::Cancel,
            _ => Self::Unknown,
        }
    }

    /// MonoCode `github_check_state`: an unfinished run reports its status.
    fn from_run(status: &str, conclusion: &str) -> Self {
        match status.trim().to_ascii_uppercase().as_str() {
            "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" | "REQUESTED" => Self::Pending,
            "COMPLETED" | "" => Self::from_conclusion(conclusion),
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub workflow: String,
    pub state: CheckState,
    pub url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checks {
    pub head_oid: String,
    pub checks: Vec<Check>,
}

pub fn pr_checks(cwd: &Path, repo: &str, number: i64) -> Result<Checks, String> {
    split_repo(repo)?;
    let number = number.to_string();
    let json = gh(
        cwd,
        &[
            "pr",
            "view",
            &number,
            "--repo",
            repo,
            "--json",
            "headRefOid,statusCheckRollup",
        ],
    )?;
    parse_checks(&json)
}

fn parse_checks(json: &str) -> Result<Checks, String> {
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct CheckRow {
        #[serde(rename = "__typename")]
        typename: String,
        name: String,
        context: String,
        status: String,
        conclusion: Option<String>,
        state: String,
        workflow_name: String,
        details_url: Option<String>,
        target_url: Option<String>,
        created_at: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Row {
        head_ref_oid: String,
        status_check_rollup: Option<Vec<CheckRow>>,
    }
    let row: Row = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let head_oid = row.head_ref_oid.trim().to_string();
    if head_oid.is_empty() {
        return Err("Pull request is missing head commit".into());
    }
    let url = |u: Option<String>| u.filter(|u| !u.trim().is_empty());
    let checks = row
        .status_check_rollup
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let status_context = if r.typename.is_empty() {
                !r.context.is_empty()
            } else {
                r.typename.eq_ignore_ascii_case("StatusContext")
            };
            if status_context {
                Check {
                    name: r.context,
                    workflow: String::new(),
                    state: CheckState::from_conclusion(&r.state),
                    url: url(r.target_url),
                    started_at: r.created_at,
                    completed_at: None,
                }
            } else {
                Check {
                    state: CheckState::from_run(&r.status, r.conclusion.as_deref().unwrap_or("")),
                    name: r.name,
                    workflow: r.workflow_name,
                    url: url(r.details_url),
                    started_at: r.started_at,
                    completed_at: r.completed_at,
                }
            }
        })
        .collect();
    Ok(Checks { head_oid, checks })
}

/// MonoCode `githubActionsJobId`: the Actions job a check URL points at.
pub fn actions_job_id(url: &str, repo: &str) -> Option<String> {
    let path = url.strip_prefix("https://github.com/")?;
    let prefix = format!("{}/", repo.to_lowercase());
    if !path.to_lowercase().starts_with(&prefix) {
        return None;
    }
    let rest: Vec<&str> = path[prefix.len()..]
        .trim_end_matches('/')
        .split('/')
        .collect();
    let id = match rest.as_slice() {
        ["actions", "runs", run, "job", id] | ["runs", run, "jobs", id]
            if run.bytes().all(|b| b.is_ascii_digit()) =>
        {
            *id
        }
        _ => return None,
    };
    (!id.is_empty() && !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| id.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    pub state: CheckState,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Annotation {
    #[serde(default)]
    pub path: String,
    #[serde(default, rename = "start_line")]
    pub line: u64,
    #[serde(default)]
    pub message: String,
    #[serde(default, rename = "annotation_level")]
    pub level: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckDetails {
    pub steps: Vec<Step>,
    pub annotations: Vec<Annotation>,
    pub notice: Option<String>,
}

/// MonoCode `github_check_details_with`: an Actions job's steps and its
/// check run's annotations.
pub fn check_details(cwd: &Path, repo: &str, job_id: &str) -> Result<CheckDetails, String> {
    let (owner, name) = split_repo(repo)?;
    if job_id.is_empty() || !job_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid GitHub repository or job ID".into());
    }
    #[derive(Deserialize)]
    struct JobStep {
        name: String,
        status: String,
        conclusion: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    }
    #[derive(Deserialize)]
    struct Job {
        id: u64,
        #[serde(default)]
        steps: Vec<JobStep>,
        check_run_url: Option<String>,
    }
    let prefix = format!("repos/{owner}/{name}");
    let api = |endpoint: &str| gh(cwd, &["api", "--hostname", "github.com", endpoint]);
    let job: Job = serde_json::from_str(&api(&format!("{prefix}/actions/jobs/{job_id}"))?)
        .map_err(|err| err.to_string())?;
    if job.id.to_string() != job_id {
        return Err("GitHub returned a different job".into());
    }
    let mut details = CheckDetails {
        steps: job
            .steps
            .into_iter()
            .map(|s| Step {
                state: CheckState::from_run(&s.status, s.conclusion.as_deref().unwrap_or("")),
                name: s.name,
                started_at: s.started_at,
                completed_at: s.completed_at,
            })
            .collect(),
        ..CheckDetails::default()
    };
    let check_prefix = format!("https://api.github.com/{prefix}/check-runs/");
    let check_id = job
        .check_run_url
        .as_deref()
        .and_then(|u| u.strip_prefix(&check_prefix))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
    match check_id {
        Some(id) => {
            let annotations = api(&format!(
                "{prefix}/check-runs/{id}/annotations?per_page=100"
            ))
            .and_then(|json| {
                serde_json::from_str::<Vec<Annotation>>(&json).map_err(|err| err.to_string())
            });
            match annotations {
                Ok(annotations) => {
                    if annotations.len() == 100 {
                        details.notice = Some(
                            "Showing the first 100 annotations. View the full log on GitHub for more."
                                .into(),
                        );
                    }
                    details.annotations = annotations;
                }
                Err(_) => {
                    details.notice = Some(
                        "Could not load error annotations. View the full log on GitHub.".into(),
                    )
                }
            }
        }
        None => {
            details.notice =
                Some("Error annotations are unavailable. View the full log on GitHub.".into())
        }
    }
    Ok(details)
}

// ---------------------------------------------------------------------------
// Pull request actions (MonoCode `git_github_pr_action`).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrAction {
    Merge,
    Squash,
    Rebase,
    Draft,
    Ready,
    Close,
    Reopen,
}

impl PrAction {
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Merge => &["merge", "--merge"],
            Self::Squash => &["merge", "--squash"],
            Self::Rebase => &["merge", "--rebase"],
            Self::Draft => &["ready", "--undo"],
            Self::Ready => &["ready"],
            Self::Close => &["close"],
            Self::Reopen => &["reopen"],
        }
    }

    pub fn is_merge(self) -> bool {
        matches!(self, Self::Merge | Self::Squash | Self::Rebase)
    }
}

/// One item by number (MonoCode `git_github_work_item`).
pub fn work_item(cwd: &Path, repo: &str, kind: Kind, number: i64) -> Result<WorkItem, String> {
    split_repo(repo)?;
    let number_arg = number.to_string();
    let json = gh(
        cwd,
        &[
            kind.arg(),
            "view",
            &number_arg,
            "--repo",
            repo,
            "--json",
            kind.fields(),
        ],
    )?;
    parse_work_items(&format!("[{json}]"), kind, repo, &cwd.to_string_lossy())?
        .into_iter()
        .next()
        .ok_or_else(|| "GitHub did not return the item".to_string())
}

/// Runs `action` on pull request `number`, then reads it back.
pub fn pr_action(
    cwd: &Path,
    repo: &str,
    number: i64,
    action: PrAction,
) -> Result<WorkItem, String> {
    split_repo(repo)?;
    let number_arg = number.to_string();
    let (verb, flags) = action
        .args()
        .split_first()
        .expect("every action has a verb");
    let mut args = vec!["pr", verb, &number_arg, "--repo", repo];
    args.extend_from_slice(flags);
    gh(cwd, &args)?;
    work_item(cwd, repo, Kind::Pr, number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_variables_keep_strings_as_strings() {
        let args = thread_args("q", "2048", "true", 7);
        let pair = |flag: &str, value: &str| args.windows(2).any(|w| w[0] == flag && w[1] == value);
        assert!(pair("-f", "owner=2048"));
        assert!(pair("-f", "name=true"));
        assert!(pair("-F", "number=7"));
    }

    #[test]
    fn repositories_include_a_forks_parent_once() {
        let json = r#"{"nameWithOwner":"me/app","parent":{"name":"app","owner":{"login":"org"}}}"#;
        assert_eq!(parse_repositories(json).unwrap(), ["me/app", "org/app"]);
        let json = r#"{"nameWithOwner":"org/app","parent":null}"#;
        assert_eq!(parse_repositories(json).unwrap(), ["org/app"]);
    }

    #[test]
    fn work_items_parse_gh_list_output() {
        let json = r#"[{"number":7,"title":"Fix login","url":"https://github.com/o/r/pull/7",
            "state":"OPEN","createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-02T00:00:00Z",
            "labels":[{"name":"bug","color":"d73a4a","id":"x"}],
            "assignees":[{"login":"ben"}],"isDraft":true}]"#;
        let items = parse_work_items(json, Kind::Pr, "o/r", "/p").unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!((item.number, item.draft, item.kind), (7, true, Kind::Pr));
        assert_eq!(item.labels[0].name, "bug");
        assert_eq!(item.assignees, ["ben"]);
        assert_eq!(item.key(), "o/r:pr:7");
        assert!(
            parse_work_items("", Kind::Issue, "o/r", "/p")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn details_tolerate_missing_fields() {
        let details = parse_details(r#"{"body":"Steps","author":{"login":"ann"}}"#).unwrap();
        assert_eq!(
            (details.body.as_str(), details.author.as_str()),
            ("Steps", "ann")
        );
        assert_eq!(details.review_decision, "");
    }

    #[test]
    fn pr_thread_merges_comments_reviews_and_threads() {
        let json = r#"{"data":{"repository":{"pullRequest":{
            "commits":{"totalCount":1,"nodes":[{"commit":{"oid":"abc","messageHeadline":"Fix",
                "committedDate":"2026-01-01T00:00:00Z","url":"u","author":{"name":"Maya","user":{"login":"maya"}}}}]},
            "comments":{"totalCount":3,"nodes":[
                {"id":"c1","author":{"login":"ann"},"body":"hi","createdAt":"2026-01-02T00:00:00Z","url":"u1","isMinimized":false},
                {"id":"c2","author":{"login":"spam"},"body":"x","createdAt":"2026-01-01T00:00:00Z","url":"u2","isMinimized":true}]},
            "reviews":{"totalCount":2,"nodes":[
                {"id":"r1","author":{"login":"bob"},"body":"","state":"APPROVED","submittedAt":"2026-01-03T00:00:00Z","url":"u3"},
                {"id":"r2","author":{"login":"bob"},"body":"","state":"COMMENTED","submittedAt":"2026-01-03T00:00:00Z","url":"u4"}]},
            "reviewThreads":{"totalCount":1,"nodes":[{"id":"T1","isResolved":true,"path":"src/a.rs",
                "comments":{"totalCount":2,"nodes":[
                    {"id":"t1","author":{"login":"cat"},"body":"nit","createdAt":"2026-01-01T12:00:00Z","url":"u5","line":4,"isMinimized":false},
                    {"id":"t2","author":{"login":"ann"},"body":"done","createdAt":"2026-01-01T13:00:00Z","url":"u6","isMinimized":false}]}}]}
        }}}}"#;
        let thread = parse_thread(json, Kind::Pr).unwrap();
        assert!(thread.truncated, "3 comments reported, 2 returned");
        assert_eq!(thread.commits[0].author, "maya");
        let ids: Vec<_> = thread.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["t1", "c1", "r1"],
            "minimized and empty reviews dropped, oldest first"
        );
        let review_thread = &thread.comments[0];
        assert_eq!(
            (review_thread.path.as_str(), review_thread.line),
            ("src/a.rs", Some(4))
        );
        assert!(review_thread.resolved);
        assert_eq!(review_thread.replies[0].thread_id, "T1");
        assert_eq!(thread.comments[2].state, "APPROVED");
    }

    #[test]
    fn graphql_errors_surface() {
        let json = r#"{"data":{"repository":null},"errors":[{"message":"Could not resolve"}]}"#;
        assert_eq!(
            parse_thread(json, Kind::Issue).unwrap_err(),
            "Could not resolve"
        );
    }

    #[test]
    fn checks_map_runs_and_status_contexts() {
        let json = r#"{"headRefOid":"abc","statusCheckRollup":[
            {"__typename":"CheckRun","name":"test","workflowName":"CI","status":"COMPLETED","conclusion":"FAILURE",
             "detailsUrl":"https://github.com/o/r/actions/runs/1/job/22"},
            {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":"SUCCESS"},
            {"__typename":"StatusContext","context":"deploy","state":"SUCCESS","targetUrl":""}]}"#;
        let checks = parse_checks(json).unwrap();
        let states: Vec<_> = checks.checks.iter().map(|c| c.state).collect();
        assert_eq!(
            states,
            [CheckState::Fail, CheckState::Pending, CheckState::Pass]
        );
        assert_eq!(checks.checks[2].url, None);
        assert!(parse_checks(r#"{"headRefOid":""}"#).is_err());
    }

    #[test]
    fn job_ids_come_from_actions_urls_of_the_repo() {
        let url = "https://github.com/O/R/actions/runs/123/job/456";
        assert_eq!(actions_job_id(url, "o/r").as_deref(), Some("456"));
        assert_eq!(
            actions_job_id("https://github.com/o/r/runs/1/jobs/9/", "o/r").as_deref(),
            Some("9")
        );
        assert_eq!(actions_job_id(url, "x/y"), None);
        assert_eq!(
            actions_job_id("https://ci.example.com/o/r/actions/runs/1/job/2", "o/r"),
            None
        );
    }

    #[test]
    fn repos_and_node_ids_are_validated() {
        assert!(split_repo("o/r").is_ok());
        assert!(split_repo("o/../r").is_err());
        assert!(split_repo("noslash").is_err());
        assert!(valid_node_id("PRRT_kwDOabc="));
        assert!(!valid_node_id("a b"));
    }
}
