//! One machine's connection: `ssh <machine> bencode-host connect`, kept open
//! while BenCode runs (MonoCode keeps an SSH forward per machine,
//! `remote_ssh.rs` `Tunnels`). Requests wait for their reply by id; a run's
//! events go to whoever follows it. When the connection drops, the next
//! request opens it again, and a run still followed is picked up where it
//! was (`RunAttach`), so a turn survives a laptop's sleep.
//!
//! Blocking throughout: call it from a background executor or a thread of
//! its own, never the UI thread.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use bencode_core::harness::{AgentEvent, DoneStatus};
use bencode_core::remote::protocol::{
    ClientFrame, HostInfo, MAX_FRAME_BYTES, PROTOCOL_VERSION, Request, RunAttached, ServerFrame,
};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;

use super::Machine;
use super::askpass::Askpass;

/// How long a request waits for its reply unless it says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long SSH and the host's hello may take in the background.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);
/// How long they may take while the user answers SSH's questions.
const INTERACTIVE_CONNECT_TIMEOUT: Duration = Duration::from_secs(180);
/// After a failed connection, how long requests fail at once before the
/// next attempt (MonoCode keeps a failure for 10 seconds).
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// Delays between attempts to reconnect for a run still followed.
const RECONNECT_BACKOFF: [u64; 6] = [1, 2, 5, 10, 20, 30];

/// What `LOST_RUN` says on this side: the host no longer has the turn.
const RUN_GONE: &str =
    "The machine no longer has this turn, so it was cut off. Send a message to continue.";

/// What the Settings page and the rail show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing asked for it yet.
    Idle,
    Connecting,
    Connected(HostInfo),
    /// Why the last attempt failed.
    Offline(String),
}

enum Link {
    Down {
        error: Option<String>,
        at: Option<Instant>,
    },
    Connecting,
    Up {
        stdin: Arc<Mutex<ChildStdin>>,
        child: Arc<Mutex<Child>>,
    },
}

struct Follower {
    tx: UnboundedSender<AgentEvent>,
    /// The last event passed on.
    seen: u64,
}

struct Inner {
    link: Link,
    /// Bumped on every new link, so a dead link's reader leaves the next alone.
    generation: u64,
    next_id: u64,
    pending: HashMap<u64, std::sync::mpsc::Sender<Result<serde_json::Value, String>>>,
    runs: HashMap<String, Follower>,
    info: Option<HostInfo>,
    /// The machine was removed; nothing opens the link again.
    closed: bool,
    reconnecting: bool,
}

pub struct Connection {
    /// This connection, for the threads it starts.
    this: Weak<Self>,
    machine: RwLock<Machine>,
    inner: Mutex<Inner>,
    changed: Condvar,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connection {
    pub fn new(machine: Machine, on_change: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            machine: RwLock::new(machine),
            inner: Mutex::new(Inner {
                link: Link::Down {
                    error: None,
                    at: None,
                },
                generation: 0,
                next_id: 0,
                pending: HashMap::new(),
                runs: HashMap::new(),
                info: None,
                closed: false,
                reconnecting: false,
            }),
            changed: Condvar::new(),
            on_change,
        })
    }

    pub fn machine(&self) -> Machine {
        self.machine
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_machine(&self, machine: Machine) {
        *self.machine.write().unwrap_or_else(PoisonError::into_inner) = machine;
    }

    pub fn status(&self) -> Status {
        let inner = lock(&self.inner);
        match &inner.link {
            Link::Up { .. } => inner
                .info
                .clone()
                .map_or(Status::Connecting, Status::Connected),
            Link::Connecting => Status::Connecting,
            Link::Down {
                error: Some(error), ..
            } => Status::Offline(error.clone()),
            Link::Down { error: None, .. } => Status::Idle,
        }
    }

    /// Opens the link when it is down. With `askpass`, SSH may ask the user
    /// (Settings' Connect and Reconnect); without it, it never asks, and a
    /// failure in the last `RETRY_AFTER` fails at once. `master` shares a
    /// setup's SSH login.
    pub fn connect(
        self: &Arc<Self>,
        askpass: Option<&Askpass>,
        master: Option<&Path>,
    ) -> Result<HostInfo, String> {
        let generation = {
            let mut inner = lock(&self.inner);
            loop {
                if inner.closed {
                    return Err("This machine was removed.".into());
                }
                match &inner.link {
                    Link::Up { .. } => {
                        if let Some(info) = &inner.info {
                            return Ok(info.clone());
                        }
                    }
                    Link::Connecting => {}
                    Link::Down { error, at } => {
                        let recent = at.is_some_and(|at| at.elapsed() < RETRY_AFTER);
                        if let (Some(error), true, None) = (error, recent, askpass) {
                            return Err(error.clone());
                        }
                        break;
                    }
                }
                let (next, _) = self
                    .changed
                    .wait_timeout(inner, INTERACTIVE_CONNECT_TIMEOUT)
                    .unwrap_or_else(PoisonError::into_inner);
                inner = next;
            }
            inner.link = Link::Connecting;
            inner.generation += 1;
            inner.generation
        };
        (self.on_change)();
        let opened = self.open(generation, askpass, master);
        let result = {
            let mut inner = lock(&self.inner);
            match opened {
                Ok((stdin, child, info)) if inner.generation == generation && !inner.closed => {
                    inner.link = Link::Up { stdin, child };
                    inner.info = Some(info.clone());
                    Ok(info)
                }
                Ok((_, child, _)) => {
                    kill(&child);
                    Err("This machine was removed.".to_string())
                }
                Err(error) => {
                    if inner.generation == generation {
                        inner.link = Link::Down {
                            error: Some(error.clone()),
                            at: Some(Instant::now()),
                        };
                    }
                    Err(error)
                }
            }
        };
        self.changed.notify_all();
        (self.on_change)();
        if result.is_ok() {
            self.reattach_runs();
        }
        result
    }

    /// Starts ssh, its reader, and says hello.
    #[allow(clippy::type_complexity)]
    fn open(
        self: &Arc<Self>,
        generation: u64,
        askpass: Option<&Askpass>,
        master: Option<&Path>,
    ) -> Result<(Arc<Mutex<ChildStdin>>, Arc<Mutex<Child>>, HostInfo), String> {
        let machine = self.machine();
        let mut command = super::ssh::command(&machine, askpass.is_some(), master);
        if let Some(askpass) = askpass {
            askpass.configure(&mut command)?;
        }
        command
            .arg("--")
            .arg(&machine.target)
            .arg(super::ssh::connect_command());
        let mut child = command
            .spawn()
            .map_err(|err| format!("Could not start OpenSSH: {err}"))?;
        let stdin = child.stdin.take().ok_or("ssh has no stdin")?;
        let stdout = child.stdout.take().ok_or("ssh has no stdout")?;
        let stderr = child.stderr.take().ok_or("ssh has no stderr")?;
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || keep_tail(stderr, &tail));
        }
        let stdin = Arc::new(Mutex::new(stdin));
        let child = Arc::new(Mutex::new(child));
        {
            let this = self.clone();
            let tail = stderr_tail.clone();
            let child = child.clone();
            std::thread::Builder::new()
                .name(format!("remote-{}", machine.id))
                .spawn(move || this.read_frames(generation, stdout, &tail, &child))
                .map_err(|err| err.to_string())?;
        }
        let (id, reply) = self.register();
        let hello = ClientFrame {
            id,
            request: Request::Hello {
                protocol: PROTOCOL_VERSION,
                client: format!("BenCode {}", env!("CARGO_PKG_VERSION")),
            },
        };
        let timeout = if askpass.is_some() {
            INTERACTIVE_CONNECT_TIMEOUT
        } else {
            CONNECT_TIMEOUT
        };
        let answered = write_frame(&stdin, &hello)
            .map_err(|err| err.to_string())
            .and_then(|()| match reply.recv_timeout(timeout) {
                Ok(result) => result,
                Err(_) => Err("timed out".into()),
            })
            .and_then(|value| {
                serde_json::from_value::<HostInfo>(value).map_err(|err| err.to_string())
            });
        match answered {
            Ok(info) if info.environment_id == machine.id || machine.id.is_empty() => {
                Ok((stdin, child, info))
            }
            Ok(info) => {
                kill(&child);
                Err(format!(
                    "{} answered as another host ({}). Remove it and add it again.",
                    machine.target, info.environment_id
                ))
            }
            Err(error) => {
                kill(&child);
                // What ssh said is the useful part (a refused key, a host
                // that changed, no `bencode-host` installed).
                std::thread::sleep(Duration::from_millis(100));
                let said = lock(&stderr_tail).trim().to_string();
                Err(offline_message(&machine, &error, &said))
            }
        }
    }

    /// Reads the host's frames until the link ends.
    fn read_frames(
        self: Arc<Self>,
        generation: u64,
        stdout: impl std::io::Read,
        stderr_tail: &Mutex<String>,
        child: &Mutex<Child>,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = (&mut reader)
                .take(MAX_FRAME_BYTES as u64 + 1)
                .read_until(b'\n', &mut line);
            match read {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() > MAX_FRAME_BYTES => {
                    log::error!("remote: a frame over the limit; dropping the connection");
                    break;
                }
                Ok(_) => {}
            }
            match serde_json::from_slice::<ServerFrame>(&line) {
                Ok(frame) => self.dispatch(frame),
                Err(err) => log::warn!("remote: unreadable frame: {err}"),
            }
        }
        if let Ok(Some(status)) = lock(child).try_wait() {
            log::info!("remote: ssh exited with {status}");
        }
        kill(child);
        let said = lock(stderr_tail).trim().to_string();
        self.link_lost(generation, &said);
    }

    fn dispatch(&self, frame: ServerFrame) {
        let mut inner = lock(&self.inner);
        match frame {
            ServerFrame::Reply { id, result, error } => {
                if let Some(tx) = inner.pending.remove(&id) {
                    let _ = tx.send(match error {
                        Some(error) => Err(error),
                        None => Ok(result.unwrap_or(serde_json::Value::Null)),
                    });
                }
            }
            ServerFrame::Event { run, seq, event } => {
                let Some(follower) = inner.runs.get_mut(&run) else {
                    return;
                };
                if seq <= follower.seen {
                    return;
                }
                if seq != follower.seen + 1 {
                    log::warn!(
                        "remote run {run}: event {seq} after {}; reading again",
                        follower.seen
                    );
                    let after = follower.seen;
                    drop(inner);
                    self.send_attach(&run, after);
                    return;
                }
                follower.seen = seq;
                if follower.tx.send(event).is_err() {
                    // The app let go of it (the thread was deleted).
                    inner.runs.remove(&run);
                }
            }
            ServerFrame::RunEnded { run, seq } => {
                let Some(follower) = inner.runs.get(&run) else {
                    return;
                };
                if follower.seen < seq {
                    let after = follower.seen;
                    drop(inner);
                    self.send_attach(&run, after);
                    return;
                }
                // Dropping the sender ends the app's stream: the turn is over.
                inner.runs.remove(&run);
                drop(inner);
                self.forget_run(&run);
            }
        }
    }

    fn link_lost(self: &Arc<Self>, generation: u64, stderr: &str) {
        let follow_up = {
            let mut inner = lock(&self.inner);
            if inner.generation != generation {
                return;
            }
            let machine = self.machine();
            let reason = if stderr.is_empty() {
                format!("The connection to {} dropped.", machine.name)
            } else {
                offline_message(&machine, "the connection dropped", stderr)
            };
            if matches!(inner.link, Link::Up { .. }) {
                log::info!("remote: {reason}");
            }
            inner.link = Link::Down {
                error: (!inner.closed).then_some(reason),
                at: Some(Instant::now()),
            };
            for (_, tx) in inner.pending.drain() {
                let _ = tx.send(Err("The connection to the machine dropped.".into()));
            }
            !inner.runs.is_empty() && !inner.closed
        };
        self.changed.notify_all();
        (self.on_change)();
        if follow_up {
            self.keep_reconnecting();
        }
    }

    /// While a run is followed, the link is opened again in the background
    /// (never asking anything), backing off between attempts.
    fn keep_reconnecting(self: &Arc<Self>) {
        {
            let mut inner = lock(&self.inner);
            if inner.reconnecting {
                return;
            }
            inner.reconnecting = true;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let mut attempt = 0;
            loop {
                let wanted = {
                    let inner = lock(&this.inner);
                    !inner.closed
                        && !inner.runs.is_empty()
                        && !matches!(inner.link, Link::Up { .. })
                };
                if !wanted {
                    break;
                }
                let wait = RECONNECT_BACKOFF[attempt.min(RECONNECT_BACKOFF.len() - 1)];
                std::thread::sleep(Duration::from_secs(wait));
                attempt += 1;
                // Past the failure window, so this is a real attempt.
                {
                    let mut inner = lock(&this.inner);
                    if let Link::Down { at, .. } = &mut inner.link {
                        *at = None;
                    }
                }
                if let Err(err) = this.connect(None, None) {
                    log::info!("remote: reconnecting to {}: {err}", this.machine().name);
                }
            }
            lock(&this.inner).reconnecting = false;
        });
    }

    fn register(
        &self,
    ) -> (
        u64,
        std::sync::mpsc::Receiver<Result<serde_json::Value, String>>,
    ) {
        let mut inner = lock(&self.inner);
        inner.next_id += 1;
        let id = inner.next_id;
        let (tx, rx) = std::sync::mpsc::channel();
        inner.pending.insert(id, tx);
        (id, rx)
    }

    fn stdin(&self) -> Option<Arc<Mutex<ChildStdin>>> {
        match &lock(&self.inner).link {
            Link::Up { stdin, .. } => Some(stdin.clone()),
            _ => None,
        }
    }

    /// Sends `request` and waits for its reply, opening the link first when
    /// it is down.
    pub fn call_value(
        self: &Arc<Self>,
        request: Request,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        self.connect(None, None)?;
        let stdin = self
            .stdin()
            .ok_or("The connection to the machine dropped.")?;
        let (id, reply) = self.register();
        if let Err(err) = write_frame(&stdin, &ClientFrame { id, request }) {
            lock(&self.inner).pending.remove(&id);
            return Err(format!("Could not reach the machine: {err}"));
        }
        let result = reply.recv_timeout(timeout);
        lock(&self.inner).pending.remove(&id);
        match result {
            Ok(result) => result,
            Err(_) => Err("The machine did not answer in time.".into()),
        }
    }

    pub fn call<T: DeserializeOwned>(
        self: &Arc<Self>,
        request: Request,
        timeout: Duration,
    ) -> Result<T, String> {
        let value = self.call_value(request, timeout)?;
        serde_json::from_value(value)
            .map_err(|err| format!("The machine's answer is unreadable: {err}"))
    }

    /// Passes `run`'s events after `after` to `tx` from now on. A new run
    /// follows from 0 before `RunStart` goes out; a run the app found again
    /// after a restart also starts at 0 and is read again (`RunAttach`).
    pub fn follow(self: &Arc<Self>, run: &str, tx: UnboundedSender<AgentEvent>) {
        lock(&self.inner)
            .runs
            .insert(run.to_string(), Follower { tx, seen: 0 });
    }

    pub fn unfollow(&self, run: &str) {
        lock(&self.inner).runs.remove(run);
    }

    /// Asks the host for `run` again, from 0. Opens the link if it must; a
    /// link that cannot open now is retried in the background.
    pub fn attach(self: &Arc<Self>, run: &str) {
        match self.connect(None, None) {
            // `connect` reattached every followed run on opening.
            Ok(_) => self.send_attach(run, 0),
            Err(err) => {
                log::info!("remote: run {run} waits for the machine: {err}");
                self.keep_reconnecting();
            }
        }
    }

    fn reattach_runs(self: &Arc<Self>) {
        let runs: Vec<(String, u64)> = lock(&self.inner)
            .runs
            .iter()
            .map(|(run, follower)| (run.clone(), follower.seen))
            .collect();
        for (run, after) in runs {
            self.send_attach(&run, after);
        }
    }

    /// `RunAttach` without waiting on the caller's thread: its events come
    /// on the reader, its reply here.
    fn send_attach(&self, run: &str, after: u64) {
        let Some(this) = self.self_arc() else {
            return;
        };
        let run = run.to_string();
        std::thread::spawn(move || {
            let attached: Result<RunAttached, String> = this.call(
                Request::RunAttach {
                    run: run.clone(),
                    after,
                },
                DEFAULT_TIMEOUT,
            );
            match attached {
                Ok(RunAttached { known: true }) => {}
                Ok(RunAttached { known: false }) => {
                    let follower = lock(&this.inner).runs.remove(&run);
                    if let Some(follower) = follower {
                        log::warn!("remote: the host has no run {run}");
                        let _ = follower.tx.send(AgentEvent::Error(RUN_GONE.into()));
                        let _ = follower.tx.send(AgentEvent::Done(DoneStatus::Failed));
                    }
                }
                Err(err) => log::info!("remote: following run {run} again: {err}"),
            }
        });
    }

    fn forget_run(&self, run: &str) {
        let Some(this) = self.self_arc() else {
            return;
        };
        let run = run.to_string();
        std::thread::spawn(move || {
            let forgot: Result<(), String> =
                this.call(Request::RunForget { run: run.clone() }, DEFAULT_TIMEOUT);
            if let Err(err) = forgot {
                log::info!("remote: the host keeps run {run} a while: {err}");
            }
        });
    }

    fn self_arc(&self) -> Option<Arc<Self>> {
        self.this.upgrade()
    }

    /// The link is dropped and stays down: the machine was removed.
    pub fn close(&self) {
        let child = {
            let mut inner = lock(&self.inner);
            inner.closed = true;
            inner.runs.clear();
            let link = std::mem::replace(
                &mut inner.link,
                Link::Down {
                    error: None,
                    at: None,
                },
            );
            match link {
                Link::Up { child, .. } => Some(child),
                _ => None,
            }
        };
        if let Some(child) = child {
            kill(&child);
        }
        self.changed.notify_all();
    }

    /// Drops the link (Settings' Reconnect starts afresh); runs stay followed.
    pub fn drop_link(&self) {
        let child = {
            let mut inner = lock(&self.inner);
            inner.generation += 1;
            let link = std::mem::replace(
                &mut inner.link,
                Link::Down {
                    error: None,
                    at: None,
                },
            );
            for (_, tx) in inner.pending.drain() {
                let _ = tx.send(Err("The connection was restarted.".into()));
            }
            match link {
                Link::Up { child, .. } => Some(child),
                _ => None,
            }
        };
        if let Some(child) = child {
            kill(&child);
        }
        self.changed.notify_all();
        (self.on_change)();
    }
}

fn write_frame(stdin: &Mutex<ChildStdin>, frame: &ClientFrame) -> std::io::Result<()> {
    let line = serde_json::to_string(frame).map_err(std::io::Error::other)?;
    let mut stdin = lock(stdin);
    stdin.write_all(line.as_bytes())?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

fn kill(child: &Mutex<Child>) {
    let mut child = lock(child);
    let _ = child.kill();
    let _ = child.wait();
}

fn keep_tail(mut stderr: impl std::io::Read, tail: &Mutex<String>) {
    let mut buffer = [0u8; 2048];
    while let Ok(n) = stderr.read(&mut buffer) {
        if n == 0 {
            break;
        }
        let mut tail = lock(tail);
        tail.push_str(&String::from_utf8_lossy(&buffer[..n]));
        if tail.len() > 8192 {
            let cut = tail.len() - 4096;
            let cut = (cut..tail.len())
                .find(|&i| tail.is_char_boundary(i))
                .unwrap_or(0);
            tail.drain(..cut);
        }
    }
}

/// What Settings shows for a link that would not open.
fn offline_message(machine: &Machine, error: &str, ssh_said: &str) -> String {
    let said: String = ssh_said
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(600)
        .collect();
    if said.contains("bencode-host")
        && (said.contains("not found") || said.contains("No such file"))
    {
        return format!(
            "BenCode Host is not installed on {}. Open Settings › Connections and click Reconnect.",
            machine.name
        );
    }
    if said.contains("Permission denied") || said.contains("Host key verification failed") {
        return format!(
            "SSH needs you to sign in to {} ({said}). Open Settings › Connections and click Reconnect.",
            machine.name
        );
    }
    if said.is_empty() {
        format!("Could not connect to {}: {error}.", machine.name)
    } else {
        format!("Could not connect to {}: {said}", machine.name)
    }
}
