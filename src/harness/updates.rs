//! MonoCode `harness_updates.rs` and `harnessUpdates.ts`: whether an
//! installed CLI is behind its npm release, and running the CLI's own
//! updater. Blocking; run these on a background executor.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;

use crate::harness::HarnessKind;
use crate::harness::process::child_path;

const REGISTRY_URL: &str = "https://registry.npmjs.org";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// A download plus, for npm installs, a full dependency install.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessUpdate {
    pub harness: HarnessKind,
    pub installed: String,
    pub latest: String,
}

/// Only harnesses whose releases are published to npm. The rest ship through
/// their own installers with no public version feed to compare against.
fn npm_package(kind: HarnessKind) -> Option<&'static str> {
    match kind {
        HarnessKind::Claude => Some("@anthropic-ai/claude-code"),
        HarnessKind::Codex => Some("@openai/codex"),
        HarnessKind::OpenCode => Some("opencode-ai"),
        HarnessKind::Antigravity => None,
    }
}

/// Each CLI's own updater, which knows how it was installed (native, npm,
/// Homebrew) better than BenCode could guess from the binary path.
fn update_args(kind: HarnessKind) -> Option<&'static [&'static str]> {
    match kind {
        HarnessKind::Claude | HarnessKind::Codex => Some(&["update"]),
        HarnessKind::OpenCode => Some(&["upgrade"]),
        HarnessKind::Antigravity => None,
    }
}

pub fn is_updatable(kind: HarnessKind) -> bool {
    npm_package(kind).is_some() && update_args(kind).is_some()
}

/// `kind`'s update, if its CLI at `program` is behind npm. Any failure,
/// offline or otherwise, is `None`: this runs unprompted at launch and must
/// never surface an error of its own.
pub fn find_update(kind: HarnessKind, program: &Path) -> Option<HarnessUpdate> {
    let package = npm_package(kind)?;
    let installed = installed_version(program)
        .map_err(|err| log::debug!("{} --version: {err:#}", kind.label()))
        .ok()?;
    let latest = latest_version(package)
        .map_err(|err| log::debug!("{} latest version: {err:#}", kind.label()))
        .ok()?;
    (compare_semver(&latest, &installed) == std::cmp::Ordering::Greater).then_some(HarnessUpdate {
        harness: kind,
        installed,
        latest,
    })
}

/// Runs the CLI's self-update, then returns the version it reports
/// afterwards. stdin is closed, so an updater that stops to ask fails
/// instead of hanging.
pub fn run_update(kind: HarnessKind, program: &Path) -> Result<String> {
    let args = update_args(kind).with_context(|| format!("No updater for {}", kind.label()))?;
    let (ok, stdout, stderr) = run(program, args, UPDATE_TIMEOUT)?;
    if !ok {
        bail!("{}", update_failure(&stdout, &stderr));
    }
    installed_version(program)
}

fn installed_version(program: &Path) -> Result<String> {
    let (_, stdout, _) = run(program, &["--version"], VERSION_TIMEOUT)?;
    parse_version(&String::from_utf8_lossy(&stdout)).context("CLI returned no valid version.")
}

fn latest_version(package: &str) -> Result<String> {
    let response = crate::rate_limits::http::get(
        &format!("{REGISTRY_URL}/{package}/latest"),
        &[("Accept", "application/json")],
        HTTP_TIMEOUT,
    )?;
    if response.status != 200 {
        bail!("npm registry answered {}", response.status);
    }
    let body: Value =
        serde_json::from_str(&response.body).context("npm registry returned invalid JSON")?;
    registry_version(&body).context("npm registry returned no version")
}

/// Runs `program args` to completion within `timeout`: (success, stdout, stderr).
fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", child_path(program))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start {}", program.display()))?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Err(err) = pipe.read_to_end(&mut out) {
                log::debug!("reading CLI output: {err}");
            }
            out
        })
    };
    let stdout = drain(Box::new(child.stdout.take().context("no stdout")?));
    let stderr = drain(Box::new(child.stderr.take().context("no stderr")?));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            if let Err(err) = child.kill() {
                log::debug!("killing a timed-out CLI: {err}");
            }
            if let Err(err) = child.wait() {
                log::debug!("reaping a timed-out CLI: {err}");
            }
            bail!("Timed out after {}s.", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let join = |handle: std::thread::JoinHandle<Vec<u8>>| {
        handle.join().map_err(|_| anyhow!("output reader panicked"))
    };
    Ok((status.success(), join(stdout)?, join(stderr)?))
}

/// Updaters print their reason to either stream; the last line is the one
/// that says what went wrong.
fn update_failure(stdout: &[u8], stderr: &[u8]) -> String {
    [stderr, stdout]
        .iter()
        .filter_map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .map(str::to_string)
        })
        .next()
        .unwrap_or_else(|| "Update failed".to_string())
}

fn registry_version(body: &Value) -> Option<String> {
    let version = body.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// MonoCode `parseOpenCodeVersion`: the first `N.N.N` in a CLI's output.
pub fn parse_version(output: &str) -> Option<String> {
    let bytes = output.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_digit() {
            start += 1;
            continue;
        }
        let mut end = start;
        let mut dots = 0;
        while end < bytes.len() {
            let ch = bytes[end];
            if ch.is_ascii_digit() {
                end += 1;
            } else if ch == b'.' && dots < 2 && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
                dots += 1;
                end += 1;
            } else {
                break;
            }
        }
        if dots == 2 {
            return Some(output[start..end].to_string());
        }
        start = end.max(start + 1);
    }
    None
}

/// MonoCode `compareSemver`: major, minor, patch as numbers.
pub fn compare_semver(left: &str, right: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> [u64; 3] {
        let mut out = [0; 3];
        for (slot, part) in out.iter_mut().zip(v.split('.')) {
            *slot = part.parse().unwrap_or(0);
        }
        out
    };
    parts(left).cmp(&parts(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cmp::Ordering;

    #[test]
    fn maps_only_npm_published_harnesses() {
        assert_eq!(
            npm_package(HarnessKind::Claude),
            Some("@anthropic-ai/claude-code")
        );
        assert_eq!(npm_package(HarnessKind::Antigravity), None);
        assert!(is_updatable(HarnessKind::OpenCode));
        assert!(!is_updatable(HarnessKind::Antigravity));
    }

    #[test]
    fn updates_only_through_each_cli_own_updater() {
        assert_eq!(update_args(HarnessKind::OpenCode), Some(&["upgrade"][..]));
        assert_eq!(update_args(HarnessKind::Claude), Some(&["update"][..]));
    }

    #[test]
    fn reports_the_last_line_an_updater_printed() {
        assert_eq!(
            update_failure(
                b"checking\n",
                b"npm ERR! code EACCES\nnpm ERR! permission denied\n\n"
            ),
            "npm ERR! permission denied"
        );
        assert_eq!(update_failure(b"no write access\n", b""), "no write access");
        assert_eq!(update_failure(b"", b""), "Update failed");
    }

    #[test]
    fn reads_version_from_registry_payload() {
        assert_eq!(
            registry_version(&json!({ "name": "opencode-ai", "version": "1.18.33" })),
            Some("1.18.33".to_string())
        );
        assert_eq!(registry_version(&json!({ "version": " " })), None);
        assert_eq!(registry_version(&json!({})), None);
    }

    #[test]
    fn finds_the_first_full_version_in_cli_output() {
        assert_eq!(
            parse_version("2.1.294 (Claude Code)\n").as_deref(),
            Some("2.1.294")
        );
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(
            parse_version("v1.2 build 3.4.5.6").as_deref(),
            Some("3.4.5")
        );
        assert_eq!(parse_version("1.2."), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn compares_versions_numerically() {
        assert_eq!(compare_semver("2.1.295", "2.1.294"), Ordering::Greater);
        assert_eq!(compare_semver("2.1.10", "2.1.9"), Ordering::Greater);
        assert_eq!(compare_semver("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_semver("0.9.0", "1.0.0"), Ordering::Less);
    }
}
