//! MonoCode `LinkedWorkItem` (`session.ts`, `sessionWorkItem.ts`): the
//! issue or pull request a thread is linked to, stored as JSON in
//! `sessions.linked_work_item_json`.
//!
//! MonoCode's shape is `{kind, repo, number, url}`, a GitHub item. A
//! Backlog issue (BenCode's own) adds `provider: "backlog"`, its key as
//! `identifier` and the space's `host`; its `repo` is the project key,
//! which has no `/`, so MonoCode reads such a row as unlinked rather than
//! as a GitHub item. A row without `provider` is GitHub, as before.

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::AppDb;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkItemKind {
    Issue,
    Pr,
}

/// `provider` of a Backlog link.
const BACKLOG: &str = "backlog";
/// The domains Backlog spaces live under.
const BACKLOG_DOMAINS: [&str; 3] = [".backlog.jp", ".backlog.com", ".backlogtool.com"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedWorkItem {
    pub kind: WorkItemKind,
    /// GitHub: `owner/name`, case kept. Backlog: the project key.
    pub repo: String,
    pub number: i64,
    pub url: String,
    /// `backlog`; absent for GitHub (MonoCode's shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    /// A Backlog issue's key, `PROJ-42`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identifier: Option<String>,
    /// A Backlog space's host, `acme.backlog.com`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
}

fn repo_ok(repo: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    };
    repo.split_once('/')
        .is_some_and(|(owner, name)| ok(owner) && ok(name))
}

/// A Backlog issue key: `^[A-Z][A-Z0-9_]*-[0-9]+$`; its project and number.
fn backlog_key(key: &str) -> Option<(&str, i64)> {
    let (project, number) = key.rsplit_once('-')?;
    let mut chars = project.chars();
    let project_ok = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    let number_ok = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    if !project_ok || !number_ok {
        return None;
    }
    let number = number.parse::<i64>().ok().filter(|n| *n > 0)?;
    Some((project, number))
}

/// A Backlog space's host: one label under one of Backlog's domains.
fn backlog_host_ok(host: &str) -> bool {
    BACKLOG_DOMAINS.iter().any(|domain| {
        host.strip_suffix(domain).is_some_and(|space| {
            !space.is_empty()
                && !space.starts_with('-')
                && !space.ends_with('-')
                && space
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
    })
}

impl LinkedWorkItem {
    /// A checked `owner/name` and number, its URL built from them.
    pub fn new(kind: WorkItemKind, repo: &str, number: i64) -> Option<Self> {
        let repo = repo.trim();
        if !repo_ok(repo) || number <= 0 {
            return None;
        }
        let path = match kind {
            WorkItemKind::Pr => "pull",
            WorkItemKind::Issue => "issues",
        };
        Some(Self {
            url: format!("https://github.com/{repo}/{path}/{number}"),
            kind,
            repo: repo.to_string(),
            number,
            provider: None,
            identifier: None,
            host: None,
        })
    }

    /// A checked Backlog issue key on a checked space host, its URL built
    /// from them.
    pub fn backlog(key: &str, host: &str) -> Option<Self> {
        let (key, host) = (key.trim(), host.trim().to_ascii_lowercase());
        let (project, number) = backlog_key(key)?;
        if !backlog_host_ok(&host) {
            return None;
        }
        Some(Self {
            kind: WorkItemKind::Issue,
            repo: project.to_string(),
            number,
            url: format!("https://{host}/view/{key}"),
            provider: Some(BACKLOG.to_string()),
            identifier: Some(key.to_string()),
            host: Some(host),
        })
    }

    pub fn is_backlog(&self) -> bool {
        self.provider.as_deref() == Some(BACKLOG)
    }

    /// A Backlog issue's space host.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// How the tracker names it: `#42`, `PROJ-42`.
    pub fn identifier(&self) -> String {
        match &self.identifier {
            Some(key) => key.clone(),
            None => format!("#{}", self.number),
        }
    }

    /// The link an Inbox key (`WorkItem::key`) stands for, to fetch the
    /// item by: a Backlog key needs the connected space's `host`. A
    /// GitHub key keeps its repository lower case, which GitHub ignores.
    pub fn from_item_key(key: &str, host: Option<&str>) -> Option<Self> {
        if let Some(issue) = key.strip_prefix("backlog:") {
            return Self::backlog(&issue.to_ascii_uppercase(), host?);
        }
        let mut parts = key.rsplitn(3, ':');
        let number = parts.next()?.parse().ok()?;
        let kind = match parts.next()? {
            "pr" => WorkItemKind::Pr,
            "issue" => WorkItemKind::Issue,
            _ => return None,
        };
        Self::new(kind, parts.next()?, number)
    }

    /// The Inbox's key for the item (`WorkItem::key`).
    pub fn item_key(&self) -> String {
        match &self.identifier {
            Some(key) => format!("backlog:{}", key.to_lowercase()),
            None => format!(
                "{}:{}:{}",
                self.repo.to_lowercase(),
                match self.kind {
                    WorkItemKind::Pr => "pr",
                    WorkItemKind::Issue => "issue",
                },
                self.number
            ),
        }
    }

    /// MonoCode `parseGithubWorkItemUrl`: a github.com issue or pull URL
    /// anywhere in `text`, rebuilt canonically.
    pub fn parse_url(text: &str) -> Option<Self> {
        github_url(text).map(|(_, item)| item)
    }

    /// A Backlog issue URL (`https://<host>/view/KEY`) anywhere in `text`,
    /// on `host` when one is given (the connected space), else on any
    /// Backlog space; rebuilt canonically.
    pub fn parse_backlog_url(text: &str, host: Option<&str>) -> Option<Self> {
        backlog_url(text, host).map(|(_, item)| item)
    }

    /// The first GitHub or Backlog issue URL in `text` (`host` as in
    /// `parse_backlog_url`).
    pub fn parse_any_url(text: &str, host: Option<&str>) -> Option<Self> {
        match (github_url(text), backlog_url(text, host)) {
            (Some((a, github)), Some((b, backlog))) => Some(if a < b { github } else { backlog }),
            (github, backlog) => github.or(backlog).map(|(_, item)| item),
        }
    }

    /// MonoCode `sanitizeLinkedWorkItem`: stored JSON, checked, its URL
    /// rebuilt; anything wrong reads as unlinked.
    pub fn from_json(text: &str) -> Option<Self> {
        let raw: serde_json::Value = serde_json::from_str(text).ok()?;
        match raw.get("provider").map(|p| p.as_str()) {
            None => {}
            Some(Some(BACKLOG)) => {
                return Self::backlog(raw.get("identifier")?.as_str()?, raw.get("host")?.as_str()?);
            }
            Some(_) => return None,
        }
        let kind = match raw.get("kind")?.as_str()? {
            "pr" => WorkItemKind::Pr,
            "issue" => WorkItemKind::Issue,
            _ => return None,
        };
        let number = raw.get("number")?.as_i64()?;
        Self::new(kind, raw.get("repo")?.as_str()?, number)
    }

    /// MonoCode's badge noun.
    pub fn noun(&self) -> &'static str {
        match self.kind {
            WorkItemKind::Pr => "PR",
            WorkItemKind::Issue => "issue",
        }
    }
}

/// The first github.com issue or pull URL in `text`, and where it starts.
fn github_url(text: &str) -> Option<(usize, LinkedWorkItem)> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("github.com/") {
        let at = from + found;
        let start = at + "github.com/".len();
        from = start;
        let scheme_ok = lower[..at].ends_with("https://") || lower[..at].ends_with("http://");
        if !scheme_ok {
            continue;
        }
        let rest = &text[start..];
        let mut parts = rest.split('/');
        let (Some(owner), Some(name), Some(kind), Some(tail)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let kind = match kind.to_ascii_lowercase().as_str() {
            "pull" => WorkItemKind::Pr,
            "issues" => WorkItemKind::Issue,
            _ => continue,
        };
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        // `\\b`: the number must end at a non-word character.
        let next = tail[digits.len()..].chars().next();
        if digits.is_empty() || next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Ok(number) = digits.parse::<i64>() else {
            continue;
        };
        if let Some(item) = LinkedWorkItem::new(kind, &format!("{owner}/{name}"), number) {
            return Some((at, item));
        }
    }
    None
}

/// The first Backlog issue URL in `text` (on `host`, when given), and
/// where it starts.
fn backlog_url(text: &str, host: Option<&str>) -> Option<(usize, LinkedWorkItem)> {
    let host = host.map(str::to_ascii_lowercase);
    let mut from = 0;
    while let Some(found) = text[from..].find("/view/").map(|at| from + at) {
        from = found + "/view/".len();
        // The scheme nearest the path is the URL's own; what lies between
        // must be a host.
        let before = text[..found].to_ascii_lowercase();
        let Some(scheme) = before.rfind("https://") else {
            continue;
        };
        let url_host = &before[scheme + "https://".len()..];
        if host.as_deref().is_some_and(|host| host != url_host) {
            continue;
        }
        let key: String = text[from..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if let Some(item) = LinkedWorkItem::backlog(&key, url_host) {
            return Some((scheme, item));
        }
    }
    None
}

impl AppDb {
    /// MonoCode `set_linked_work_item`; `None` unlinks.
    pub fn set_linked_work_item(
        &self,
        session_id: &str,
        item: Option<&LinkedWorkItem>,
    ) -> Result<()> {
        let json = item.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "UPDATE sessions SET linked_work_item_json = ?1 WHERE id = ?2",
            params![json, session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_keys_lead_back_to_their_links() {
        let pr = LinkedWorkItem::new(WorkItemKind::Pr, "Acme/App", 182).unwrap();
        let back = LinkedWorkItem::from_item_key(&pr.item_key(), None).unwrap();
        assert_eq!((back.repo.as_str(), back.number), ("acme/app", 182));
        assert_eq!(back.kind, WorkItemKind::Pr);
        assert_eq!(back.item_key(), pr.item_key());
        let issue = LinkedWorkItem::backlog("PROJ_D-7", "acme.backlog.jp").unwrap();
        let back = LinkedWorkItem::from_item_key(&issue.item_key(), Some("acme.backlog.jp"));
        assert_eq!(back.as_ref(), Some(&issue));
        // A Backlog key cannot be fetched without the space it is in.
        assert!(LinkedWorkItem::from_item_key("backlog:proj_d-7", None).is_none());
        assert!(LinkedWorkItem::from_item_key("o/r:thing:1", None).is_none());
        assert!(LinkedWorkItem::from_item_key("nonsense", None).is_none());
    }

    #[test]
    fn urls_parse_like_monocode() {
        let pr = LinkedWorkItem::parse_url("see https://github.com/Owner/repo.rs/pull/12/files")
            .unwrap();
        assert_eq!(pr.kind, WorkItemKind::Pr);
        assert_eq!(pr.repo, "Owner/repo.rs");
        assert_eq!(pr.url, "https://github.com/Owner/repo.rs/pull/12");
        let issue =
            LinkedWorkItem::parse_url("HTTPS://GITHUB.COM/a/b/issues/7#issuecomment-1").unwrap();
        assert_eq!((issue.kind, issue.number), (WorkItemKind::Issue, 7));
        assert!(LinkedWorkItem::parse_url("https://github.com/a/b/pulls/7").is_none());
        assert!(LinkedWorkItem::parse_url("https://github.com/a/b/pull/0").is_none());
        assert!(LinkedWorkItem::parse_url("https://github.com/a/b/pull/7x").is_none());
        assert!(LinkedWorkItem::parse_url("#123").is_none());
    }

    #[test]
    fn stored_links_are_checked_and_round_trip() {
        let item = LinkedWorkItem::parse_url("https://github.com/a/b/pull/3").unwrap();
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(LinkedWorkItem::from_json(&json), Some(item));
        let forged = r#"{"kind":"pr","repo":"a/b","number":3,"url":"https://evil.example"}"#;
        assert_eq!(
            LinkedWorkItem::from_json(forged).unwrap().url,
            "https://github.com/a/b/pull/3"
        );
        assert!(
            LinkedWorkItem::from_json(r#"{"kind":"pr","repo":"bad repo","number":3}"#).is_none()
        );
    }

    #[test]
    fn monocode_rows_read_as_github_links() {
        let stored = r#"{"kind":"issue","repo":"Owner/Repo","number":9,"url":"https://github.com/Owner/Repo/issues/9"}"#;
        let item = LinkedWorkItem::from_json(stored).unwrap();
        assert!(!item.is_backlog());
        assert_eq!(item.identifier(), "#9");
        assert_eq!(item.item_key(), "owner/repo:issue:9");
        // Written back in MonoCode's shape: no field of BenCode's own.
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::from_str::<serde_json::Value>(stored).unwrap()
        );
        assert!(
            LinkedWorkItem::from_json(
                r#"{"provider":"jira","kind":"issue","repo":"a/b","number":1}"#
            )
            .is_none()
        );
    }

    #[test]
    fn backlog_keys_and_hosts_are_checked() {
        let item = LinkedWorkItem::backlog("ACME_SHOP_C-289", "Acme.Backlog.JP").unwrap();
        assert!(item.is_backlog());
        assert_eq!(item.repo, "ACME_SHOP_C");
        assert_eq!(item.number, 289);
        assert_eq!(item.url, "https://acme.backlog.jp/view/ACME_SHOP_C-289");
        assert_eq!(item.identifier(), "ACME_SHOP_C-289");
        assert_eq!(item.item_key(), "backlog:acme_shop_c-289");
        for host in ["a.backlog.com", "a-1.backlogtool.com"] {
            assert!(LinkedWorkItem::backlog("P2-1", host).is_some(), "{host}");
        }
        for key in ["proj-1", "1P-1", "P-0", "P-", "P-1a", "P 1", "-1", "P_X"] {
            assert!(
                LinkedWorkItem::backlog(key, "a.backlog.com").is_none(),
                "{key}"
            );
        }
        for host in [
            "backlog.com",
            "a.backlog.com.evil.io",
            "evil.io",
            "a.b.backlog.com",
            "-a.backlog.jp",
            "a.backlog.com/x",
        ] {
            assert!(LinkedWorkItem::backlog("P-1", host).is_none(), "{host}");
        }
    }

    #[test]
    fn backlog_rows_round_trip_and_forged_urls_are_rebuilt() {
        let item = LinkedWorkItem::backlog("WEB-42", "acme.backlog.com").unwrap();
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(LinkedWorkItem::from_json(&json), Some(item.clone()));
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["provider"], "backlog");
        assert_eq!(raw["kind"], "issue");
        // MonoCode's check wants `owner/name`: a project key is no repo.
        assert!(!repo_ok(raw["repo"].as_str().unwrap()));
        let forged = r#"{"provider":"backlog","kind":"pr","repo":"x/y","number":1,"identifier":"WEB-42","host":"acme.backlog.com","url":"https://evil.example"}"#;
        assert_eq!(LinkedWorkItem::from_json(forged), Some(item));
        let bad_host = r#"{"provider":"backlog","identifier":"WEB-42","host":"evil.example"}"#;
        assert!(LinkedWorkItem::from_json(bad_host).is_none());
    }

    #[test]
    fn backlog_urls_parse_on_the_connected_space() {
        let text = "Ticket: https://acme.backlog.com/view/WEB-42#comment-7 please";
        let item = LinkedWorkItem::parse_backlog_url(text, Some("acme.backlog.com")).unwrap();
        assert_eq!(item.url, "https://acme.backlog.com/view/WEB-42");
        assert!(LinkedWorkItem::parse_backlog_url(text, Some("other.backlog.com")).is_none());
        assert!(LinkedWorkItem::parse_backlog_url(text, None).is_some());
        for text in [
            "https://acme.backlog.com/view/WEB-42x",
            "https://acme.backlog.com/view/web-42",
            "https://evil.io/view/WEB-42",
            "https://acme.backlog.com/x/view/WEB-42",
            "acme.backlog.com/view/WEB-42",
        ] {
            assert!(
                LinkedWorkItem::parse_backlog_url(text, None).is_none(),
                "{text}"
            );
        }
        // The first of either kind wins.
        let both = "https://acme.backlog.com/view/WEB-1 and https://github.com/a/b/pull/2";
        assert!(
            LinkedWorkItem::parse_any_url(both, None)
                .unwrap()
                .is_backlog()
        );
        let both = "https://github.com/a/b/pull/2 and https://acme.backlog.com/view/WEB-1";
        assert!(
            !LinkedWorkItem::parse_any_url(both, None)
                .unwrap()
                .is_backlog()
        );
    }

    #[test]
    fn links_are_saved_and_cleared() {
        let db = AppDb::open_in_memory().unwrap();
        let session = crate::db::SessionRow {
            id: "s".into(),
            cwd: "/p".into(),
            harness: "claude".into(),
            ..Default::default()
        };
        db.upsert_session(&session).unwrap();
        let item = LinkedWorkItem::parse_url("https://github.com/a/b/issues/9").unwrap();
        db.set_linked_work_item("s", Some(&item)).unwrap();
        assert_eq!(
            db.get_session("s").unwrap().unwrap().linked_work_item,
            Some(item.clone())
        );
        db.upsert_session(&db.get_session("s").unwrap().unwrap())
            .unwrap();
        assert_eq!(
            db.get_session("s").unwrap().unwrap().linked_work_item,
            Some(item),
            "saves keep it"
        );
        db.set_linked_work_item("s", None).unwrap();
        assert_eq!(db.get_session("s").unwrap().unwrap().linked_work_item, None);
    }
}
