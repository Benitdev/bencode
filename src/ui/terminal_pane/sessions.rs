//! The dock's terminals as sessions of the terminal host (`pty_host`):
//! each tab names one, the tabs are saved, and a launch after an update's
//! restart or a crash shows them again with their shells still running.
//! ⌘Q ends them all, so the next launch starts afresh.

use std::collections::BTreeMap;
use std::time::Duration;

use gpui::Context;
use serde::{Deserialize, Serialize};

use crate::app::BenCodeApp;
use crate::pty_host::{self, SessionInfo};

/// A saved dock tab: its host session and the folder it started in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTerminal {
    pub session: String,
    pub cwd: String,
}

/// How long a new session has to show up in the host's list.
const PID_WAIT: Duration = Duration::from_secs(5);

/// The saved tabs whose sessions the host still has, by project, and the
/// host's sessions no tab names (left by a ⌘Q that could not reach it).
pub fn plan_restore(
    saved: &BTreeMap<String, Vec<SavedTerminal>>,
    live: &[SessionInfo],
) -> (Vec<(String, SavedTerminal, u32, bool)>, Vec<String>) {
    let mut keep = Vec::new();
    for (project, tabs) in saved {
        for tab in tabs {
            if let Some(info) = live.iter().find(|s| s.id == tab.session) {
                keep.push((project.clone(), tab.clone(), info.pid, info.alive));
            }
        }
    }
    let orphans = live
        .iter()
        .filter(|s| !keep.iter().any(|(_, tab, ..)| tab.session == s.id))
        .map(|s| s.id.clone())
        .collect();
    (keep, orphans)
}

impl BenCodeApp {
    /// The dock's tabs as `settings.json` keeps them.
    pub(crate) fn saved_terminals(&self) -> BTreeMap<String, Vec<SavedTerminal>> {
        // Not shown again yet: a save meanwhile keeps them.
        if self.terminals.restoring {
            return self.terminals.saved.clone();
        }
        self.terminals
            .docks
            .iter()
            .filter(|(_, dock)| !dock.tabs.is_empty())
            .map(|(project, dock)| {
                let tabs = dock
                    .tabs
                    .iter()
                    .map(|tab| SavedTerminal {
                        session: tab.session.clone(),
                        cwd: tab.cwd.clone(),
                    })
                    .collect();
                (project.clone(), tabs)
            })
            .collect()
    }

    /// At launch: shows again the saved tabs whose sessions the host still
    /// runs, ends the sessions no tab names, then starts the current
    /// project's shell if its dock is open and it has none.
    pub(crate) fn restore_terminals(&mut self, cx: &mut Context<Self>) {
        // Only the BenCode that owns this data folder shows the saved tabs
        // again and ends the sessions no tab names. Another one beside it
        // (`cargo run`) has its own: the saved tabs are the first one's,
        // which is showing them, and the host gives a session to one client.
        let owner = self.owns_data_folder();
        if !owner {
            self.terminals.saved.clear();
        }
        let saved = self.terminals.saved.clone();
        self.terminals.restoring = true;
        let task = cx
            .background_executor()
            .spawn(async move { pty_host::list_sessions() });
        cx.spawn(async move |this, cx| {
            let live = task.await;
            let landed = this.update(cx, |this, cx| {
                match live {
                    Ok(live) => {
                        let (keep, orphans) = plan_restore(&saved, &live);
                        for (project, tab, pid, alive) in keep {
                            this.add_terminal_tab(
                                &project,
                                tab.session,
                                &tab.cwd,
                                alive.then_some(pid),
                                cx,
                            );
                        }
                        if owner && !orphans.is_empty() {
                            cx.background_executor()
                                .spawn(async move { pty_host::kill_sessions(orphans) })
                                .detach();
                        }
                    }
                    // The host did not answer in time: every saved tab comes
                    // back, so a shell still running is not left without one
                    // (a tab whose session is gone starts a new shell).
                    Err(err) => {
                        log::warn!("could not list the terminal sessions: {err:#}");
                        for (project, tabs) in &saved {
                            for tab in tabs {
                                let id = this.add_terminal_tab(
                                    project,
                                    tab.session.clone(),
                                    &tab.cwd,
                                    None,
                                    cx,
                                );
                                this.find_terminal_process(id, tab.session.clone(), cx);
                            }
                        }
                    }
                }
                this.terminals.saved.clear();
                this.terminals.restoring = false;
                if this.is_terminal_open() {
                    this.ensure_project_terminal(cx);
                }
                this.save_settings(cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("terminal restore after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Finds the process a new session started, for its tab's job and
    /// folder; the host lists it once the attach client has reached it.
    pub(super) fn find_terminal_process(
        &mut self,
        id: u64,
        session: String,
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            while started.elapsed() < PID_WAIT {
                match pty_host::list_sessions() {
                    Ok(list) => {
                        if let Some(info) = list.into_iter().find(|s| s.id == session) {
                            return Some(info.pid);
                        }
                    }
                    Err(err) => log::debug!("terminal session list: {err:#}"),
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            None
        });
        cx.spawn(async move |this, cx| {
            let Some(pid) = task.await else {
                log::debug!("terminal {id}: its session's process was not found");
                return;
            };
            let landed = this.update(cx, |this, cx| {
                if let Some(tab) = this
                    .terminals
                    .docks
                    .values_mut()
                    .flat_map(|d| &mut d.tabs)
                    .find(|t| t.id == id)
                {
                    tab.process = Some(pid);
                    this.start_terminal_poll(cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("terminal process after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Ends the host sessions of closed tabs, off the UI thread.
    pub(super) fn end_terminal_sessions(&self, sessions: Vec<String>, cx: &mut Context<Self>) {
        if sessions.is_empty() {
            return;
        }
        cx.background_executor()
            .spawn(async move { pty_host::kill_sessions(sessions) })
            .detach();
    }

    /// ⌘Q: this BenCode's sessions end (blocking, on the way out).
    pub(crate) fn end_all_terminal_sessions(&self) {
        let sessions = self
            .terminals
            .docks
            .values()
            .flat_map(|dock| &dock.tabs)
            .map(|tab| tab.session.clone())
            .chain(
                self.terminals
                    .saved
                    .values()
                    .flatten()
                    .map(|tab| tab.session.clone()),
            )
            .collect();
        pty_host::kill_sessions(sessions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(session: &str) -> SavedTerminal {
        SavedTerminal {
            session: session.into(),
            cwd: "/p".into(),
        }
    }

    fn live(id: &str, alive: bool) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            pid: 7,
            alive,
        }
    }

    #[test]
    fn saved_tabs_with_a_session_come_back_and_unnamed_sessions_end() {
        let saved = BTreeMap::from([("/p".to_string(), vec![tab("a"), tab("gone")])]);
        let (keep, orphans) = plan_restore(&saved, &[live("a", true), live("stray", true)]);
        assert_eq!(keep, vec![("/p".to_string(), tab("a"), 7, true)]);
        assert_eq!(orphans, vec!["stray".to_string()]);
    }

    #[test]
    fn an_ended_session_still_comes_back_to_show_its_output() {
        let saved = BTreeMap::from([("/p".to_string(), vec![tab("a")])]);
        let (keep, _) = plan_restore(&saved, &[live("a", false)]);
        assert_eq!(keep[0].3, false);
    }
}
