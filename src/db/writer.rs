//! One thread owns a second connection to BenCode's database and runs writes
//! in the order they were queued, so saving a thread never blocks the UI. A
//! save that a later save of the same thread replaces before it ran is
//! skipped. (No MonoCode counterpart: its writes go through Tauri IPC.)

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::oneshot;

use super::{AppDb, SessionRow};

type Job = Box<dyn FnOnce(&AppDb) + Send>;

enum Msg {
    /// A full snapshot of one thread; a later snapshot of the same thread
    /// replaces it if neither has run yet.
    Save(Box<SessionRow>),
    /// Any other work, in order.
    Job(Job),
    /// Answers once everything queued before it ran.
    Flush(mpsc::Sender<()>),
}

pub struct DbWriter {
    queue: mpsc::Sender<Msg>,
}

impl DbWriter {
    /// Opens a second connection to `path` and starts the writer thread.
    pub fn open(path: &Path) -> Result<Self> {
        let db = AppDb::open_at(path)?;
        let (queue, jobs) = mpsc::channel::<Msg>();
        std::thread::Builder::new()
            .name("bencode-db-writer".into())
            .spawn(move || {
                while let Ok(first) = jobs.recv() {
                    let mut batch = vec![first];
                    while let Ok(next) = jobs.try_recv() {
                        batch.push(next);
                    }
                    let skip = superseded(&batch);
                    for (msg, skip) in batch.into_iter().zip(skip) {
                        match msg {
                            Msg::Save(_) if skip => {}
                            Msg::Save(session) => {
                                if let Err(err) = db.upsert_session(&session) {
                                    log::error!("failed to save session {}: {err:#}", session.id);
                                }
                            }
                            Msg::Job(job) => job(&db),
                            Msg::Flush(done) => {
                                // Nobody waits any more when the flush timed out.
                                if done.send(()).is_err() {
                                    log::trace!("flush reply dropped");
                                }
                            }
                        }
                    }
                }
            })?;
        Ok(Self { queue })
    }

    fn send(&self, msg: Msg) {
        if self.queue.send(msg).is_err() {
            log::error!("the database writer is not running");
        }
    }

    /// Queues a save of `session` (a snapshot taken now).
    pub fn save(&self, session: SessionRow) {
        self.send(Msg::Save(Box::new(session)));
    }

    /// Queues `job`; its result arrives on the returned channel.
    pub fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&AppDb) -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (done, result) = oneshot::channel();
        self.send(Msg::Job(Box::new(move |db| {
            // The receiver is gone when nobody waits for the result.
            if done.send(job(db)).is_err() {
                log::trace!("database result dropped");
            }
        })));
        result
    }

    /// Queues `job` for its effect; a failure is only logged.
    pub fn run_logged(
        &self,
        what: &'static str,
        job: impl FnOnce(&AppDb) -> Result<()> + Send + 'static,
    ) {
        drop(self.run(move |db| {
            if let Err(err) = job(db) {
                log::error!("database {what} failed: {err:#}");
            }
        }));
    }

    /// Blocks until everything queued so far ran, or `timeout` passed.
    /// Returns whether the queue drained.
    pub fn flush(&self, timeout: Duration) -> bool {
        let (done, drained) = mpsc::channel();
        self.send(Msg::Flush(done));
        drained.recv_timeout(timeout).is_ok()
    }
}

impl Drop for DbWriter {
    /// Dropping the app (its window closed) must not lose queued saves.
    fn drop(&mut self) {
        if !self.flush(Duration::from_secs(5)) {
            log::error!("the database writer was dropped before every write ran");
        }
    }
}

/// Which saves to skip: a save is skipped when a later save of the same
/// thread follows it with no job or flush in between (a job may delete or
/// edit that row, so it is a barrier).
fn superseded(batch: &[Msg]) -> Vec<bool> {
    let mut later: HashSet<&str> = HashSet::new();
    let mut skip = vec![false; batch.len()];
    for (ix, msg) in batch.iter().enumerate().rev() {
        match msg {
            Msg::Save(session) => skip[ix] = !later.insert(session.id.as_str()),
            Msg::Job(_) | Msg::Flush(_) => later.clear(),
        }
    }
    skip
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A database file of its own per test; removed on drop.
    struct TempDb {
        dir: PathBuf,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("bencode-writer-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn path(&self) -> PathBuf {
            self.dir.join("bencode.db")
        }

        fn reader(&self) -> AppDb {
            AppDb::open_reader(&self.path()).unwrap()
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.dir) {
                eprintln!("could not remove {}: {err}", self.dir.display());
            }
        }
    }

    fn session(id: &str, title: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: title.into(),
            cwd: "/tmp/project".into(),
            harness: "claude".into(),
            model: "claude:opus".into(),
            created_at: 1,
            updated_at: 2,
            ..SessionRow::default()
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn saves_land_in_order_and_last_wins() {
        let temp = TempDb::new("order");
        let writer = DbWriter::open(&temp.path()).unwrap();
        for title in ["a", "b", "c"] {
            writer.save(session("s1", title));
        }
        assert!(writer.flush(WAIT));
        let saved = temp.reader().get_session("s1").unwrap().unwrap();
        assert_eq!(saved.title, "c");
    }

    #[test]
    fn delete_job_is_a_barrier() {
        let temp = TempDb::new("barrier");
        let writer = DbWriter::open(&temp.path()).unwrap();
        writer.save(session("s1", "a"));
        writer.run_logged("delete", |db| db.delete_session("s1"));
        assert!(writer.flush(WAIT));
        assert!(temp.reader().get_session("s1").unwrap().is_none());
        writer.save(session("s1", "again"));
        assert!(writer.flush(WAIT));
        assert!(temp.reader().get_session("s1").unwrap().is_some());
    }

    #[test]
    fn run_returns_the_job_result() {
        let temp = TempDb::new("run");
        let writer = DbWriter::open(&temp.path()).unwrap();
        writer.save(session("s1", "a"));
        let count = writer.run(|db| db.list_recent_sessions(10).map(|rows| rows.len()));
        assert_eq!(count.blocking_recv().unwrap().unwrap(), 1);
    }

    #[test]
    fn drop_flushes_the_queue() {
        let temp = TempDb::new("drop");
        let writer = DbWriter::open(&temp.path()).unwrap();
        writer.save(session("s1", "a"));
        drop(writer);
        assert!(temp.reader().get_session("s1").unwrap().is_some());
    }

    #[test]
    fn superseded_skips_only_saves_followed_by_a_save_of_the_same_thread() {
        let save = |id: &str| Msg::Save(Box::new(session(id, "t")));
        let batch = [
            save("s1"),
            save("s2"),
            save("s1"),
            Msg::Job(Box::new(|_| {})),
            save("s1"),
            Msg::Flush(mpsc::channel().0),
        ];
        assert_eq!(
            superseded(&batch),
            [true, false, false, false, false, false]
        );
    }

    #[test]
    fn refuses_unparsed_transcripts_without_stopping() {
        let temp = TempDb::new("refuse");
        let writer = DbWriter::open(&temp.path()).unwrap();
        writer.save(SessionRow {
            blocks_parse_failed: true,
            ..session("broken", "a")
        });
        // A flush keeps the two saves in separate batches too.
        assert!(writer.flush(WAIT));
        writer.save(session("fine", "b"));
        assert!(writer.flush(WAIT));
        let reader = temp.reader();
        assert!(reader.get_session("broken").unwrap().is_none());
        assert!(reader.get_session("fine").unwrap().is_some());
    }
}
