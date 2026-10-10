//! Shared child-process plumbing for every harness: spawn, stdin writer,
//! stdout line pump, stderr capture, cancellation and exit reporting.
//! Harness modules only describe the command line and parse stdout lines.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

use crate::harness::accounts::AccountProfile;
use crate::harness::attachments::Attachment;
use crate::harness::events::{AgentEvent, DoneStatus};
use crate::harness::handle::{HarnessProcessHandle, PermissionResponder, StdinMsg, Steer};
use crate::harness::runtime::runtime;

const STDERR_TAIL_LINES: usize = 20;
/// How long a child may keep running once its turn is `Done` and its stdin
/// closed; an agent server that ignores EOF is killed after this.
const EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Turns one stdout line into zero or more events. Implementations are pure
/// state machines so they can be unit-tested with recorded transcripts.
pub trait LineParser: Send + 'static {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent>;

    /// Lines the protocol must answer with after the last `parse_line`
    /// (a JSON-RPC handshake step, an automatic approval). Written to stdin
    /// before that line's events go out; each gets a trailing newline.
    fn take_replies(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// A follow-up the user sent into the running turn (MonoCode
    /// `steerTurn`). Only called for a `ProcessSpec` with `can_steer`; what
    /// it must write comes back from `take_replies`.
    fn steer(&mut self, _prompt: &str, _attachments: &[Attachment]) {}
}

pub enum StdinMode {
    /// The prompt travels on argv; the child gets no stdin.
    Null,
    /// Write `initial` then keep stdin open for protocol replies until the
    /// parser reports `Done`.
    Protocol { initial: String },
}

pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: String,
    pub stdin: StdinMode,
    pub permission_responder: Option<PermissionResponder>,
    /// Whether the parser takes follow-ups mid-turn (`LineParser::steer`).
    pub can_steer: bool,
    /// The account profile the child signs in with, if not the default.
    pub account: Option<AccountProfile>,
}

type EventRx = mpsc::UnboundedReceiver<AgentEvent>;
type StderrTail = Arc<Mutex<VecDeque<String>>>;

/// Spawns the child on the harness runtime. Safe to call from any thread.
pub fn spawn(
    spec: ProcessSpec,
    parser: impl LineParser,
) -> Result<(HarnessProcessHandle, EventRx)> {
    let _guard = runtime().enter();

    let mut cmd = Command::new(&spec.program);
    cmd.current_dir(&spec.cwd)
        .args(&spec.args)
        .env("PATH", child_path(&spec.program))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(account) = &spec.account {
        account.apply_async(&mut cmd);
    }
    cmd.stdin(match spec.stdin {
        StdinMode::Null => Stdio::null(),
        StdinMode::Protocol { .. } => Stdio::piped(),
    });

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to start {}", spec.program.display()))?;

    let stdout = child.stdout.take().context("child stdout unavailable")?;
    let stderr = child.stderr.take().context("child stderr unavailable")?;

    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (stdin_tx, stdin_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();

    if let StdinMode::Protocol { initial } = spec.stdin {
        let stdin = child.stdin.take().context("child stdin unavailable")?;
        let _ = stdin_tx.send(StdinMsg::Line(initial));
        tokio::spawn(write_stdin(stdin, stdin_rx));
    }

    let stderr_tail: StderrTail = Arc::default();
    let stderr_task = tokio::spawn(capture_stderr(stderr, stderr_tail.clone()));

    let (steer_tx, steer_rx) = if spec.can_steer {
        let (tx, rx) = mpsc::unbounded_channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let handle = HarnessProcessHandle::new(
        stdin_tx.clone(),
        cancel_tx,
        spec.permission_responder,
        steer_tx,
    );
    tokio::spawn(pump(PumpCtx {
        child,
        stdout,
        parser,
        event_tx,
        stdin_tx,
        cancel_rx,
        steer_rx,
        stderr_task,
        stderr_tail,
    }));

    Ok((handle, event_rx))
}

async fn write_stdin(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<StdinMsg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            StdinMsg::Line(line) => {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            StdinMsg::Close => break,
        }
    }
    // Dropping stdin sends EOF so protocol-mode children exit after the turn.
}

async fn capture_stderr(stderr: tokio::process::ChildStderr, tail: StderrTail) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        log::debug!("harness stderr: {line}");
        let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
        if tail.len() == STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

struct PumpCtx<P> {
    child: Child,
    stdout: tokio::process::ChildStdout,
    parser: P,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    stdin_tx: mpsc::UnboundedSender<StdinMsg>,
    cancel_rx: oneshot::Receiver<()>,
    steer_rx: Option<mpsc::UnboundedReceiver<Steer>>,
    stderr_task: tokio::task::JoinHandle<()>,
    stderr_tail: StderrTail,
}

/// The next follow-up; never resolves for a harness that takes none.
async fn next_steer(rx: &mut Option<mpsc::UnboundedReceiver<Steer>>) -> Option<Steer> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn pump<P: LineParser>(mut ctx: PumpCtx<P>) {
    let mut lines = BufReader::new(ctx.stdout).lines();
    let mut done = false;
    let exit_deadline = tokio::time::sleep(std::time::Duration::MAX);
    tokio::pin!(exit_deadline);

    loop {
        tokio::select! {
            () = &mut exit_deadline, if done => {
                log::debug!("harness still running {EXIT_GRACE:?} after its turn; killing it");
                let _ = ctx.child.kill().await;
                break;
            }
            Ok(()) = &mut ctx.cancel_rx => {
                let _ = ctx.child.kill().await;
                if !done {
                    let _ = ctx.event_tx.send(AgentEvent::Done(DoneStatus::Cancelled));
                }
                return;
            }
            steer = next_steer(&mut ctx.steer_rx), if !done => match steer {
                Some(steer) => {
                    ctx.parser.steer(&steer.prompt, &steer.attachments);
                    for reply in ctx.parser.take_replies() {
                        let _ = ctx.stdin_tx.send(StdinMsg::Line(format!("{reply}\n")));
                    }
                }
                // Every handle is gone; nothing more can be sent.
                None => ctx.steer_rx = None,
            },
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let events = ctx.parser.parse_line(&line);
                    for reply in ctx.parser.take_replies() {
                        if !done {
                            let _ = ctx.stdin_tx.send(StdinMsg::Line(format!("{reply}\n")));
                        }
                    }
                    for event in events {
                        // The turn is closed once Done is seen; later output is noise.
                        if done {
                            break;
                        }
                        if matches!(event, AgentEvent::Done(_)) {
                            done = true;
                            let _ = ctx.stdin_tx.send(StdinMsg::Close);
                            exit_deadline
                                .as_mut()
                                .reset(tokio::time::Instant::now() + EXIT_GRACE);
                        }
                        let _ = ctx.event_tx.send(event);
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = ctx.event_tx.send(AgentEvent::Error(format!("reading harness output: {err}")));
                    break;
                }
            }
        }
    }

    let exit = ctx.child.wait().await;
    let _ = ctx.stderr_task.await;
    if done {
        return;
    }

    let succeeded = exit.as_ref().is_ok_and(|status| status.success());
    if succeeded {
        let _ = ctx.event_tx.send(AgentEvent::Done(DoneStatus::Completed));
        return;
    }

    let tail = ctx.stderr_tail.lock().unwrap_or_else(|p| p.into_inner());
    let reason = match &exit {
        Ok(status) => format!("harness exited with {status}"),
        Err(err) => format!("waiting for harness: {err}"),
    };
    let message = if tail.is_empty() {
        reason
    } else {
        format!(
            "{reason}\n{}",
            tail.iter().cloned().collect::<Vec<_>>().join("\n")
        )
    };
    let _ = ctx.event_tx.send(AgentEvent::Error(message));
    let _ = ctx.event_tx.send(AgentEvent::Done(DoneStatus::Failed));
}

/// GUI apps launched from Finder inherit a minimal PATH. Make sure the
/// harness can find its own siblings (node, git, rg, …).
pub(crate) fn child_path(program: &std::path::Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = program.parent().filter(|p| !p.as_os_str().is_empty()) {
        dirs.push(parent.to_path_buf());
    }
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    dirs.dedup();
    std::env::join_paths(dirs).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoParser;

    impl LineParser for EchoParser {
        fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
            vec![AgentEvent::TextDelta(line.to_string())]
        }
    }

    fn sh(script: &str) -> ProcessSpec {
        ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            stdin: StdinMode::Null,
            permission_responder: None,
            can_steer: false,
            account: None,
        }
    }

    fn collect(mut rx: EventRx) -> Vec<AgentEvent> {
        runtime().block_on(async move {
            let mut out = Vec::new();
            while let Some(event) = rx.recv().await {
                out.push(event);
            }
            out
        })
    }

    #[test]
    fn successful_exit_emits_lines_then_completed() {
        let (_handle, rx) = spawn(sh("echo one; echo two"), EchoParser).unwrap();
        assert_eq!(
            collect(rx),
            vec![
                AgentEvent::TextDelta("one".into()),
                AgentEvent::TextDelta("two".into()),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    #[test]
    fn failing_exit_reports_stderr_tail() {
        let (_handle, rx) = spawn(sh("echo boom >&2; exit 3"), EchoParser).unwrap();
        let events = collect(rx);
        let AgentEvent::Error(message) = &events[0] else {
            panic!("expected error, got {events:?}");
        };
        assert!(message.contains("boom"), "{message}");
        assert_eq!(events.last(), Some(&AgentEvent::Done(DoneStatus::Failed)));
    }

    #[test]
    fn cancel_kills_child_and_reports_cancelled() {
        let (handle, rx) = spawn(sh("sleep 30"), EchoParser).unwrap();
        handle.cancel();
        assert_eq!(collect(rx), vec![AgentEvent::Done(DoneStatus::Cancelled)]);
    }

    #[test]
    fn protocol_stdin_is_delivered_and_closed_after_done() {
        struct DoneOnLine;
        impl LineParser for DoneOnLine {
            fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
                vec![
                    AgentEvent::TextDelta(line.into()),
                    AgentEvent::Done(DoneStatus::Completed),
                ]
            }
        }
        let mut spec = sh("read first; echo \"got:$first\"; cat");
        spec.stdin = StdinMode::Protocol {
            initial: "hello\n".into(),
        };
        let (_handle, rx) = spawn(spec, DoneOnLine).unwrap();
        // `cat` only exits because stdin is closed once Done is seen.
        assert_eq!(
            collect(rx),
            vec![
                AgentEvent::TextDelta("got:hello".into()),
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn parser_replies_reach_stdin() {
        /// Answers the first line, finishes on the second.
        #[derive(Default)]
        struct Handshake {
            replies: Vec<String>,
        }
        impl LineParser for Handshake {
            fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
                if line == "ready" {
                    self.replies.push("go".into());
                    return Vec::new();
                }
                vec![
                    AgentEvent::TextDelta(line.into()),
                    AgentEvent::Done(DoneStatus::Completed),
                ]
            }
            fn take_replies(&mut self) -> Vec<String> {
                std::mem::take(&mut self.replies)
            }
        }
        let mut spec = sh("read first; echo ready; read second; echo \"$first $second\"; cat");
        spec.stdin = StdinMode::Protocol {
            initial: "hello\n".into(),
        };
        let (_handle, rx) = spawn(spec, Handshake::default()).unwrap();
        assert_eq!(
            collect(rx),
            vec![
                AgentEvent::TextDelta("hello go".into()),
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn a_follow_up_reaches_the_parser_and_its_line_the_child() {
        /// Writes each follow-up to the child; finishes on its echo.
        #[derive(Default)]
        struct Steered {
            replies: Vec<String>,
        }
        impl LineParser for Steered {
            fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
                vec![
                    AgentEvent::TextDelta(line.into()),
                    AgentEvent::Done(DoneStatus::Completed),
                ]
            }
            fn take_replies(&mut self) -> Vec<String> {
                std::mem::take(&mut self.replies)
            }
            fn steer(&mut self, prompt: &str, _: &[Attachment]) {
                self.replies.push(format!("steered:{prompt}"));
            }
        }
        let mut spec = sh("read first; read second; echo \"$second\"");
        spec.stdin = StdinMode::Protocol {
            initial: "hello\n".into(),
        };
        spec.can_steer = true;
        let (handle, rx) = spawn(spec, Steered::default()).unwrap();
        assert!(handle.can_steer());
        assert!(handle.steer("also this", &[]));
        assert_eq!(
            collect(rx),
            vec![
                AgentEvent::TextDelta("steered:also this".into()),
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );

        let (handle, _rx) = spawn(sh("sleep 30"), EchoParser).unwrap();
        assert!(!handle.can_steer());
        assert!(!handle.steer("no", &[]));
    }

    #[test]
    fn a_child_that_outlives_its_turn_is_killed() {
        struct DoneAtOnce;
        impl LineParser for DoneAtOnce {
            fn parse_line(&mut self, _: &str) -> Vec<AgentEvent> {
                vec![AgentEvent::Done(DoneStatus::Completed)]
            }
        }
        // Ignores the closed stdin and keeps stdout open.
        let started = std::time::Instant::now();
        let (_handle, rx) = spawn(sh("echo done; exec sleep 30"), DoneAtOnce).unwrap();
        assert_eq!(collect(rx), vec![AgentEvent::Done(DoneStatus::Completed)]);
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }

    #[test]
    fn events_after_done_are_dropped() {
        struct DoneThenNoise;
        impl LineParser for DoneThenNoise {
            fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
                vec![
                    AgentEvent::Done(DoneStatus::Completed),
                    AgentEvent::TextDelta(line.into()),
                ]
            }
        }
        let (_handle, rx) = spawn(sh("echo a; echo b"), DoneThenNoise).unwrap();
        assert_eq!(collect(rx), vec![AgentEvent::Done(DoneStatus::Completed)]);
    }
}
