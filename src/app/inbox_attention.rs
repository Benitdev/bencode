//! Asks the trackers what they notified the user of, for the Inbox's
//! "Needs you" (`work_items/attention.rs`): Backlog's notifications, and
//! the GitHub threads each signed-in account participates in. It follows
//! the list's poll but asks each tracker at most once a minute (GitHub's
//! `X-Poll-Interval` when longer), off the UI thread; a failure is logged
//! and leaves what the tracker said before. Nothing is marked read.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use gpui::Context;

use super::BenCodeApp;
use crate::work_items::Provider;
use crate::work_items::attention::{self, Found};

/// GitHub's notifications for the projects listed, one ask per account
/// (accounts differ per project): the threads about `repos`, and the
/// user's login in each project. `None` when every ask failed.
fn github_attention(projects: &[String], repos: &HashSet<String>) -> Option<Found> {
    let mut found = Found::default();
    let mut asked: Vec<String> = Vec::new();
    let mut answered = false;
    for project in projects {
        let cwd = Path::new(project);
        let login = match crate::github::viewer_login(cwd) {
            Ok(login) => login,
            Err(err) => {
                log::warn!("inbox: no GitHub login for {project}: {err}");
                found.partial = true;
                continue;
            }
        };
        found.me.insert(project.clone(), login.clone());
        if asked.contains(&login) {
            continue;
        }
        asked.push(login);
        let notified = crate::github::notifications(cwd).and_then(|(headers, body)| {
            found.poll = found.poll.max(attention::poll_interval(&headers));
            attention::parse_github(&body, repos)
        });
        match notified {
            Ok(notified) => {
                found.notified.extend(notified);
                answered = true;
            }
            Err(err) => {
                log::warn!("inbox: could not read GitHub notifications for {project}: {err}");
                found.partial = true;
            }
        }
    }
    answered.then_some(found)
}

impl BenCodeApp {
    /// Asks the trackers that are due; `force` (an explicit refresh) asks
    /// Backlog now. GitHub's pace is its own to set.
    pub(crate) fn fetch_inbox_attention(&mut self, force: bool, cx: &mut Context<Self>) {
        let now = Instant::now();
        let inbox = self.inbox(cx);
        if inbox.attention.fetching {
            return;
        }
        let backlog_due =
            inbox.backlog_connected && (force || inbox.attention.due(Provider::Backlog, now));
        let mut repos: HashSet<String> = HashSet::new();
        let mut projects: Vec<String> = Vec::new();
        for item in inbox
            .items
            .iter()
            .filter(|i| i.provider == Provider::GitHub)
        {
            repos.insert(item.repo.to_lowercase());
            if !item.project.is_empty() && !projects.contains(&item.project) {
                projects.push(item.project.clone());
            }
        }
        let github_due = !projects.is_empty() && inbox.attention.due(Provider::GitHub, now);
        if !backlog_due && !github_due {
            return;
        }
        self.inbox_mut(cx, |inbox| inbox.attention.fetching = true);
        let task = cx.background_executor().spawn(async move {
            let backlog = backlog_due.then(|| {
                crate::backlog::notifications()?
                    .inspect_err(|err| {
                        log::warn!("inbox: could not read Backlog notifications: {err}")
                    })
                    .ok()
            });
            let github = github_due.then(|| github_attention(&projects, &repos));
            (backlog, github)
        });
        cx.spawn(async move |this, cx| {
            let (backlog, github) = task.await;
            let landed = this.update(cx, |app, cx| {
                app.inbox_state.update(cx, |inbox, cx| {
                    let now = Instant::now();
                    inbox.attention.fetching = false;
                    let mut changed = false;
                    if let Some(found) = backlog {
                        changed |= inbox.attention.land(Provider::Backlog, found, now);
                    }
                    if let Some(found) = github {
                        changed |= inbox.attention.land(Provider::GitHub, found, now);
                    }
                    // What shows the Inbox, and the rail's mark, follow it.
                    if changed {
                        cx.notify();
                    }
                });
            });
            if let Err(err) = landed {
                log::debug!("inbox notifications after app drop: {err:#}");
            }
        })
        .detach();
    }
}
