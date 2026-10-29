//! BenCode's own: the Backlog tickets a pull request is named after. A
//! pull request titled `ACME_SHOP_D-1147 docs(checkout): …`, or
//! pushed from `feature/acme_shop_d-1147-sync`, belongs with that
//! ticket, so the Inbox shows them together instead of as strangers. The
//! index is built once, when the list lands, never per frame.

use std::collections::HashMap;

use super::{Kind, Provider, WorkItem};

/// The Inbox's key of Backlog issue `identifier` (`PROJ-12`), as
/// `WorkItem::key` makes it.
pub fn ticket_item_key(identifier: &str) -> String {
    format!("backlog:{}", identifier.to_lowercase())
}

/// The issue keys of `projects` that `text` names, as Backlog writes them
/// (`PROJ-12`), each once, in the order they first appear. Case does not
/// matter (branches are often lower case), and a key is a whole word:
/// `PROJ-120` is not `PROJ-12`, nor is `XPROJ-12` or `X_PROJ-12`.
pub fn ticket_keys(text: &str, projects: &[String]) -> Vec<String> {
    // ASCII lowering keeps every byte where it was.
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found: Vec<(usize, String)> = Vec::new();
    for project in projects {
        let project = project.trim();
        if project.is_empty() {
            continue;
        }
        let needle = format!("{}-", project.to_ascii_lowercase());
        let mut from = 0;
        while let Some(at) = lower[from..].find(&needle) {
            let start = from + at;
            // The needle starts with an ASCII byte: the next search starts
            // on a character's boundary.
            from = start + 1;
            let joined = start.checked_sub(1).is_some_and(|before| {
                bytes[before].is_ascii_alphanumeric() || bytes[before] == b'_'
            });
            let number_at = start + needle.len();
            let digits = (bytes[number_at..].iter())
                .take_while(|b| b.is_ascii_digit())
                .count();
            let end = number_at + digits;
            let runs_on = bytes.get(end).is_some_and(|b| b.is_ascii_alphanumeric());
            if joined || digits == 0 || runs_on {
                continue;
            }
            let key = format!(
                "{}-{}",
                project.to_ascii_uppercase(),
                &lower[number_at..end]
            );
            found.push((start, key));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    let mut keys: Vec<String> = Vec::new();
    for (_, key) in found {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// `title` without the ticket key `key` it starts with (`PROJ-12 fix: …`,
/// `[PROJ-12] fix: …`), for where the ticket is already shown; whole when
/// it starts otherwise or is nothing else.
pub fn title_without_key<'a>(title: &'a str, key: &str) -> &'a str {
    let trimmed = title.trim_start();
    let (open, rest) = match trimmed.strip_prefix('[') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    let starts = rest
        .get(..key.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(key));
    let after = rest.get(key.len()..).unwrap_or("");
    let whole_word = !after.starts_with(|c: char| c.is_ascii_alphanumeric());
    if !starts || !whole_word {
        return title;
    }
    let after = if open {
        after.strip_prefix(']').unwrap_or(after)
    } else {
        after
    };
    let left = after.trim_start_matches([' ', ':', '-', '|']);
    if left.is_empty() { title } else { left }
}

/// Which listed pull requests name which tickets, both ways.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TicketLinks {
    /// A ticket's item key: its pull requests' item keys, in list order.
    prs: HashMap<String, Vec<String>>,
    /// A pull request's item key: the tickets it names (`PROJ-12`), listed
    /// or not.
    tickets: HashMap<String, Vec<String>>,
}

impl TicketLinks {
    /// The links among `items`. Tickets are looked for among the projects
    /// of `known` (the connected space's) and of the listed Backlog
    /// issues, in each pull request's title, then its head branch.
    pub fn build(items: &[WorkItem], known: &[String]) -> Self {
        let mut projects: Vec<String> = known.to_vec();
        for item in items.iter().filter(|i| i.provider == Provider::Backlog) {
            if !projects.iter().any(|p| p.eq_ignore_ascii_case(&item.repo)) {
                projects.push(item.repo.clone());
            }
        }
        let mut links = Self::default();
        if projects.is_empty() {
            return links;
        }
        let prs = (items.iter()).filter(|i| i.provider == Provider::GitHub && i.kind == Kind::Pr);
        for pr in prs {
            let mut keys = ticket_keys(&pr.title, &projects);
            for key in ticket_keys(&pr.head_ref, &projects) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
            if keys.is_empty() {
                continue;
            }
            let pr_key = pr.key();
            for key in &keys {
                let listed = links.prs.entry(ticket_item_key(key)).or_default();
                if !listed.contains(&pr_key) {
                    listed.push(pr_key.clone());
                }
            }
            links.tickets.insert(pr_key, keys);
        }
        links
    }

    /// The pull requests named after ticket `key` (an item key).
    pub fn prs(&self, key: &str) -> &[String] {
        self.prs.get(key).map_or(&[], Vec::as_slice)
    }

    /// The tickets pull request `key` (an item key) is named after.
    pub fn tickets(&self, key: &str) -> &[String] {
        self.tickets.get(key).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> Vec<String> {
        vec!["ACME_SHOP_C".into(), "ACME_SHOP_D".into()]
    }

    #[test]
    fn keys_are_found_in_titles_and_branches_whatever_their_case() {
        let keys = |text| ticket_keys(text, &projects());
        assert_eq!(
            keys("ACME_SHOP_D-1147 docs(checkout): sync the coupon code rules"),
            ["ACME_SHOP_D-1147"]
        );
        assert_eq!(keys("feature/acme_shop_d-1147-sync"), ["ACME_SHOP_D-1147"]);
        assert_eq!(keys("fix/ACME_SHOP_C-289_search"), ["ACME_SHOP_C-289"]);
        assert_eq!(keys("[ACME_SHOP_D-7] tidy"), ["ACME_SHOP_D-7"]);
    }

    #[test]
    fn keys_are_whole_words() {
        let keys = |text| ticket_keys(text, &projects());
        assert_eq!(keys("ACME_SHOP_D-11470"), ["ACME_SHOP_D-11470"]);
        assert!(keys("ACME_SHOP_D-1147a").is_empty());
        assert!(keys("XACME_SHOP_D-1147").is_empty());
        assert!(keys("ACME_SHOP_D-").is_empty());
        assert!(keys("ACME_SHOP_E-12").is_empty(), "not a known project");
        // A project whose key ends another's is not found inside it.
        let nested = vec!["SHOP_D".to_string(), "ACME_SHOP_D".to_string()];
        assert_eq!(ticket_keys("ACME_SHOP_D-5", &nested), ["ACME_SHOP_D-5"]);
    }

    #[test]
    fn several_keys_keep_their_order_once_each() {
        assert_eq!(
            ticket_keys(
                "ACME_SHOP_D-1162, acme_shop_c-289 and ACME_SHOP_D-1162 again — テスト",
                &projects()
            ),
            ["ACME_SHOP_D-1162", "ACME_SHOP_C-289"]
        );
        assert!(ticket_keys("no tickets", &[]).is_empty());
    }

    #[test]
    fn a_title_loses_the_key_it_starts_with() {
        let key = "ACME_SHOP_D-1147";
        let strip = |title| title_without_key(title, key);
        assert_eq!(
            strip("ACME_SHOP_D-1147 docs(checkout): sync"),
            "docs(checkout): sync"
        );
        assert_eq!(strip("[acme_shop_d-1147] fix: x"), "fix: x");
        assert_eq!(strip("ACME_SHOP_D-1147: fix"), "fix");
        assert_eq!(strip("ACME_SHOP_D-11470 fix"), "ACME_SHOP_D-11470 fix");
        assert_eq!(strip("fix ACME_SHOP_D-1147"), "fix ACME_SHOP_D-1147");
        assert_eq!(strip("ACME_SHOP_D-1147"), "ACME_SHOP_D-1147");
        assert_eq!(strip("テスト"), "テスト");
    }

    fn pr(number: i64, title: &str, head: &str) -> WorkItem {
        WorkItem {
            kind: Kind::Pr,
            number,
            title: title.into(),
            head_ref: head.into(),
            repo: "Acme/app".into(),
            ..Default::default()
        }
    }

    fn ticket(key: &str) -> WorkItem {
        WorkItem {
            provider: Provider::Backlog,
            identifier: key.into(),
            repo: key.rsplit_once('-').unwrap().0.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_index_links_tickets_and_pull_requests_both_ways() {
        let items = vec![
            pr(182, "ACME_SHOP_D-1147 docs: sync", "main"),
            ticket("ACME_SHOP_D-1147"),
            pr(190, "Follow-up", "feature/acme_shop_d-1147-more"),
            pr(483, "ACME_SHOP_D-1163 fix(deps): upgrade", "renovate/x"),
            pr(7, "Unrelated", "fix/thing"),
            WorkItem {
                kind: Kind::Issue,
                title: "ACME_SHOP_D-1147 an issue, not a pull request".into(),
                repo: "Acme/app".into(),
                ..Default::default()
            },
        ];
        let links = TicketLinks::build(&items, &[]);
        assert_eq!(
            links.prs("backlog:acme_shop_d-1147"),
            ["acme/app:pr:182", "acme/app:pr:190"]
        );
        assert_eq!(links.tickets("acme/app:pr:182"), ["ACME_SHOP_D-1147"]);
        // D-1163 is not listed, but its project is: the PR still names it.
        assert_eq!(links.tickets("acme/app:pr:483"), ["ACME_SHOP_D-1163"]);
        assert!(links.tickets("acme/app:pr:7").is_empty());
        // A project the space knows counts though none of its issues is listed.
        let known = TicketLinks::build(&items[..1], &["ACME_SHOP_D".into()]);
        assert_eq!(known.tickets("acme/app:pr:182"), ["ACME_SHOP_D-1147"]);
        assert_eq!(TicketLinks::build(&items[..1], &[]), TicketLinks::default());
    }
}
