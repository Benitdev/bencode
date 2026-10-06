//! MonoCode `useProjectDiffStats`: the rail's +N/−N for every project, not
//! just the open one. Each project is read again once its stats are
//! `STATS_TTL` old: on a timer, when the window regains focus, and when a
//! project joins the rail. The reads run one after another on the
//! background executor so eight projects never mean sixteen `git`s at once.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::Context;

use crate::app::{BenCodeApp, normalize_project_path, same_project_path};
use crate::git;

/// MonoCode `RESUME_TTL_MS`.
const STATS_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct ProjectStats {
    /// Keyed by normalised project path; `None` stats outside a repository.
    entries: HashMap<String, Entry>,
    in_flight: bool,
}

struct Entry {
    stats: Option<(usize, usize)>,
    loaded_at: Instant,
}

impl ProjectStats {
    pub fn get(&self, project: &str) -> Option<(usize, usize)> {
        self.entries
            .iter()
            .find(|(path, _)| same_project_path(path, project))
            .and_then(|(_, entry)| entry.stats)
    }

    /// Projects never read or read `STATS_TTL` ago.
    fn stale(&self, projects: &[String], now: Instant) -> Vec<String> {
        projects
            .iter()
            .map(|path| normalize_project_path(path))
            .filter(|path| {
                self.entries
                    .get(path)
                    .is_none_or(|entry| now.duration_since(entry.loaded_at) >= STATS_TTL)
            })
            .collect()
    }
}

impl BenCodeApp {
    /// Diff stats for a rail project as of its last read.
    pub fn project_diff_stats(&self, project: &str) -> Option<(usize, usize)> {
        self.project_stats.get(project)
    }

    pub(crate) fn start_project_stats_poll(&mut self, cx: &mut Context<Self>) {
        self.refresh_project_stats(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STATS_TTL).await;
                if this
                    .update(cx, |app, cx| app.refresh_project_stats(cx))
                    .is_err()
                {
                    return; // app dropped
                }
            }
        })
        .detach();
    }

    /// Reads the stale projects' stats; a no-op while a read is running.
    pub(crate) fn refresh_project_stats(&mut self, cx: &mut Context<Self>) {
        if self.project_stats.in_flight {
            return;
        }
        let stale = self
            .project_stats
            .stale(&self.recent_projects, Instant::now());
        if stale.is_empty() {
            return;
        }
        self.project_stats.in_flight = true;
        let task = cx.background_executor().spawn(async move {
            stale
                .into_iter()
                .map(|path| {
                    let stats = git::diff_stats(&path);
                    (path, stats)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let read = task.await;
            let applied = this.update(cx, |app, cx| {
                let now = Instant::now();
                let stats = &mut app.project_stats;
                stats.in_flight = false;
                let changed = read.into_iter().fold(false, |changed, (path, value)| {
                    let before = stats.entries.insert(
                        path,
                        Entry {
                            stats: value,
                            loaded_at: now,
                        },
                    );
                    changed || before.is_none_or(|entry| entry.stats != value)
                });
                if changed {
                    cx.notify();
                }
            });
            if let Err(err) = applied {
                log::debug!("project stats read after app drop: {err:#}");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unread_or_expired_projects_are_stale() {
        let now = Instant::now();
        let mut stats = ProjectStats::default();
        stats.entries.insert(
            "/fresh".into(),
            Entry {
                stats: Some((1, 2)),
                loaded_at: now,
            },
        );
        stats.entries.insert(
            "/old".into(),
            Entry {
                stats: None,
                loaded_at: now - STATS_TTL,
            },
        );

        let stale = stats.stale(&["/fresh/".into(), "/old".into(), "/new".into()], now);

        assert_eq!(stale, ["/old", "/new"]);
        assert_eq!(stats.get("/fresh/"), Some((1, 2)));
        assert_eq!(stats.get("/old"), None);
    }
}
