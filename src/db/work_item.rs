//! MonoCode `LinkedWorkItem` (`session.ts`, `sessionWorkItem.ts`): the
//! GitHub issue or pull request a thread is linked to, stored as JSON in
//! `sessions.linked_work_item_json`.

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedWorkItem {
    pub kind: WorkItemKind,
    /// `owner/name`, case kept.
    pub repo: String,
    pub number: i64,
    pub url: String,
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

impl LinkedWorkItem {
    fn new(kind: WorkItemKind, repo: &str, number: i64) -> Option<Self> {
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
        })
    }

    /// MonoCode `parseGithubWorkItemUrl`: a github.com issue or pull URL
    /// anywhere in `text`, rebuilt canonically.
    pub fn parse_url(text: &str) -> Option<Self> {
        let lower = text.to_ascii_lowercase();
        let mut from = 0;
        while let Some(found) = lower[from..].find("github.com/") {
            let start = from + found + "github.com/".len();
            from = start;
            let scheme_ok = lower[..start - "github.com/".len()].ends_with("https://")
                || lower[..start - "github.com/".len()].ends_with("http://");
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
            if let Some(item) = Self::new(kind, &format!("{owner}/{name}"), number) {
                return Some(item);
            }
        }
        None
    }

    /// MonoCode `sanitizeLinkedWorkItem`: stored JSON, checked, its URL
    /// rebuilt; anything wrong reads as unlinked.
    pub fn from_json(text: &str) -> Option<Self> {
        let raw: serde_json::Value = serde_json::from_str(text).ok()?;
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
