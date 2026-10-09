//! The macOS `security` tool, which every Keychain read and write goes
//! through (Claude's usage token, Antigravity's sign-in). Blocking, for up
//! to five seconds: call it on a background executor.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

/// What `security` printed, and whether it succeeded.
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `security args…`. Fails when it cannot start or outlives the
/// timeout (a locked Keychain waiting on a prompt).
pub fn run(args: &[&str]) -> Result<Output, String> {
    run_with_input(args, None)
}

/// Runs `security` with `input` on its stdin: `security -i` reads its
/// commands there, so a secret it writes never shows on an argv.
pub fn run_with_input(args: &[&str], input: Option<&str>) -> Result<Output, String> {
    let mut child = Command::new("security")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // Dropping stdin afterwards ends `security -i`'s command list.
        stdin
            .write_all(input.as_bytes())
            .map_err(|err| format!("could not write to security: {err}"))?;
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (mut stdout, mut stderr) = (String::new(), String::new());
                if let Some(mut pipe) = child.stdout.take()
                    && let Err(read) = pipe.read_to_string(&mut stdout)
                {
                    log::debug!("security stdout: {read}");
                }
                if let Some(mut pipe) = child.stderr.take()
                    && let Err(read) = pipe.read_to_string(&mut stderr)
                {
                    log::debug!("security stderr: {read}");
                }
                return Ok(Output {
                    ok: status.success(),
                    stdout: stdout.trim().to_string(),
                    stderr: stderr.trim().to_string(),
                });
            }
            Ok(None) if started.elapsed() > TIMEOUT => {
                if let Err(err) = child.kill() {
                    log::debug!("security already exited: {err}");
                }
                if let Err(err) = child.wait() {
                    log::debug!("security wait: {err}");
                }
                return Err("the Keychain did not answer".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(err) => return Err(err.to_string()),
        }
    }
}

/// `security`'s answer for an item that does not exist.
pub fn not_found(output: &Output) -> bool {
    output.stderr.contains("could not be found")
}
