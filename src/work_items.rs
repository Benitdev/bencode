//! The Inbox's items, whichever tracker they come from (MonoCode
//! `InboxItem`, `GithubWorkItemDetails`, `GitHubWorkItemComment`). Each
//! source (`github.rs`, `backlog.rs`) fills these; the views read only
//! them and ask `provider` where a source differs.

use serde::Deserialize;

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
    pub assignees: Vec<String>,
    pub draft: bool,
    pub priority: String,
    /// `YYYY-MM-DD`, or empty.
    pub due_date: String,
    /// GitHub: `owner/name`. Backlog: the project key.
    pub repo: String,
    /// The tracker's id of what `repo` names (a Backlog project id).
    pub container_id: String,
    /// The project folder it was listed for; empty when the tracker is
    /// not tied to one.
    pub project: String,
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
            Provider::Backlog => format!("backlog:{}", self.identifier.to_lowercase()),
        }
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
}
