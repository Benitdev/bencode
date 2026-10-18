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

/// Claude Code's Keychain item for the sign-in: `config_dir` is an account
/// profile's directory, `None` the default one. Claude Code hashes the exact
/// config dir string (NFC-normalized; BenCode's profile paths are ASCII
/// under an ASCII home, where that changes nothing) and suffixes the first
/// eight hex characters.
pub fn claude_service(config_dir: Option<&std::path::Path>) -> String {
    use sha2::{Digest, Sha256};
    const SERVICE: &str = "Claude Code-credentials";
    let Some(config_dir) = config_dir else {
        return SERVICE.into();
    };
    let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
    let suffix: String = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{SERVICE}-{suffix}")
}

/// Deletes the Claude sign-in an account profile keeps in the Keychain
/// (MonoCode `delete_claude_keychain_credentials`); one already gone is
/// fine. Elsewhere it is `.credentials.json`, which goes with the
/// directory. Blocking.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn delete_claude_credentials(config_dir: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let service = claude_service(Some(config_dir));
        let failed = |detail: String| {
            format!("Could not remove the Claude credentials from Keychain ({service}): {detail}")
        };
        let output = run(&["delete-generic-password", "-s", service.as_str()]).map_err(failed)?;
        if !(output.ok || not_found(&output)) {
            return Err(failed(output.stderr));
        }
    }
    Ok(())
}
