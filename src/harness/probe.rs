//! Short-lived CLI children for one-off questions (model catalogs, Codex
//! thread edits): stdout read line by line, every read bounded by one
//! overall deadline, the child killed on drop. Blocking; run these on a
//! background executor.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::harness::accounts::AccountProfile;

/// Runs a listing command in `cwd` to completion within `timeout`.
pub fn run_to_end(command: &mut Command, cwd: &Path, timeout: Duration) -> Result<String> {
    let mut probe = LineProbe::spawn(command, cwd, timeout)?;
    let mut out = String::new();
    while let Some(line) = probe.next_line()? {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// A child whose stdout arrives line by line.
pub struct LineProbe {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    deadline: Instant,
}

impl LineProbe {
    pub fn spawn(command: &mut Command, cwd: &Path, timeout: Duration) -> Result<Self> {
        let mut child = command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("could not start the CLI")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child,
            lines,
            deadline: Instant::now() + timeout,
        })
    }

    pub fn send(&mut self, message: &Value) -> Result<()> {
        let stdin = self.stdin.as_mut().context("stdin closed")?;
        writeln!(stdin, "{message}")?;
        stdin.flush()?;
        Ok(())
    }

    /// The next line, or None once the CLI closes stdout.
    pub fn next_line(&mut self) -> Result<Option<String>> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        match self.lines.recv_timeout(left) {
            Ok(line) => Ok(Some(line)),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(anyhow!("the CLI timed out")),
        }
    }

    /// The next JSON line, skipping anything else the CLI prints.
    pub fn next_json(&mut self) -> Result<Value> {
        loop {
            let line = self
                .next_line()?
                .context("the CLI exited before answering")?;
            let line = line.trim();
            if line.starts_with('{')
                && let Ok(value) = serde_json::from_str::<Value>(line)
            {
                return Ok(value);
            }
        }
    }
}

impl Drop for LineProbe {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Err(err) = self.child.kill() {
            log::debug!("probe already exited: {err}");
        }
        if let Err(err) = self.child.wait() {
            log::debug!("probe wait: {err}");
        }
    }
}

/// `codex app-server`, initialized as MonoCode's client, for a few calls.
pub struct AppServer {
    probe: LineProbe,
    next_id: u64,
}

impl AppServer {
    /// `account` signs the server in with that profile instead of the
    /// default one.
    pub fn open(
        program: &Path,
        cwd: &Path,
        timeout: Duration,
        account: Option<&AccountProfile>,
    ) -> Result<Self> {
        let mut command = Command::new(program);
        command.arg("app-server");
        if let Some(account) = account {
            account.apply(&mut command);
        }
        let probe = LineProbe::spawn(&mut command, cwd, timeout)?;
        let mut server = Self { probe, next_id: 0 };
        server.call(
            "initialize",
            json!({
                "clientInfo": { "name": "monocode", "title": "MonoCode", "version": "0.1.0" },
                "capabilities": { "experimentalApi": true },
            }),
        )?;
        server.probe.send(&json!({ "method": "initialized" }))?;
        Ok(server)
    }

    /// One request and its result; requests from the server meanwhile get
    /// an empty result (nothing is granted during a probe).
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.probe
            .send(&json!({ "id": id, "method": method, "params": params }))?;
        loop {
            let rec = self.probe.next_json()?;
            if let Some(asked) = rec.get("method").and_then(Value::as_str) {
                if let Some(req) = rec.get("id") {
                    log::debug!("codex app-server: answering {asked} with an empty result");
                    self.probe.send(&json!({ "id": req, "result": {} }))?;
                }
                continue;
            }
            if rec.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = rec.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| error.to_string(), str::to_string);
                bail!("codex {method}: {message}");
            }
            return Ok(rec.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}
