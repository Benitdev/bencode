//! Turns on machines outlive BenCode (MonoCode: "closing the desktop does
//! not stop a host session"). Each one running is recorded in
//! `remote-runs.json` in the data folder: quitting leaves it running on the
//! machine (`interrupt_runs_for_quit` skips it), and the next launch
//! follows it again. The thread drops what the turn had written, back to
//! where it began, and the host replays the turn's events from the first,
//! so the transcript comes back whole whatever was saved before the quit.

use std::path::PathBuf;

use gpui::Context;
use serde::{Deserialize, Serialize};

use crate::app::BenCodeApp;
use crate::app::agent::RunPurpose;
use crate::db::SessionRow;
use crate::harness::HarnessKind;

const FILE: &str = "remote-runs.json";

/// The machine and the run a turn is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRunRef {
    pub machine: String,
    pub run: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRunRecord {
    pub session_id: String,
    #[serde(flatten)]
    pub run: RemoteRunRef,
    pub harness: String,
    /// The thread's blocks when the turn began (its user block included):
    /// what the replay starts from.
    pub blocks_before: usize,
    #[serde(default)]
    pub compact: bool,
    #[serde(default)]
    pub auto_approve: bool,
}

fn path() -> Option<PathBuf> {
    crate::storage::data_dir().map(|dir| dir.join(FILE))
}

fn read_records() -> Vec<RemoteRunRecord> {
    let Some(path) = path() else {
        return Vec::new();
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            log::error!("{} is unreadable: {err}", path.display());
            Vec::new()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => {
            log::error!("cannot read {}: {err}", path.display());
            Vec::new()
        }
    }
}

/// The newest list written so far: a save that comes after a newer one
/// (threads finish in any order) writes nothing.
static WRITTEN: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
static NEXT_SAVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn write_records(seq: u64, records: &[RemoteRunRecord]) {
    let Some(path) = path() else {
        return;
    };
    let mut written = WRITTEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *written > seq {
        return;
    }
    *written = seq;
    let written = serde_json::to_vec_pretty(records)
        .map_err(std::io::Error::other)
        .and_then(|bytes| {
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &path)
        });
    if let Err(err) = written {
        log::error!("cannot write {}: {err}", path.display());
    }
}

/// The thread as it was when its turn began: what it wrote since goes, the
/// replay writes it again.
fn rewind(session: &mut SessionRow, blocks_before: usize) {
    if session.blocks.len() > blocks_before {
        session.blocks.truncate(blocks_before);
    }
}

impl BenCodeApp {
    pub(super) fn remember_remote_run(
        &mut self,
        session_id: &str,
        run: RemoteRunRef,
        blocks_before: usize,
        purpose: RunPurpose,
        auto_approve: bool,
    ) {
        let harness = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.harness.clone())
            .unwrap_or_default();
        self.remote_runs.retain(|r| r.session_id != session_id);
        self.remote_runs.push(RemoteRunRecord {
            session_id: session_id.to_string(),
            run,
            harness,
            blocks_before,
            compact: purpose == RunPurpose::Compact,
            auto_approve,
        });
        self.save_remote_runs();
    }

    pub(super) fn forget_remote_run(&mut self, session_id: &str) {
        let before = self.remote_runs.len();
        self.remote_runs.retain(|r| r.session_id != session_id);
        if self.remote_runs.len() != before {
            self.save_remote_runs();
        }
    }

    fn save_remote_runs(&self) {
        if !self.owns_data_folder() {
            return;
        }
        let records = self.remote_runs.clone();
        let seq = NEXT_SAVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || write_records(seq, &records));
    }

    /// At launch: every turn a machine was running when BenCode quit is
    /// followed again. Only the BenCode that owns the data folder does it.
    pub(crate) fn follow_remote_runs(&mut self, cx: &mut Context<Self>) {
        if !self.owns_data_folder() {
            return;
        }
        let task = cx.background_executor().spawn(async { read_records() });
        cx.spawn(async move |this, cx| {
            let records = task.await;
            if records.is_empty() {
                return;
            }
            let ids: Vec<String> = records.iter().map(|r| r.session_id.clone()).collect();
            let landed = this.update(cx, |this, cx| {
                this.remote_runs = records;
                this.db_then(
                    cx,
                    move |db| {
                        let mut rows = Vec::new();
                        for id in ids {
                            match db.get_session(&id) {
                                Ok(Some(row)) => rows.push(row),
                                Ok(None) => log::info!("remote run's thread {id} is gone"),
                                Err(err) => log::error!("could not read thread {id}: {err:#}"),
                            }
                        }
                        Ok(rows)
                    },
                    |this, rows: anyhow::Result<Vec<SessionRow>>, cx| match rows {
                        Ok(rows) => this.attach_remote_runs(rows, cx),
                        Err(err) => log::error!("could not read the remote runs' threads: {err:#}"),
                    },
                );
            });
            if let Err(err) = landed {
                log::debug!("remote runs after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn attach_remote_runs(&mut self, rows: Vec<SessionRow>, cx: &mut Context<Self>) {
        for row in rows {
            if !self.sessions.iter().any(|s| s.id == row.id) {
                self.sessions.push(row);
            }
        }
        let records = self.remote_runs.clone();
        for record in records {
            let id = record.session_id.clone();
            if self.is_agent_running_in(&id) || !self.sessions.iter().any(|s| s.id == id) {
                self.forget_remote_run(&id);
                continue;
            }
            let harness = HarnessKind::from_id(&record.harness).unwrap_or(HarnessKind::Claude);
            let attached =
                crate::remote::spawn::attach(&record.run.machine, &record.run.run, harness);
            let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
                continue;
            };
            match attached {
                Ok(turn) => {
                    rewind(session, record.blocks_before);
                    log::info!(
                        "following turn {} on {} again",
                        record.run.run,
                        record.run.machine
                    );
                    let steers = turn.handle.can_steer();
                    self.track_run(
                        id.clone(),
                        turn.handle,
                        turn.events,
                        record.auto_approve,
                        cx,
                    );
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.purpose = if record.compact {
                            RunPurpose::Compact
                        } else {
                            RunPurpose::Turn
                        };
                        run.can_steer = steers && !record.compact;
                        run.remote = Some(record.run.clone());
                    }
                }
                Err(err) => {
                    log::warn!("could not follow turn {}: {err:#}", record.run.run);
                    crate::app::agent::mark_turn_interrupted(session, crate::app::agent::now_ms());
                    self.persist_session(&id);
                    self.forget_remote_run(&id);
                }
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Block;

    #[test]
    fn records_round_trip_in_camel_case() {
        let record = RemoteRunRecord {
            session_id: "s".into(),
            run: RemoteRunRef {
                machine: "m-1".into(),
                run: "0011aabb".into(),
            },
            harness: "claude".into(),
            blocks_before: 3,
            compact: false,
            auto_approve: true,
        };
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains(r#""sessionId":"s""#), "{text}");
        assert!(text.contains(r#""machine":"m-1""#), "{text}");
        assert_eq!(
            serde_json::from_str::<RemoteRunRecord>(&text).unwrap(),
            record
        );
    }

    #[test]
    fn a_rewind_keeps_the_turn_s_start() {
        let mut session = SessionRow {
            blocks: vec![
                Block::new("u", "user", "go"),
                Block::new("a", "assistant", "half"),
            ],
            ..Default::default()
        };
        rewind(&mut session, 1);
        assert_eq!(session.blocks.len(), 1);
        rewind(&mut session, 5);
        assert_eq!(session.blocks.len(), 1);
    }
}
