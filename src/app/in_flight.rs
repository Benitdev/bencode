//! MonoCode `inFlight.ts` with `appLifecycle.ts` `loadResumedWorkspaceOnce`
//! and `App.tsx`'s auto-continue: the threads whose turn is running are
//! kept in `in_flight_sessions` while it runs, so a quit, an update's
//! restart or a crash that cuts it off is found on the next launch. Those
//! turns are marked interrupted and offered for resuming: the harness
//! session is resumed with a continue prompt. MonoCode continues them at
//! once; BenCode asks first unless Settings › General says not to, as
//! Orca's native chats do.

use std::collections::HashSet;

use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::agent::{TurnInput, mark_turn_interrupted, now_ms};
use crate::db::{InFlightSession, SessionRow};
use crate::harness::HarnessKind;

/// MonoCode `CONTINUE_PROMPT`: what the thread shows for the resumed turn.
pub const CONTINUE_PROMPT: &str = "Continue from where you left off.";

/// What the agent reads instead (Orca's restart continuation): the turn
/// was cut off, so its last action may have run already.
const CONTINUE_AGENT_PROMPT: &str = "BenCode restarted, so your previous reply was cut off partway \
     through. Before continuing, check whether your most recent action completed — don't repeat it \
     if it did. Then carry on.";

/// A thread offered for resuming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeOffer {
    pub session_id: String,
    pub title: String,
    pub harness: String,
}

/// The interrupted turns found at launch, and which the dialog has picked.
#[derive(Default)]
pub struct ResumeState {
    pub offers: Vec<ResumeOffer>,
    pub picked: HashSet<String>,
    /// The in-flight list last written, so an unchanged one is not.
    written: Vec<InFlightSession>,
    /// Held for the app's life: only the BenCode holding it reads or
    /// writes the list. A second one on the same data (an installed app
    /// beside `cargo run`) would take the first's live turns for
    /// interrupted ones and run them twice.
    lock: Option<std::fs::File>,
}

/// Locks `in-flight.lock` in the data folder; `None` while another
/// BenCode holds it.
fn claim_lock() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd as _;
    let path = crate::storage::data_dir()?.join("in-flight.lock");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(err) => {
            log::warn!("could not open {}: {err}", path.display());
            return None;
        }
    };
    // SAFETY: `flock` on a descriptor this function owns; the lock lasts as
    // long as the file stays open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        log::info!("another BenCode holds the in-flight list; turns are not resumed here");
        return None;
    }
    Some(file)
}

impl ResumeState {
    pub fn is_open(&self) -> bool {
        !self.offers.is_empty()
    }
}

/// MonoCode `canAutoContinue`: a harness session to resume, and the
/// interrupt notice still last (a later turn ends the offer).
fn can_continue(session: &SessionRow) -> bool {
    !session.worktree_removed
        && session
            .provider_session_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
        && HarnessKind::from_id(&session.harness).is_some()
        && session.blocks.last().is_some_and(|b| {
            b.role == "system"
                && b.extra.get("notice").and_then(|n| n.as_str()) == Some("interrupt")
        })
}

/// MonoCode `canResumeAfterQuit`: a thread with a turn worth coming back to.
fn worth_keeping(session: &SessionRow) -> bool {
    !session.worktree_removed
        && session.cwd != "~"
        && session.blocks.iter().any(|b| b.role == "user")
}

impl BenCodeApp {
    /// Whether this is the one BenCode on its data folder (it holds the
    /// lock), not a second one beside it.
    pub(crate) fn owns_data_folder(&self) -> bool {
        self.resume.lock.is_some()
    }

    /// Writes the running user turns to `in_flight_sessions` when they
    /// changed. Called as runs start and end; a quit leaves the list as it
    /// was, which is what the next launch reads.
    pub(crate) fn sync_in_flight(&mut self) {
        if self.resume.lock.is_none() {
            return;
        }
        let refs: Vec<InFlightSession> = self
            .sessions
            .iter()
            .filter(|s| {
                self.runs
                    .get(&s.id)
                    .is_some_and(|run| run.purpose == super::agent::RunPurpose::Turn)
            })
            .filter(|s| worth_keeping(s))
            .map(|s| InFlightSession {
                session_id: s.id.clone(),
                cwd: s.cwd.clone(),
            })
            .collect();
        if refs == self.resume.written {
            return;
        }
        self.resume.written = refs.clone();
        self.db_write("in-flight list", move |db| db.replace_in_flight(&refs));
    }

    /// At launch: reads and clears the list the last run left, marks those
    /// turns interrupted (a crash never did), and offers them. Queued on
    /// the writer before any run can start, so no new list is read.
    pub(crate) fn load_interrupted_turns(&mut self, cx: &mut Context<Self>) {
        // Without the lock (another BenCode, or no data folder) the list
        // is left alone.
        self.resume.lock = claim_lock();
        if self.resume.lock.is_none() {
            return;
        }
        self.db_then(
            cx,
            |db| {
                let refs = db.list_in_flight()?;
                db.replace_in_flight(&[])?;
                let mut rows = Vec::new();
                for entry in refs {
                    match db.get_session(&entry.session_id) {
                        Ok(Some(row)) => rows.push(row),
                        Ok(None) => log::info!("interrupted thread {} is gone", entry.session_id),
                        Err(err) => log::error!(
                            "could not read interrupted thread {}: {err:#}",
                            entry.session_id
                        ),
                    }
                }
                Ok(rows)
            },
            |this, rows: anyhow::Result<Vec<SessionRow>>, cx| match rows {
                Ok(rows) => this.offer_interrupted(rows, cx),
                Err(err) => log::error!("could not read the interrupted threads: {err:#}"),
            },
        );
    }

    fn offer_interrupted(&mut self, rows: Vec<SessionRow>, cx: &mut Context<Self>) {
        let mut offers = Vec::new();
        for row in rows {
            let id = row.id.clone();
            // A thread already in memory is newer than the read.
            if !self.sessions.iter().any(|s| s.id == id) {
                self.sessions.push(row);
            }
            if self.is_agent_running_in(&id) {
                continue;
            }
            let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
                continue;
            };
            if !worth_keeping(session) {
                continue;
            }
            mark_turn_interrupted(session, now_ms());
            let offer = can_continue(session).then(|| ResumeOffer {
                session_id: id.clone(),
                title: session.title.clone(),
                harness: session.harness.clone(),
            });
            self.persist_session(&id);
            offers.extend(offer);
        }
        if offers.is_empty() {
            return;
        }
        if self.resume_interrupted_auto {
            for offer in offers {
                self.continue_interrupted(&offer.session_id, cx);
            }
        } else {
            self.resume.picked = offers.iter().map(|o| o.session_id.clone()).collect();
            self.resume.offers = offers;
        }
        cx.notify();
    }

    /// Resumes one interrupted turn, unless something has happened in the
    /// thread since.
    fn continue_interrupted(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let ready = !self.is_agent_running_in(session_id)
            && self
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .is_some_and(can_continue);
        if !ready {
            log::info!("interrupted thread {session_id} moved on; not resumed");
            return;
        }
        let input = TurnInput {
            text: CONTINUE_PROMPT.to_string(),
            agent_prompt: Some(CONTINUE_AGENT_PROMPT.to_string()),
            ..Default::default()
        };
        self.send_turn(session_id, input, cx);
    }

    pub fn toggle_resume_pick(&mut self, session_id: &str, on: bool, cx: &mut Context<Self>) {
        if on {
            self.resume.picked.insert(session_id.to_string());
        } else {
            self.resume.picked.remove(session_id);
        }
        cx.notify();
    }

    /// The dialog's Resume: the picked threads, in the dialog's order.
    pub fn resume_picked(&mut self, cx: &mut Context<Self>) {
        let offers = std::mem::take(&mut self.resume.offers);
        let picked = std::mem::take(&mut self.resume.picked);
        for offer in offers.iter().filter(|o| picked.contains(&o.session_id)) {
            self.continue_interrupted(&offer.session_id, cx);
        }
        cx.notify();
    }

    /// The dialog's Not now: the threads keep their interrupt notice, and
    /// the user's next message continues them.
    pub fn dismiss_resume(&mut self, cx: &mut Context<Self>) {
        self.resume.offers.clear();
        self.resume.picked.clear();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Block;

    fn session(blocks: Vec<Block>) -> SessionRow {
        let mut s = SessionRow::default();
        s.id = "t".into();
        s.cwd = "/p".into();
        s.harness = "claude".into();
        s.provider_session_id = Some("abc".into());
        s.blocks = blocks;
        s
    }

    fn interrupted(s: &mut SessionRow) {
        mark_turn_interrupted(s, 2);
    }

    #[test]
    fn an_interrupted_turn_with_a_harness_session_can_continue() {
        let mut s = session(vec![Block::new("u", "user", "go")]);
        assert!(!can_continue(&s));
        interrupted(&mut s);
        assert!(can_continue(&s));
    }

    #[test]
    fn no_harness_session_or_a_later_turn_ends_the_offer() {
        let mut s = session(vec![Block::new("u", "user", "go")]);
        interrupted(&mut s);
        s.provider_session_id = None;
        assert!(!can_continue(&s));
        s.provider_session_id = Some("abc".into());
        s.blocks.push(Block::new("u2", "user", "next"));
        assert!(!can_continue(&s));
    }

    #[test]
    fn a_removed_worktree_or_a_blank_thread_is_not_kept() {
        let mut s = session(vec![Block::new("u", "user", "go")]);
        assert!(worth_keeping(&s));
        s.worktree_removed = true;
        assert!(!worth_keeping(&s));
        assert!(!worth_keeping(&session(Vec::new())));
    }
}
