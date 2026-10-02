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

use crate::harness::events::{AgentEvent, DoneStatus};
use crate::harness::handle::{HarnessProcessHandle, PermissionResponder, StdinMsg};
use crate::harness::runtime::runtime;

const STDERR_TAIL_LINES: usize = 20;

/// Turns one stdout line into zero or more events. Implementations are pure
/// state machines so they can be unit-tested with recorded transcripts.
pub trait LineParser: Send + 'static {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent>;
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

    let handle = HarnessProcessHandle::new(stdin_tx.clone(), cancel_tx, spec.permission_responder);
    tokio::spawn(pump(PumpCtx {
        child,
        stdout,
        parser,
        event_tx,
        stdin_tx,
        cancel_rx,
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
    stderr_task: tokio::task::JoinHandle<()>,
    stderr_tail: StderrTail,
}

async fn pump<P: LineParser>(mut ctx: PumpCtx<P>) {
    let mut lines = BufReader::new(ctx.stdout).lines();
    let mut done = false;

    loop {
        tokio::select! {
            Ok(()) = &mut ctx.cancel_rx => {
                let _ = ctx.child.kill().await;
                if !done {
                    let _ = ctx.event_tx.send(AgentEvent::Done(DoneStatus::Cancelled));
                }
                return;
            }
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    for event in ctx.parser.parse_line(&line) {
                        // The turn is closed once Done is seen; later output is noise.
                        if done {
                            break;
                        }
                        if matches!(event, AgentEvent::Done(_)) {
                            done = true;
                            let _ = ctx.stdin_tx.send(StdinMsg::Close);
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
fn child_path(program: &std::path::Path) -> std::ffi::OsString {
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
