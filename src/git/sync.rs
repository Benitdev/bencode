//! What MonoCode's Changes panel and Graph read and do beyond the file
//! list (`src-tauri/src/fs.rs`): the branch's remote, upstream and default
//! branch with ahead/behind counts, the commit history with parents and
//! refs, commit (or amend), push, pull, sync, and the branch's pull request
//! through `gh`.
//!
//! Every call runs git and blocks; use the background executor.

use std::path::Path;

use serde::Deserialize;

use super::{git_command, run_git_string};

/// MonoCode `GIT_HISTORY_DEFAULT`.
const HISTORY_LIMIT: usize = 200;

/// MonoCode `GitDiffIndex`'s synchronization fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncInfo {
    /// The checked-out branch; `None` when detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub remote: Option<String>,
    pub upstream: Option<String>,
    pub default_branch: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub ahead_of_default: usize,
    /// HEAD is already on some remote branch.
    pub head_pushed: bool,
}

/// `git` output, trimmed; `None` on failure or nothing printed (MonoCode
/// `git_stdout`).
fn stdout(cwd: &str, args: &[&str]) -> Option<String> {
    run_git_string(cwd, args)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Runs git for an action the user asked for; the error is git's own text
/// (MonoCode `git_checked`).
fn checked(cwd: &str, args: &[&str]) -> Result<(), String> {
    let output = git_command(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("git {} failed", args.join(" "))
    })
}

fn ref_exists(cwd: &str, spec: &str) -> bool {
    git_command(cwd)
        .args(["show-ref", "--verify", "--quiet", spec])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// MonoCode `git_url_repo_name`: the last path segment of a remote URL.
pub fn url_repo_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let name = trimmed.rsplit(['/', ':']).next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') {
        return None;
    }
    Some(name.to_string())
}

/// MonoCode `GitInfo.repo`: the origin's repository name, else the
/// checkout's top-level folder name, else `cwd`'s own name.
pub fn repo_name(cwd: &str) -> Option<String> {
    let folder = |path: &str| {
        std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    };
    let Some(top) = stdout(cwd, &["rev-parse", "--show-toplevel"]) else {
        return folder(cwd);
    };
    let top = top.trim().to_string();
    stdout(&top, &["remote", "get-url", "origin"])
        .and_then(|url| url_repo_name(&url))
        .or_else(|| folder(&top))
}

/// The top-level folder of `cwd`'s checkout, written the way `cwd` is
/// (`remote://` paths included): `cwd` less git's `--show-prefix`. `None`
/// outside a repository.
pub fn checkout_root(cwd: &str) -> Option<String> {
    let prefix = run_git_string(cwd, &["rev-parse", "--show-prefix"]).ok()?;
    Some(strip_prefix_dirs(cwd, prefix.trim()))
}

/// `cwd` without its trailing `prefix` folders (`sub/dir/`).
fn strip_prefix_dirs(cwd: &str, prefix: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return cwd.to_string();
    }
    cwd.strip_suffix(prefix)
        .and_then(|top| top.strip_suffix('/'))
        .filter(|top| !top.is_empty())
        .unwrap_or(cwd)
        .to_string()
}

/// MonoCode `git_remote_name`: `origin` when there is one, else the first.
pub fn remote_name(cwd: &str) -> Option<String> {
    pick_remote(&remote_names(cwd))
}

fn pick_remote(names: &[String]) -> Option<String> {
    if names.iter().any(|n| n == "origin") {
        return Some("origin".into());
    }
    names.first().cloned()
}

fn remote_names(cwd: &str) -> Vec<String> {
    stdout(cwd, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect()
}

/// MonoCode `git_default_branch`.
pub fn default_branch(cwd: &str, remote: Option<&str>) -> Option<String> {
    if let Some(remote) = remote {
        if let Some(head) = stdout(
            cwd,
            &[
                "symbolic-ref",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        ) {
            return Some(
                head.split_once('/')
                    .map_or(head.clone(), |(_, name)| name.to_string()),
            );
        }
        for name in ["main", "master"] {
            if ref_exists(cwd, &format!("refs/remotes/{remote}/{name}")) {
                return Some(name.into());
            }
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|name| ref_exists(cwd, &format!("refs/heads/{name}")))
        .map(str::to_string)
}

/// MonoCode `git_ahead_behind`: (ahead, behind) of HEAD against `base`.
fn ahead_behind(cwd: &str, base: &str) -> (usize, usize) {
    let Some(text) = stdout(
        cwd,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...HEAD"),
        ],
    ) else {
        return (0, 0);
    };
    let mut parts = text.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let behind = parts.next().unwrap_or(0);
    let ahead = parts.next().unwrap_or(0);
    (ahead, behind)
}

/// What the sync info and the history both start from: the remotes, the
/// branch's upstream and the default branch. A snapshot reads it once.
#[derive(Clone, Debug, Default)]
pub struct RepoBase {
    remotes: Vec<String>,
    upstream: Option<String>,
    default_branch: Option<String>,
}

impl RepoBase {
    /// The upstream is read beside the remotes; the default branch needs
    /// the remote.
    pub fn read(cwd: &str) -> Self {
        std::thread::scope(|scope| {
            let upstream =
                scope.spawn(|| stdout(cwd, &["rev-parse", "--abbrev-ref", "@{upstream}"]));
            let remotes = remote_names(cwd);
            let default_branch = default_branch(cwd, pick_remote(&remotes).as_deref());
            Self {
                remotes,
                upstream: joined(upstream),
                default_branch,
            }
        })
    }

    fn remote(&self) -> Option<String> {
        pick_remote(&self.remotes)
    }

    /// `remote/branch` of the default branch, when there is a remote.
    fn default_ref(&self) -> Option<String> {
        self.remote()
            .zip(self.default_branch.as_ref())
            .map(|(r, b)| format!("{r}/{b}"))
    }
}

/// A scoped git read's result; a panic counts as nothing read.
fn joined<T: Default>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().unwrap_or_else(|_| {
        log::error!("a git read panicked");
        T::default()
    })
}

/// MonoCode `git_sync_for` with the branch and HEAD. The snapshot reads
/// the base once for this and the history (`sync_info_from`).
pub fn sync_info(cwd: &str) -> SyncInfo {
    if crate::remote::missing_here(cwd) {
        return SyncInfo::default();
    }
    sync_info_from(cwd, &RepoBase::read(cwd))
}

/// The sync info from a base already read. Each git call is a process of
/// its own and none needs another's answer, so they run side by side.
pub fn sync_info_from(cwd: &str, base: &RepoBase) -> SyncInfo {
    let default_ref = base.default_ref();
    // Ahead and behind count against the upstream, else the default branch.
    let tracked = match (&base.upstream, &default_ref) {
        (Some(_), _) => Some("@{upstream}"),
        (None, Some(base)) => Some(base.as_str()),
        _ => None,
    };
    std::thread::scope(|scope| {
        let branch = scope.spawn(|| stdout(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]));
        let head = scope.spawn(|| stdout(cwd, &["rev-parse", "HEAD"]));
        let head_pushed = scope.spawn(|| {
            stdout(
                cwd,
                &[
                    "for-each-ref",
                    "--count=1",
                    "--contains",
                    "HEAD",
                    "refs/remotes",
                ],
            )
            .is_some()
        });
        // The count against the default branch, unless that is the one
        // `tracked` already counts against.
        let of_default = default_ref
            .as_deref()
            .filter(|base| tracked != Some(*base))
            .map(|base| scope.spawn(move || ahead_behind(cwd, base).0));
        let (ahead, behind) = tracked.map_or((0, 0), |base| ahead_behind(cwd, base));
        SyncInfo {
            branch: joined(branch),
            head: joined(head),
            head_pushed: joined(head_pushed),
            remote: base.remote(),
            upstream: base.upstream.clone(),
            default_branch: base.default_branch.clone(),
            ahead,
            behind,
            ahead_of_default: of_default.map_or(ahead, joined),
        }
    })
}

// ---------------------------------------------------------------------------
// History for the Graph (MonoCode `git_history`).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRef {
    pub name: String,
    /// `local`, `remote` or `tag`.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCommit {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<HistoryRef>,
    pub head: bool,
}

/// MonoCode `git_history_tips`: HEAD, its upstream, the default branch.
/// The remote branch is named by its full ref and `log` skips it if it is
/// not there (`--ignore-missing`), so it needs no check of its own.
fn history_tips(base: &RepoBase) -> Vec<String> {
    let mut tips = vec!["HEAD".to_string()];
    if base.upstream.is_some() {
        tips.push("@{upstream}".into());
    }
    if let Some(spec) = base.default_ref() {
        tips.push(format!("refs/remotes/{spec}"));
    }
    tips
}

/// Recent commits of HEAD, its upstream and the default branch, newest
/// first in topological order, with parents for the graph.
#[cfg(test)]
pub fn history(cwd: &str) -> Vec<HistoryCommit> {
    history_from(cwd, &RepoBase::read(cwd))
}

/// The history (newest first, topological, with parents for the graph)
/// from a base already read; HEAD is read beside the log.
pub fn history_from(cwd: &str, base: &RepoBase) -> Vec<HistoryCommit> {
    let count = HISTORY_LIMIT.to_string();
    let mut args = vec![
        "log".to_string(),
        "--no-show-signature".into(),
        "--topo-order".into(),
        "--decorate=short".into(),
        "--ignore-missing".into(),
        "--max-count".into(),
        count,
        "--format=%H%x00%h%x00%P%x00%an%x00%at%x00%D%x00%s%x1e".into(),
    ];
    args.extend(history_tips(base));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    std::thread::scope(|scope| {
        let head = scope.spawn(|| stdout(cwd, &["rev-parse", "HEAD"]));
        let text = run_git_string(cwd, &refs);
        let head = joined(head);
        // No commit yet: no history, whatever `log` said.
        if head.is_none() {
            return Vec::new();
        }
        let Ok(text) = text else {
            return Vec::new();
        };
        parse_history(&text, head.as_deref(), &base.remotes)
    })
}

fn parse_history(text: &str, head: Option<&str>, remotes: &[String]) -> Vec<HistoryCommit> {
    text.split('\u{1e}')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let mut fields = record.split('\0');
            let sha = fields.next().filter(|s| !s.is_empty())?;
            let short = fields.next()?;
            let parents = fields.next()?;
            let author = fields.next()?;
            let timestamp = fields.next()?;
            let decorations = fields.next()?;
            let subject = fields.next().unwrap_or("");
            let (is_head, refs) = parse_decorations(decorations, head, sha, remotes);
            Some(HistoryCommit {
                sha: sha.to_string(),
                short_sha: if short.is_empty() {
                    sha.chars().take(7).collect()
                } else {
                    short.to_string()
                },
                parents: parents.split_whitespace().map(str::to_string).collect(),
                author: author.to_string(),
                timestamp: timestamp.parse().unwrap_or(0),
                subject: subject.to_string(),
                refs,
                head: is_head,
            })
        })
        .collect()
}

/// MonoCode `parse_git_decorations`.
fn parse_decorations(
    raw: &str,
    head: Option<&str>,
    sha: &str,
    remotes: &[String],
) -> (bool, Vec<HistoryRef>) {
    let mut is_head = head == Some(sha);
    let mut refs = Vec::new();
    let r = |name: &str, kind: &str| HistoryRef {
        name: name.to_string(),
        kind: kind.to_string(),
    };
    for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(name) = part.strip_prefix("HEAD -> ") {
            is_head = true;
            if !name.is_empty() {
                refs.push(r(name, "local"));
            }
        } else if part == "HEAD" {
            is_head = true;
        } else if let Some(tag) = part.strip_prefix("tag: ") {
            if !tag.is_empty() {
                refs.push(r(tag, "tag"));
            }
        } else if part.ends_with("/HEAD") {
            continue;
        } else if remotes
            .iter()
            .any(|remote| part == remote || part.starts_with(&format!("{remote}/")))
        {
            refs.push(r(part, "remote"));
        } else {
            refs.push(r(part, "local"));
        }
    }
    (is_head, refs)
}

// ---------------------------------------------------------------------------
// Commit, push, pull, sync.
// ---------------------------------------------------------------------------

/// MonoCode `with_signing_hint`.
fn with_signing_hint(error: String) -> String {
    if !error.contains("failed to sign") && !error.contains("ssh-keygen") {
        return error;
    }
    format!(
        "{error}\n\nGit couldn't sign this commit. BenCode runs git without a terminal, \
         so your signer needs a GUI passphrase prompt (e.g. pinentry-mac) or an unlocked agent."
    )
}

/// MonoCode `git_commit` (and its amend).
pub fn commit(cwd: &str, message: &str, amend: bool) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Commit message cannot be empty".into());
    }
    let mut args = vec!["commit"];
    if amend {
        args.push("--amend");
    }
    args.extend(["--cleanup=strip", "-m", message]);
    checked(cwd, &args).map_err(with_signing_hint)
}

/// MonoCode `git_head_message`: HEAD's subject and body.
pub fn head_message(cwd: &str) -> Result<String, String> {
    stdout(cwd, &["log", "-1", "--pretty=%B"]).ok_or_else(|| "No commits yet".into())
}

/// VS Code "Undo Last Commit": the branch steps back one commit, whose
/// changes stay staged. BenCode's own, like the two below.
pub fn undo_last_commit(cwd: &str) -> Result<(), String> {
    checked(cwd, &["reset", "--soft", "HEAD~1"])
}

/// A new commit that takes back what `sha` changed. A revert that does not
/// apply is given up, so the tree is left as it was.
pub fn revert_commit(cwd: &str, sha: &str) -> Result<(), String> {
    super::diffs::validate_sha(sha).map_err(|err| format!("{err:#}"))?;
    // One the user started in a terminal is theirs to finish or abort.
    if stdout(cwd, &["rev-parse", "-q", "--verify", "REVERT_HEAD"]).is_some() {
        return Err("A revert is already in progress in this repository".into());
    }
    checked(cwd, &["revert", "--no-edit", sha]).map_err(|err| {
        if stdout(cwd, &["rev-parse", "-q", "--verify", "REVERT_HEAD"]).is_some()
            && let Err(abort) = checked(cwd, &["revert", "--abort"])
        {
            log::warn!("could not abort the failed revert: {abort}");
        }
        with_signing_hint(err)
    })
}

/// `sha`'s subject and body.
pub fn commit_message(cwd: &str, sha: &str) -> Result<String, String> {
    super::diffs::validate_sha(sha).map_err(|err| format!("{err:#}"))?;
    stdout(cwd, &["log", "-1", "--pretty=%B", sha]).ok_or_else(|| "No such commit".into())
}

/// MonoCode `git_push_for`: to the upstream, or publish to the remote.
pub fn push(cwd: &str) -> Result<(), String> {
    if stdout(cwd, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some() {
        return checked(cwd, &["push"]);
    }
    let remote = remote_name(cwd).ok_or_else(|| "No git remote to push to".to_string())?;
    checked(cwd, &["push", "-u", &remote, "HEAD"])
}

/// MonoCode `git_pull`: fast-forward only.
pub fn pull(cwd: &str) -> Result<(), String> {
    checked(cwd, &["pull", "--ff-only"])
}

/// VS Code `git.autofetch`: a quiet `git fetch` so ahead/behind see new
/// remote commits. `Ok(true)` when a remote-tracking ref moved. Never
/// prompts: no terminal, and `output()` gives git a null stdin.
pub fn auto_fetch(cwd: &str) -> Result<bool, String> {
    let remote_refs = || {
        stdout(
            cwd,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/remotes",
            ],
        )
    };
    let before = remote_refs();
    checked(cwd, &["fetch", "--quiet"])?;
    Ok(remote_refs() != before)
}

/// MonoCode `git_sync_changes_for`: pull then push, or publish.
pub fn sync(cwd: &str) -> Result<(), String> {
    if stdout(cwd, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some() {
        checked(cwd, &["pull", "--no-edit", "--ff"]).map_err(with_signing_hint)?;
        return checked(cwd, &["push"]);
    }
    push(cwd)
}

/// MonoCode `git_staged_context`: what a commit message is written from.
pub struct StagedContext {
    pub branch: Option<String>,
    pub summary: String,
    pub patch: String,
}

pub fn staged_context(cwd: &str) -> Result<StagedContext, String> {
    let run = |args: &[&str]| run_git_string(cwd, args).unwrap_or_default();
    let mut summary = run(&["diff", "--cached", "--stat", "--", "."]);
    let mut patch = run(&["diff", "--cached", "--no-ext-diff", "--", "."]);
    if summary.trim().is_empty() && patch.trim().is_empty() {
        summary = run(&["diff", "HEAD", "--stat", "--", "."]);
        patch = run(&["diff", "HEAD", "--no-ext-diff", "--", "."]);
        let untracked = run(&["ls-files", "--others", "--exclude-standard"]);
        if !untracked.trim().is_empty() {
            if !summary.trim().is_empty() {
                summary.push('\n');
            }
            summary.push_str("Untracked files:\n");
            summary.push_str(untracked.trim());
        }
    }
    if summary.trim().is_empty() && patch.trim().is_empty() {
        return Err("No changes to summarize".into());
    }
    Ok(StagedContext {
        branch: stdout(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        summary,
        patch,
    })
}

/// MonoCode `git_range_context`: what a pull request is written from.
pub struct RangeContext {
    pub base: String,
    pub head: String,
    pub commit_summary: String,
    pub diff_summary: String,
    pub diff_patch: String,
}

pub fn range_context(cwd: &str) -> Result<RangeContext, String> {
    let head = stdout(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok_or_else(|| "Not on a branch".to_string())?;
    let remote = remote_name(cwd);
    let base = default_branch(cwd, remote.as_deref())
        .ok_or_else(|| "Could not resolve the default branch".to_string())?;
    let base_ref = match &remote {
        Some(remote) if ref_exists(cwd, &format!("refs/remotes/{remote}/{base}")) => {
            format!("{remote}/{base}")
        }
        _ => base.clone(),
    };
    let spec = format!("{base_ref}...HEAD");
    let run = |args: &[&str]| run_git_string(cwd, args).unwrap_or_default();
    let commit_summary = run(&["log", "--format=%s", &format!("{base_ref}..HEAD")]);
    let diff_summary = run(&["diff", "--stat", &spec]);
    let diff_patch = run(&["diff", "--no-ext-diff", &spec]);
    if commit_summary.trim().is_empty() && diff_patch.trim().is_empty() {
        return Err("No commits to include in a pull request".into());
    }
    Ok(RangeContext {
        base,
        head,
        commit_summary,
        diff_summary,
        diff_patch,
    })
}

// ---------------------------------------------------------------------------
// The branch's pull request, through `gh` (MonoCode `git_pr_status`).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPr {
    pub number: i64,
    pub title: String,
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
}

/// The Inbox's runner (`github::gh`): finds `gh` on PATH first.
fn gh(cwd: &str, args: &[&str]) -> Result<String, String> {
    crate::github::gh(Path::new(cwd), args)
}

/// MonoCode `parse_gh_pr_list`: the open one, else the latest, among
/// those whose head is in `owner`'s repository (a fork's branch of the same
/// name is someone else's).
fn parse_pr_list(json: &str, owner: &str) -> Option<BranchPr> {
    #[derive(Deserialize)]
    struct Owner {
        login: String,
    }
    #[derive(Deserialize)]
    struct Row {
        number: i64,
        title: String,
        url: String,
        state: String,
        #[serde(rename = "headRepositoryOwner")]
        head_owner: Option<Owner>,
    }
    let rows: Vec<Row> = serde_json::from_str(json).ok()?;
    let prs: Vec<BranchPr> = rows
        .into_iter()
        .filter(|r| {
            r.head_owner
                .as_ref()
                .is_some_and(|o| o.login.eq_ignore_ascii_case(owner))
        })
        .map(|r| BranchPr {
            number: r.number,
            title: r.title,
            url: r.url,
            state: r.state.to_lowercase(),
        })
        .collect();
    prs.iter()
        .find(|p| p.state == "open")
        .or(prs.first())
        .cloned()
}

/// The latest pull request whose head is this branch.
pub fn pr_status(cwd: &str) -> Option<BranchPr> {
    let branch = stdout(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    #[derive(Deserialize)]
    struct View {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let view: View =
        serde_json::from_str(&gh(cwd, &["repo", "view", "--json", "nameWithOwner"]).ok()?).ok()?;
    let owner = view.name_with_owner.split_once('/')?.0;
    // `--head` takes the bare branch: gh finds nothing for `owner:branch`.
    let json = gh(
        cwd,
        &[
            "pr",
            "list",
            "--head",
            &branch,
            "--json",
            "number,title,url,state,headRepositoryOwner",
            "--limit",
            "20",
            "--state",
            "all",
        ],
    )
    .ok()?;
    parse_pr_list(&json, owner)
}

/// MonoCode `git_pr_create`: returns the new pull request's URL.
pub fn pr_create(
    cwd: &str,
    title: &str,
    body: &str,
    base: &str,
    head: &str,
) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Pull request title cannot be empty".into());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let body_path = std::env::temp_dir().join(format!("bencode-pr-{stamp}.md"));
    std::fs::write(&body_path, body.trim()).map_err(|err| err.to_string())?;
    let result = gh(
        cwd,
        &[
            "pr",
            "create",
            "--title",
            title,
            "--body-file",
            &body_path.to_string_lossy(),
            "--base",
            base.trim(),
            "--head",
            head.trim(),
        ],
    );
    if let Err(err) = std::fs::remove_file(&body_path) {
        log::warn!("could not remove {}: {err}", body_path.display());
    }
    let output = result?;
    output
        .lines()
        .rev()
        .find(|l| l.starts_with("http://") || l.starts_with("https://"))
        .map(|l| l.trim().to_string())
        .ok_or_else(|| {
            if output.trim().is_empty() {
                "gh returned no pull request URL".into()
            } else {
                output
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorations_split_heads_tags_and_remotes() {
        let remotes = vec!["origin".to_string()];
        let (head, refs) = parse_decorations(
            "HEAD -> main, origin/main, origin/HEAD, tag: v1.0, feature",
            None,
            "abc",
            &remotes,
        );
        assert!(head);
        let pairs: Vec<_> = refs
            .iter()
            .map(|r| (r.name.as_str(), r.kind.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("main", "local"),
                ("origin/main", "remote"),
                ("v1.0", "tag"),
                ("feature", "local")
            ]
        );
    }

    #[test]
    fn history_records_parse() {
        let text = "aaa\0a\0bbb ccc\0Ann\x001700000000\0HEAD -> main\0Merge it\x1e\nbbb\0b\0\0Bob\x001600000000\0\0First\x1e";
        let commits = parse_history(text, Some("aaa"), &[]);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, ["bbb", "ccc"]);
        assert!(commits[0].head);
        assert_eq!(commits[1].parents.len(), 0);
        assert_eq!(commits[1].subject, "First");
    }

    #[test]
    fn pr_list_prefers_open() {
        let json = r#"[{"number":2,"title":"Old","url":"u2","state":"MERGED","headRepositoryOwner":{"login":"me"}},{"number":3,"title":"Now","url":"u3","state":"OPEN","headRepositoryOwner":{"login":"Me"}}]"#;
        let pr = parse_pr_list(json, "me").unwrap();
        assert_eq!((pr.number, pr.state.as_str()), (3, "open"));
        assert!(parse_pr_list("[]", "me").is_none());
    }

    #[test]
    fn pr_list_skips_forks_branches() {
        let json = r#"[{"number":4,"title":"Fork","url":"u4","state":"OPEN","headRepositoryOwner":{"login":"other"}},{"number":1,"title":"Mine","url":"u1","state":"MERGED","headRepositoryOwner":{"login":"me"}}]"#;
        let pr = parse_pr_list(json, "me").unwrap();
        assert_eq!((pr.number, pr.state.as_str()), (1, "merged"));
    }

    #[test]
    fn sync_info_reads_a_fresh_repo() {
        let dir = std::env::temp_dir().join(format!("bencode-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let git = |args: &[&str]| {
            assert!(
                git_command(&cwd)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "git {args:?}"
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "."]);
        commit(&cwd, "first", false).unwrap();
        commit(&cwd, "first, amended", true).unwrap();
        assert_eq!(head_message(&cwd).unwrap(), "first, amended");
        assert!(commit(&cwd, "  ", false).is_err());
        let info = sync_info(&cwd);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.default_branch.as_deref(), Some("main"));
        assert!(info.remote.is_none() && !info.head_pushed);
        assert_eq!(history(&cwd).len(), 1);
        // A remote never fetched: its default branch is a tip that is not
        // there, which the log must skip, not fail on.
        git(&[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ]);
        let base = RepoBase::read(&cwd);
        assert_eq!(base.default_ref().as_deref(), Some("origin/main"));
        assert_eq!(history_from(&cwd, &base).len(), 1);
        let info = sync_info_from(&cwd, &base);
        assert_eq!(info.remote.as_deref(), Some("origin"));
        assert_eq!((info.ahead, info.behind, info.ahead_of_default), (0, 0, 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checkout_root_strips_the_prefix_folders() {
        assert_eq!(strip_prefix_dirs("/repo", ""), "/repo");
        assert_eq!(strip_prefix_dirs("/repo/", ""), "/repo");
        assert_eq!(strip_prefix_dirs("/repo/sub", "sub/"), "/repo");
        assert_eq!(strip_prefix_dirs("/repo/a/b/", "a/b/"), "/repo");
        assert_eq!(
            strip_prefix_dirs("remote://box/home/me/repo/sub", "sub/"),
            "remote://box/home/me/repo"
        );
        // A prefix that is not `cwd`'s tail (a symlinked folder) keeps `cwd`.
        assert_eq!(strip_prefix_dirs("/link", "real/"), "/link");
        assert_eq!(strip_prefix_dirs("/xsub", "sub/"), "/xsub");
    }
}
