//! The agent turns the host runs, and their events (MonoCode `host/engine.ts`
//! and `host/store.ts` keep whole sessions; BenCode's app keeps the
//! transcript, so the host keeps only each run's events, numbered from 1).
//!
//! Every event is appended to `runs/<id>.jsonl` before it goes out, so a
//! connection that drops, or an app that quits, reads the run again from
//! where it was (`attach`). A run the daemon did not see end (it stopped,
//! the machine restarted) is closed with an error when the daemon starts.
//! The app forgets a run once it has folded the whole of it; anything
//! older than `KEEP_ENDED` goes anyway.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use bencode_core::harness::{
    self, AgentEvent, Attachment, DoneStatus, HarnessKind, HarnessProcessHandle, PermissionRequest,
};
use bencode_core::remote::protocol::{RunSummary, ServerFrame, SpawnWire, valid_run_id};
use serde::{Deserialize, Serialize};

use crate::session::Outbox;

/// How long an ended run nobody forgot is kept.
const KEEP_ENDED: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What the host says when it lost a run it was running.
pub const LOST_RUN: &str =
    "The host stopped while this turn was running, so it was cut off. Send a message to continue.";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunMeta {
    cwd: String,
    harness: HarnessKind,
    started_ms: i64,
}

struct RunState {
    events: Vec<AgentEvent>,
    ended: bool,
    handle: Option<HarnessProcessHandle>,
    /// Connections following the run, by connection id.
    followers: Vec<(u64, Arc<Outbox>)>,
    journal: Option<std::fs::File>,
}

pub struct Run {
    id: String,
    meta: RunMeta,
    state: Mutex<RunState>,
}

impl Run {
    fn state(&self) -> MutexGuard<'_, RunState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn summary(&self) -> RunSummary {
        let state = self.state();
        RunSummary {
            run: self.id.clone(),
            cwd: self.meta.cwd.clone(),
            harness: self.meta.harness,
            events: state.events.len() as u64,
            ended: state.ended,
            started_ms: self.meta.started_ms,
        }
    }

    /// Records `event` and sends it to every follower.
    fn push(&self, event: AgentEvent) {
        let mut state = self.state();
        if state.ended {
            return;
        }
        if let Some(journal) = state.journal.as_mut() {
            let line = serde_json::to_string(&event).unwrap_or_default();
            if let Err(err) = writeln!(journal, "{line}") {
                log::error!("run {}: journal write failed: {err}", self.id);
                state.journal = None;
            }
        }
        state.events.push(event.clone());
        let seq = state.events.len() as u64;
        let frame = ServerFrame::Event {
            run: self.id.clone(),
            seq,
            event,
        };
        state.followers.retain(|(_, outbox)| outbox.send(&frame));
    }

    /// No more events: the CLI's stream closed.
    fn end(&self, dir: &Path) {
        let mut state = self.state();
        if state.ended {
            return;
        }
        state.ended = true;
        state.handle = None;
        if let Some(journal) = state.journal.take()
            && let Err(err) = journal.sync_all()
        {
            log::warn!("run {}: journal sync failed: {err}", self.id);
        }
        if let Err(err) = std::fs::write(ended_marker(dir, &self.id), b"") {
            log::error!("run {}: could not mark it ended: {err}", self.id);
        }
        let frame = ServerFrame::RunEnded {
            run: self.id.clone(),
            seq: state.events.len() as u64,
        };
        for (_, outbox) in state.followers.drain(..) {
            outbox.send(&frame);
        }
    }
}

fn journal_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.jsonl"))
}

fn meta_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn ended_marker(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.ended"))
}

pub struct Runs {
    dir: PathBuf,
    attachments: PathBuf,
    runs: Mutex<HashMap<String, Arc<Run>>>,
}

impl Runs {
    /// The runs left on disk: ended ones as they were, the others closed
    /// with `LOST_RUN`, and the stale ones deleted.
    pub fn load(dir: PathBuf, attachments: PathBuf) -> Self {
        let runs = Self {
            dir,
            attachments,
            runs: Mutex::new(HashMap::new()),
        };
        let entries = match std::fs::read_dir(&runs.dir) {
            Ok(entries) => entries,
            Err(err) => {
                log::error!("cannot read {}: {err}", runs.dir.display());
                return runs;
            }
        };
        let now = jiff::Timestamp::now().as_millisecond();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            match runs.restore(id, now) {
                Ok(Some(run)) => {
                    runs.table().insert(id.to_string(), run);
                }
                Ok(None) => runs.delete_files(id),
                Err(err) => {
                    log::warn!("dropping run {id}: {err:#}");
                    runs.delete_files(id);
                }
            }
        }
        runs.sweep_attachments(now);
        runs
    }

    fn restore(&self, id: &str, now: i64) -> Result<Option<Arc<Run>>> {
        let meta: RunMeta = serde_json::from_slice(&std::fs::read(meta_path(&self.dir, id))?)
            .context("unreadable run metadata")?;
        let ended = ended_marker(&self.dir, id).exists();
        if ended && now - meta.started_ms > KEEP_ENDED.as_millis() as i64 {
            return Ok(None);
        }
        let mut events = Vec::new();
        if let Ok(file) = std::fs::File::open(journal_path(&self.dir, id)) {
            for line in BufReader::new(file).lines() {
                let line = line?;
                // A line cut off by a crash is the journal's last.
                match serde_json::from_str::<AgentEvent>(&line) {
                    Ok(event) => events.push(event),
                    Err(_) => break,
                }
            }
        }
        let run = Arc::new(Run {
            id: id.to_string(),
            meta,
            state: Mutex::new(RunState {
                events,
                ended: false,
                handle: None,
                followers: Vec::new(),
                journal: std::fs::OpenOptions::new()
                    .append(true)
                    .open(journal_path(&self.dir, id))
                    .ok(),
            }),
        });
        if ended {
            let mut state = run.state();
            state.ended = true;
            state.journal = None;
        } else {
            log::warn!("run {id} was cut off when the host stopped");
            run.push(AgentEvent::Error(LOST_RUN.into()));
            run.push(AgentEvent::Done(DoneStatus::Failed));
            run.end(&self.dir);
        }
        Ok(Some(run))
    }

    fn table(&self) -> MutexGuard<'_, HashMap<String, Arc<Run>>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self, id: &str) -> Result<Arc<Run>> {
        self.table()
            .get(id)
            .cloned()
            .with_context(|| format!("the host has no run {id}"))
    }

    pub fn running(&self) -> usize {
        self.table()
            .values()
            .filter(|run| !run.state().ended)
            .count()
    }

    pub fn summaries(&self) -> Vec<RunSummary> {
        let mut list: Vec<RunSummary> = self.table().values().map(|run| run.summary()).collect();
        list.sort_by_key(|run| run.started_ms);
        list
    }

    /// Starts a turn and makes `outbox` its first follower.
    pub fn start(
        self: &Arc<Self>,
        id: &str,
        spawn: SpawnWire,
        conn: u64,
        outbox: Arc<Outbox>,
    ) -> Result<()> {
        if !valid_run_id(id) {
            bail!("not a run id: {id}");
        }
        if self.table().contains_key(id) {
            bail!("run {id} exists already");
        }
        let id = id.to_string();
        let meta = RunMeta {
            cwd: spawn.cwd.clone(),
            harness: spawn.harness,
            started_ms: jiff::Timestamp::now().as_millisecond(),
        };
        let mut request = spawn.into_request();
        // Attachments come only from the host's attachment folder.
        request
            .attachments
            .retain(|file| Path::new(&file.path).starts_with(&self.attachments));
        let (handle, mut events) = harness::spawn(&request)?;
        crate::dirs::write_private(&meta_path(&self.dir, &id), &serde_json::to_vec(&meta)?)?;
        let journal = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(journal_path(&self.dir, &id))?;
        let run = Arc::new(Run {
            id: id.clone(),
            meta,
            state: Mutex::new(RunState {
                events: Vec::new(),
                ended: false,
                handle: Some(handle),
                followers: vec![(conn, outbox)],
                journal: Some(journal),
            }),
        });
        self.table().insert(id.clone(), run.clone());
        log::info!("run {id}: {} in {}", request.harness.label(), request.cwd);
        let runs = self.clone();
        std::thread::Builder::new()
            .name(format!("run-{id}"))
            .spawn(move || {
                while let Some(event) = events.blocking_recv() {
                    run.push(event);
                }
                run.end(&runs.dir);
                log::info!("run {}: ended", run.id);
            })?;
        Ok(())
    }

    /// Sends `outbox` the run's events after `after`, then the ones to come.
    /// False when the host has no such run.
    pub fn attach(&self, id: &str, after: u64, conn: u64, outbox: Arc<Outbox>) -> bool {
        let Ok(run) = self.get(id) else {
            return false;
        };
        let mut state = run.state();
        for (ix, event) in state.events.iter().enumerate().skip(after as usize) {
            outbox.send(&ServerFrame::Event {
                run: run.id.clone(),
                seq: ix as u64 + 1,
                event: event.clone(),
            });
        }
        if state.ended {
            outbox.send(&ServerFrame::RunEnded {
                run: run.id.clone(),
                seq: state.events.len() as u64,
            });
        } else {
            state.followers.retain(|(c, _)| *c != conn);
            state.followers.push((conn, outbox));
        }
        true
    }

    /// A connection went away: it follows nothing any more.
    pub fn detach_all(&self, conn: u64) {
        for run in self.table().values() {
            run.state().followers.retain(|(c, _)| *c != conn);
        }
    }

    fn handle(&self, id: &str) -> Result<HarnessProcessHandle> {
        self.get(id)?
            .state()
            .handle
            .clone()
            .with_context(|| format!("run {id} has ended"))
    }

    pub fn stdin(&self, id: &str, line: &str) -> Result<bool> {
        Ok(self.handle(id)?.send_line(line))
    }

    pub fn permission(&self, id: &str, request: &PermissionRequest, allow: bool) -> Result<bool> {
        Ok(self.handle(id)?.respond_permission(request, allow))
    }

    pub fn steer(&self, id: &str, prompt: &str, attachments: &[Attachment]) -> Result<bool> {
        let files: Vec<Attachment> = attachments
            .iter()
            .filter(|file| Path::new(&file.path).starts_with(&self.attachments))
            .cloned()
            .collect();
        Ok(self.handle(id)?.steer(prompt, &files))
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        if let Ok(handle) = self.handle(id) {
            handle.cancel();
        }
        Ok(())
    }

    /// Drops an ended run and its files; a live one is kept until it ends.
    pub fn forget(&self, id: &str) {
        let ended = self.get(id).is_ok_and(|run| run.state().ended);
        if ended {
            self.table().remove(id);
            self.delete_files(id);
        }
    }

    /// Update Host: every run is stopped and closed with `LOST_RUN`.
    pub fn stop_all(&self) {
        let runs: Vec<Arc<Run>> = self.table().values().cloned().collect();
        for run in runs {
            let handle = run.state().handle.clone();
            if let Some(handle) = handle {
                handle.cancel();
                run.push(AgentEvent::Error(LOST_RUN.into()));
                run.push(AgentEvent::Done(DoneStatus::Failed));
                run.end(&self.dir);
            }
        }
    }

    fn delete_files(&self, id: &str) {
        for path in [
            meta_path(&self.dir, id),
            journal_path(&self.dir, id),
            ended_marker(&self.dir, id),
        ] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => log::warn!("could not remove {}: {err}", path.display()),
            }
        }
    }

    /// Attachments outlive their turn for a while (a resumed thread may
    /// read them again), not for ever.
    fn sweep_attachments(&self, now: i64) {
        let Ok(entries) = std::fs::read_dir(&self.attachments) else {
            return;
        };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|age| {
                    now - age.as_millis() as i64 > 4 * KEEP_ENDED.as_millis() as i64
                });
            if old && let Err(err) = std::fs::remove_dir_all(entry.path()) {
                log::warn!("could not remove {}: {err}", entry.path().display());
            }
        }
    }
}
