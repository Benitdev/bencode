//! The Inbox's items, whichever tracker they come from (MonoCode
//! `InboxItem`, `GithubWorkItemDetails`, `GitHubWorkItemComment`). Each
//! source (`github.rs`, `backlog.rs`) fills these; the views read only
//! them and ask `provider` where a source differs.

use serde::Deserialize;

pub mod attention;
pub mod tickets;
pub mod translate;

/// Where an item lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Provider {
    #[default]
    GitHub,
    Backlog,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::Backlog => "Backlog",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Issue,
    Pr,
}

impl Kind {
    pub(crate) fn arg(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "pr",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Label {
    pub name: String,
    /// `rrggbb`, with or without `#`; empty for none.
    #[serde(default)]
    pub color: String,
}

/// MonoCode `InboxItem`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkItem {
    pub provider: Provider,
    pub kind: Kind,
    /// How the tracker names it: `#42`, `PROJ-42`.
    pub identifier: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    /// GitHub: `OPEN`, `CLOSED` or `MERGED`. Backlog: the status's name.
    pub state: String,
    pub state_reason: String,
    /// The status's own colour (`#rrggbb`), where the tracker has one.
    pub state_color: String,
    /// The tracker counts the item as finished.
    pub closed: bool,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<Label>,
    /// A Backlog issue's type (Task, Bug, …), coloured; also the first of
    /// `labels`, which lists it with the categories and milestones.
    pub issue_type: Option<Label>,
    pub assignees: Vec<String>,
    pub draft: bool,
    pub priority: String,
    /// The tracker's highest priority, whatever its name in the user's
    /// language.
    pub high_priority: bool,
    /// `YYYY-MM-DD`, or empty.
    pub due_date: String,
    /// GitHub: `owner/name`. Backlog: the project key.
    pub repo: String,
    /// The tracker's id of what `repo` names (a Backlog project id).
    pub container_id: String,
    /// The project folder it was listed for; empty when the tracker is
    /// not tied to one.
    pub project: String,
    /// A pull request's head branch, as listed: it may name its ticket
    /// (`tickets.rs`).
    pub head_ref: String,
    /// A pull request's checks and review, as listed: read once when the
    /// list lands, so rows draw it without parsing.
    pub pr_status: PrStatus,
}

/// A pull request's review decision (`reviewDecision`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Review {
    #[default]
    None,
    Required,
    Approved,
    ChangesRequested,
}

impl Review {
    pub fn from_decision(decision: &str) -> Self {
        match decision.trim().to_ascii_uppercase().as_str() {
            "APPROVED" => Self::Approved,
            "CHANGES_REQUESTED" => Self::ChangesRequested,
            "REVIEW_REQUIRED" => Self::Required,
            _ => Self::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Required => "Review required",
            Self::Approved => "Approved",
            Self::ChangesRequested => "Changes requested",
        }
    }
}

/// The one mark a pull request's checks show: a failure outweighs a run
/// still going, which outweighs passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksMark {
    Passed,
    Failed,
    Running,
}

/// A pull request's checks counted (`statusCheckRollup`) and its review.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrStatus {
    pub passed: u16,
    pub failed: u16,
    pub running: u16,
    pub review: Review,
}

impl PrStatus {
    pub fn checks_mark(&self) -> Option<ChecksMark> {
        if self.failed > 0 {
            Some(ChecksMark::Failed)
        } else if self.running > 0 {
            Some(ChecksMark::Running)
        } else if self.passed > 0 {
            Some(ChecksMark::Passed)
        } else {
            None
        }
    }

    /// The counts, as a tooltip or a line reads them: "1 failed, 2 passed".
    pub fn checks_summary(&self) -> String {
        [
            (self.failed, "failed"),
            (self.running, "running"),
            (self.passed, "passed"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect::<Vec<_>>()
        .join(", ")
    }
}

impl WorkItem {
    /// Unique across sources, repositories and kinds.
    pub fn key(&self) -> String {
        match self.provider {
            Provider::GitHub => format!(
                "{}:{}:{}",
                self.repo.to_lowercase(),
                self.kind.arg(),
                self.number
            ),
            Provider::Backlog => tickets::ticket_item_key(&self.identifier),
        }
    }
}

/// MonoCode `GithubWorkItemDetails`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub body: String,
    pub author: String,
    pub base_ref: String,
    pub head_ref: String,
    pub review_decision: String,
    /// A Backlog issue's editable fields.
    pub backlog: Option<crate::backlog::IssueFields>,
}

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
    /// The tracker has more than the latest page shown.
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_sources_apart() {
        let github = WorkItem {
            repo: "Owner/Repo".into(),
            number: 7,
            ..Default::default()
        };
        assert_eq!(github.key(), "owner/repo:issue:7");
        let backlog = WorkItem {
            provider: Provider::Backlog,
            identifier: "PROJ-7".into(),
            number: 7,
            ..Default::default()
        };
        assert_eq!(backlog.key(), "backlog:proj-7");
    }

    #[test]
    fn a_failure_outweighs_runs_and_passes() {
        let status = |passed, failed, running| PrStatus {
            passed,
            failed,
            running,
            ..Default::default()
        };
        assert_eq!(status(0, 0, 0).checks_mark(), None);
        assert_eq!(status(3, 0, 0).checks_mark(), Some(ChecksMark::Passed));
        assert_eq!(status(3, 0, 1).checks_mark(), Some(ChecksMark::Running));
        assert_eq!(status(3, 1, 1).checks_mark(), Some(ChecksMark::Failed));
        assert_eq!(
            status(2, 1, 1).checks_summary(),
            "1 failed, 1 running, 2 passed"
        );
        assert_eq!(
            Review::from_decision("changes_requested"),
            Review::ChangesRequested
        );
        assert_eq!(Review::from_decision("REVIEW_REQUIRED"), Review::Required);
        assert_eq!(Review::from_decision(""), Review::None);
    }
}
