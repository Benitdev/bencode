//! What a bot's chat shows of its projects when the user comes to it
//! (BenCode's own): the threads there waiting on the user (a command to
//! allow, a question) and those that finished while nobody looked. Built
//! from what BenCode already knows, so it costs no turn of the bot's and
//! no token; it saves opening each project to see how things stand.

use std::collections::HashSet;

/// How many threads it lists; the rest are counted.
pub const SHOWN: usize = 4;

/// Where a thread stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// A command waits for Allow or Deny: what it would run.
    Approval(String),
    /// The agent asked something.
    Question,
    /// Its turn ended while the user was elsewhere.
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    /// When its turn started: the same thread waiting or finishing again
    /// is news again.
    pub started_at: Option<i64>,
    pub standing: Standing,
}

impl Item {
    /// What the user set aside when they closed the digest.
    pub fn key(&self) -> String {
        let kind = match self.standing {
            Standing::Finished => "finished",
            Standing::Approval(_) | Standing::Question => "waiting",
        };
        format!(
            "{kind}:{}:{}",
            self.session_id,
            self.started_at.unwrap_or_default()
        )
    }
}

/// The threads for a bot working on `projects` (every project, while it
/// has none) that the user has not set aside: those waiting first, then
/// the finished, each as `items` gives them.
pub fn digest(
    items: Vec<Item>,
    projects: &[String],
    works_on: impl Fn(&str, &str) -> bool,
    set_aside: Option<&HashSet<String>>,
) -> Vec<Item> {
    let mut items: Vec<Item> = items
        .into_iter()
        .filter(|item| {
            projects.is_empty() || projects.iter().any(|project| works_on(project, &item.cwd))
        })
        .filter(|item| set_aside.is_none_or(|keys| !keys.contains(&item.key())))
        .collect();
    items.sort_by_key(|item| matches!(item.standing, Standing::Finished));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, cwd: &str, standing: Standing) -> Item {
        Item {
            session_id: id.into(),
            title: id.into(),
            cwd: cwd.into(),
            started_at: Some(1),
            standing,
        }
    }

    #[test]
    fn its_projects_threads_show_those_waiting_first() {
        let items = vec![
            item("done", "/app", Standing::Finished),
            item("other", "/site", Standing::Question),
            item("ask", "/app", Standing::Approval("cargo test".into())),
        ];
        let same = |a: &str, b: &str| a == b;
        let shown = digest(items.clone(), &["/app".into()], same, None);
        let ids: Vec<&str> = shown.iter().map(|i| i.session_id.as_str()).collect();
        assert_eq!(ids, ["ask", "done"]);
        assert_eq!(digest(items, &[], same, None).len(), 3, "no projects: all");
    }

    #[test]
    fn what_was_set_aside_stays_aside_until_it_is_news_again() {
        let done = item("done", "/app", Standing::Finished);
        let aside: HashSet<String> = [done.key()].into();
        let same = |a: &str, b: &str| a == b;
        assert!(digest(vec![done.clone()], &[], same, Some(&aside)).is_empty());
        let again = Item {
            started_at: Some(2),
            ..done
        };
        assert_eq!(digest(vec![again], &[], same, Some(&aside)).len(), 1);
        let waiting = item("done", "/app", Standing::Question);
        assert_ne!(
            waiting.key(),
            item("done", "/app", Standing::Finished).key()
        );
    }
}
