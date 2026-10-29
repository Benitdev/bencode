//! BenCode's own "Needs you": an item needs the user when its tracker
//! addressed a notification to them about it after they last read it in
//! BenCode, or when it is assigned to them and unread. "Unread" alone means
//! "updated since", which teammates make true of nearly every ticket all
//! day; this says which of them are the user's to answer.
//!
//! The notifications are the trackers' own: GitHub's threads the user
//! participates in, Backlog's notifications to the user. They are only
//! read, never marked read. This module maps them to item keys and keeps
//! what each tracker said last; asking is `app/inbox_attention.rs`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use super::tickets::ticket_item_key;
use super::{Provider, WorkItem};

/// The least time between two asks of one tracker; GitHub may ask for
/// more (`X-Poll-Interval`).
pub const MIN_POLL: Duration = Duration::from_secs(60);

/// A notification about item `key`, at `at_ms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notified {
    pub key: String,
    pub at_ms: i64,
}

/// What one ask of a tracker found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub notified: Vec<Notified>,
    /// Who the user is there: GitHub's login by project folder (accounts
    /// differ per project), Backlog's name under `""`.
    pub me: HashMap<String, String>,
    /// Some of the asks failed: what they found before is kept beside
    /// this.
    pub partial: bool,
    /// The tracker's own least time before the next ask.
    pub poll: Duration,
}

/// What one tracker said last.
#[derive(Debug, Default)]
struct Source {
    /// The newest notification per item key.
    notified: HashMap<String, i64>,
    me: HashMap<String, String>,
    /// Not asked again before this.
    next: Option<Instant>,
}

impl Source {
    fn land(&mut self, found: Option<Found>, now: Instant) -> bool {
        let Some(found) = found else {
            // A failed ask: what it said before stands, and it waits.
            self.next = Some(now + MIN_POLL);
            return false;
        };
        self.next = Some(now + found.poll.max(MIN_POLL));
        let mut notified = if found.partial {
            self.notified.clone()
        } else {
            HashMap::new()
        };
        for Notified { key, at_ms } in found.notified {
            let at = notified.entry(key).or_insert(at_ms);
            *at = (*at).max(at_ms);
        }
        let mut me = if found.partial {
            self.me.clone()
        } else {
            HashMap::new()
        };
        me.extend(found.me);
        let changed = notified != self.notified || me != self.me;
        self.notified = notified;
        self.me = me;
        changed
    }
}

/// The trackers' notifications and who the user is on each.
#[derive(Debug, Default)]
pub struct Attention {
    github: Source,
    backlog: Source,
    /// An ask is running.
    pub fetching: bool,
}

impl Attention {
    fn source(&self, provider: Provider) -> &Source {
        match provider {
            Provider::GitHub => &self.github,
            Provider::Backlog => &self.backlog,
        }
    }

    /// Whether `provider` may be asked again at `now`.
    pub fn due(&self, provider: Provider, now: Instant) -> bool {
        self.source(provider).next.is_none_or(|next| now >= next)
    }

    /// What an ask of `provider` found (`None`: it failed). Whether that
    /// changed anything shown.
    pub fn land(&mut self, provider: Provider, found: Option<Found>, now: Instant) -> bool {
        match provider {
            Provider::GitHub => self.github.land(found, now),
            Provider::Backlog => self.backlog.land(found, now),
        }
    }

    /// The newest notification about item `key`, from either tracker.
    pub fn notified_at(&self, key: &str) -> Option<i64> {
        let github = self.github.notified.get(key);
        let backlog = self.backlog.notified.get(key);
        github.into_iter().chain(backlog).copied().max()
    }

    /// `item` is assigned to the user as its tracker knows them.
    pub fn assigned_to_me(&self, item: &WorkItem) -> bool {
        let me = match item.provider {
            Provider::GitHub => self.github.me.get(&item.project),
            Provider::Backlog => self.backlog.me.get(""),
        };
        me.is_some_and(|me| {
            !me.is_empty() && (item.assignees.iter()).any(|a| a.eq_ignore_ascii_case(me))
        })
    }
}

/// The rule: a notification newer than the last read (`seen`, none when
/// never read), or assigned to the user and unread.
pub fn needs_you(notified: Option<i64>, assigned: bool, seen: Option<i64>, unseen: bool) -> bool {
    let notified_since = notified.is_some_and(|at| seen.is_none_or(|read| at > read));
    notified_since || (assigned && unseen)
}

/// The item key a GitHub notification's `subject.url` names:
/// `https://api.github.com/repos/o/r/pulls/12` is `o/r:pr:12`. Releases,
/// discussions and commits are no item.
pub fn github_subject_key(url: &str) -> Option<String> {
    let path = url.split_once("/repos/")?.1;
    let mut parts = path.split('/');
    let (owner, name, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || owner.is_empty() || name.is_empty() {
        return None;
    }
    let kind = match kind {
        "pulls" => "pr",
        "issues" => "issue",
        _ => return None,
    };
    let number: u64 = number.parse().ok()?;
    Some(format!("{}/{}:{kind}:{number}", owner, name).to_lowercase())
}

/// `gh api -i`'s answer split into its headers and its body.
pub fn split_response(out: &str) -> (&str, &str) {
    let mut at = 0;
    for line in out.split_inclusive('\n') {
        at += line.len();
        if line.trim().is_empty() {
            return (&out[..at], &out[at..]);
        }
    }
    ("", out)
}

/// GitHub's `X-Poll-Interval` (seconds) in `headers`, at least
/// [`MIN_POLL`].
pub fn poll_interval(headers: &str) -> Duration {
    let asked = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("x-poll-interval")
            .then(|| value.trim().parse::<u64>().ok())
            .flatten()
    });
    asked.map_or(MIN_POLL, |secs| Duration::from_secs(secs).max(MIN_POLL))
}

fn ms(iso: &str) -> Option<i64> {
    iso.parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_millisecond())
}

/// GitHub's notifications (`GET /notifications`) about issues and pull
/// requests of `repos` (`owner/name`, lower case).
pub fn parse_github(json: &str, repos: &HashSet<String>) -> Result<Vec<Notified>, String> {
    #[derive(Deserialize)]
    struct Subject {
        #[serde(default)]
        url: Option<String>,
    }
    #[derive(Deserialize)]
    struct Thread {
        subject: Subject,
        #[serde(default)]
        updated_at: String,
    }
    let threads: Vec<Thread> = serde_json::from_str(json)
        .map_err(|_| "GitHub sent unreadable notifications".to_string())?;
    Ok(threads
        .into_iter()
        .filter_map(|thread| {
            let key = github_subject_key(thread.subject.url.as_deref()?)?;
            let repo = key.split_once(':')?.0;
            repos.contains(repo).then_some(())?;
            Some(Notified {
                key,
                at_ms: ms(&thread.updated_at)?,
            })
        })
        .collect())
}

/// Backlog's notifications (`GET /api/v2/notifications`) as item keys:
/// an issue's own, or the issue a pull request is linked to. Whether the
/// item is listed is the Inbox's to check.
pub fn parse_backlog(data: &Value) -> Vec<Notified> {
    let key = |row: &Value| {
        let issue_key = |issue: Option<&Value>| {
            issue?
                .get("issueKey")?
                .as_str()
                .filter(|key| !key.is_empty())
                .map(str::to_string)
        };
        issue_key(row.get("issue"))
            .or_else(|| issue_key(row.get("pullRequest").and_then(|pr| pr.get("issue"))))
    };
    (data.as_array().into_iter().flatten())
        .filter_map(|row| {
            let at_ms = ms(row.get("created")?.as_str()?)?;
            Some(Notified {
                key: ticket_item_key(&key(row)?),
                at_ms,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_subjects_map_to_item_keys() {
        let key = |url| github_subject_key(url);
        assert_eq!(
            key("https://api.github.com/repos/Acme/Shop-Backend/pulls/483").as_deref(),
            Some("acme/shop-backend:pr:483")
        );
        assert_eq!(
            key("https://api.github.com/repos/o/r/issues/12").as_deref(),
            Some("o/r:issue:12")
        );
        assert_eq!(key("https://api.github.com/repos/o/r/releases/1"), None);
        assert_eq!(key("https://api.github.com/repos/o/r/commits/abc"), None);
        assert_eq!(
            key("https://api.github.com/repos/o/r/pulls/12/reviews"),
            None
        );
        assert_eq!(key("https://api.github.com/repos/o/r/pulls/x"), None);
        assert_eq!(key(""), None);
    }

    #[test]
    fn github_notifications_keep_listed_repositories() {
        let json = r#"[
            {"reason":"review_requested","updated_at":"2026-10-28T10:00:00Z",
             "subject":{"type":"PullRequest","url":"https://api.github.com/repos/O/R/pulls/7"}},
            {"reason":"mention","updated_at":"2026-10-28T11:00:00Z",
             "subject":{"type":"Issue","url":"https://api.github.com/repos/other/x/issues/1"}},
            {"reason":"author","updated_at":"2026-10-28T12:00:00Z",
             "subject":{"type":"Release","url":"https://api.github.com/repos/o/r/releases/2"}},
            {"reason":"comment","updated_at":"2026-10-28T13:00:00Z",
             "subject":{"type":"Discussion","url":null}}
        ]"#;
        let repos = HashSet::from(["o/r".to_string()]);
        let found = parse_github(json, &repos).unwrap();
        assert_eq!(
            found,
            [Notified {
                key: "o/r:pr:7".into(),
                at_ms: ms("2026-10-28T10:00:00Z").unwrap(),
            }]
        );
        assert!(parse_github("not json", &repos).is_err());
    }

    #[test]
    fn gh_answers_split_at_the_blank_line_and_poll_no_faster_than_a_minute() {
        let out =
            "HTTP/2.0 200 OK\nContent-Type: application/json\r\nX-Poll-Interval: 120\r\n\r\n[]";
        let (headers, body) = split_response(out);
        assert_eq!(body, "[]");
        assert_eq!(poll_interval(headers), Duration::from_secs(120));
        assert_eq!(poll_interval("x-poll-interval: 30"), MIN_POLL);
        assert_eq!(poll_interval("Date: now"), MIN_POLL);
        assert_eq!(split_response("[]"), ("", "[]"));
    }

    #[test]
    fn backlog_notifications_name_issues_or_their_pull_requests_issue() {
        let data: Value = serde_json::from_str(
            r#"[
            {"reason":2,"created":"2026-10-29T02:38:16Z",
             "issue":{"issueKey":"ACME_SHOP_D-1151"}},
            {"reason":11,"created":"2026-10-29T03:00:00Z","issue":null,
             "pullRequest":{"number":4,"issue":{"issueKey":"PROJ-12"}}},
            {"reason":12,"created":"2026-10-29T04:00:00Z","issue":null,
             "pullRequest":{"number":5,"issue":null}},
            {"reason":6,"created":"2026-10-29T05:00:00Z"}
        ]"#,
        )
        .unwrap();
        let keys: Vec<String> = parse_backlog(&data).into_iter().map(|n| n.key).collect();
        assert_eq!(keys, ["backlog:acme_shop_d-1151", "backlog:proj-12"]);
        assert!(parse_backlog(&Value::Null).is_empty());
    }

    #[test]
    fn an_item_needs_you_when_notified_since_read_or_assigned_and_unread() {
        // Notified after the last read, or never read.
        assert!(needs_you(Some(20), false, Some(10), false));
        assert!(needs_you(Some(20), false, None, false));
        // Read since.
        assert!(!needs_you(Some(10), false, Some(10), false));
        assert!(!needs_you(Some(5), false, Some(10), true));
        // Assigned: only while unread.
        assert!(needs_you(None, true, Some(10), true));
        assert!(!needs_you(None, true, Some(10), false));
        assert!(!needs_you(None, false, None, true));
    }

    #[test]
    fn landed_notifications_keep_the_newest_and_failures_keep_the_last() {
        let now = Instant::now();
        let mut attention = Attention::default();
        assert!(attention.due(Provider::GitHub, now));
        let found = Found {
            notified: vec![
                Notified {
                    key: "o/r:pr:7".into(),
                    at_ms: 5,
                },
                Notified {
                    key: "o/r:pr:7".into(),
                    at_ms: 9,
                },
            ],
            me: HashMap::from([("/p".to_string(), "Ben".to_string())]),
            partial: false,
            poll: Duration::from_secs(90),
        };
        assert!(attention.land(Provider::GitHub, Some(found), now));
        assert_eq!(attention.notified_at("o/r:pr:7"), Some(9));
        assert!(!attention.due(Provider::GitHub, now + Duration::from_secs(61)));
        assert!(attention.due(Provider::GitHub, now + Duration::from_secs(90)));
        assert!(!attention.land(Provider::GitHub, None, now));
        assert_eq!(attention.notified_at("o/r:pr:7"), Some(9), "kept");

        let item = WorkItem {
            project: "/p".into(),
            assignees: vec!["ben".into()],
            ..Default::default()
        };
        assert!(attention.assigned_to_me(&item));
        let elsewhere = WorkItem {
            project: "/q".into(),
            ..item.clone()
        };
        assert!(!attention.assigned_to_me(&elsewhere));
    }
}
