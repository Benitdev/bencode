//! MonoCode `harness_updates.rs` and `harnessUpdates.ts`: whether an
//! installed CLI is behind its latest release (npm's, or for Grok Build
//! xAI's own channel pointer), and running the CLI's own updater. Blocking;
//! run these on a background executor.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::harness::HarnessKind;
use crate::harness::process::child_path;
use crate::harness::resolver::HarnessResolver;

const REGISTRY_URL: &str = "https://registry.npmjs.org";
const GROK_FEED_URL: &str = "https://x.ai/cli";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// A download plus, for npm installs, a full dependency install.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);
/// How often a running updater's CLI is asked its version: some updaters
/// (Grok Build's) install the new release and then never exit.
const INSTALLED_POLL: Duration = Duration::from_secs(10);
/// How long a CLI's output is still read once it has exited: a daemon it
/// started may hold the pipes open for good.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessUpdate {
    pub harness: HarnessKind,
    pub installed: String,
    pub latest: String,
}

/// Where a CLI's latest version is published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Feed {
    /// `<registry>/<package>/latest`.
    Npm(&'static str),
    /// `https://x.ai/cli/<channel>`: the version on its first line, as
    /// Grok's installer reads it.
    Grok,
}

/// Only harnesses with a public version feed. Antigravity ships through
/// its own installer with nothing to compare against.
fn feed(kind: HarnessKind) -> Option<Feed> {
    match kind {
        HarnessKind::Claude => Some(Feed::Npm("@anthropic-ai/claude-code")),
        HarnessKind::Codex => Some(Feed::Npm("@openai/codex")),
        HarnessKind::OpenCode => Some(Feed::Npm("opencode-ai")),
        HarnessKind::Grok => Some(Feed::Grok),
        HarnessKind::Antigravity => None,
    }
}

/// OpenCode 2 is published as another package; `opencode-ai` stays on 1.x.
fn release_line(feed: Feed, installed: &str) -> Feed {
    let major = installed
        .split('.')
        .next()
        .and_then(|m| m.parse::<u64>().ok());
    match feed {
        Feed::Npm("opencode-ai") if major.is_some_and(|m| m >= 2) => Feed::Npm("@opencode/cli"),
        feed => feed,
    }
}

/// Each CLI's own updater, which knows how it was installed (native, npm,
/// Homebrew) better than BenCode could guess from the binary path.
fn update_args(kind: HarnessKind) -> Option<&'static [&'static str]> {
    match kind {
        HarnessKind::Claude | HarnessKind::Codex | HarnessKind::Grok => Some(&["update"]),
        HarnessKind::OpenCode => Some(&["upgrade"]),
        HarnessKind::Antigravity => None,
    }
}

pub fn is_updatable(kind: HarnessKind) -> bool {
    feed(kind).is_some() && update_args(kind).is_some()
}

/// The copy of `kind`'s CLI an update is about: the one turns run. `listed`
/// is where startup found it. Codex may run another copy, the newest of
/// several (asking each its version, hence blocking); `None` when that one
/// is an app's, which `codex update` cannot update.
fn updatable(kind: HarnessKind, listed: &Path) -> Option<PathBuf> {
    if kind != HarnessKind::Codex {
        return Some(listed.to_path_buf());
    }
    HarnessResolver::resolve_codex().filter(|path| !HarnessResolver::is_bundled_codex(path))
}

/// `kind`'s update, if the CLI turns run (`listed`, see `updatable`) is
/// behind its feed. Any failure, offline or otherwise, is `None`: this runs
/// unprompted at launch and must never surface an error of its own.
pub fn find_update(kind: HarnessKind, listed: &Path) -> Option<HarnessUpdate> {
    let feed = feed(kind)?;
    let program = &updatable(kind, listed)?;
    let installed = installed_version(program)
        .map_err(|err| log::debug!("{} --version: {err:#}", kind.label()))
        .ok()?;
    let feed = release_line(feed, &installed);
    let latest = latest_version(feed)
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
/// instead of hanging. One still running once the CLI reports `latest` has
/// done its work and is stopped.
pub fn run_update(kind: HarnessKind, listed: &Path, latest: &str) -> Result<String> {
    let args = update_args(kind).with_context(|| format!("No updater for {}", kind.label()))?;
    let program = &updatable(kind, listed)
        .with_context(|| format!("{} is updated by the app it came with.", kind.label()))?;
    let mut next_poll = Instant::now() + INSTALLED_POLL;
    let mut installed = None;
    let finished = run_while(program, args, UPDATE_TIMEOUT, || {
        if Instant::now() < next_poll {
            return true;
        }
        next_poll = Instant::now() + INSTALLED_POLL;
        installed = installed_version(program)
            .ok()
            .filter(|version| compare_semver(version, latest).is_ge());
        installed.is_none()
    })?;
    match finished {
        Some((ok, stdout, stderr)) if !ok => bail!("{}", update_failure(&stdout, &stderr)),
        Some(_) => installed_version(program),
        None => {
            let version = installed.context("updater stopped without a version")?;
            log::info!(
                "{} updater still running at {version}; stopped it",
                kind.label()
            );
            Ok(version)
        }
    }
}

fn installed_version(program: &Path) -> Result<String> {
    let (_, stdout, _) = run(program, &["--version"], VERSION_TIMEOUT)?;
    parse_version(&String::from_utf8_lossy(&stdout)).context("CLI returned no valid version.")
}

fn latest_version(feed: Feed) -> Result<String> {
    match feed {
        Feed::Npm(package) => {
            let response = crate::http::get(
                &format!("{REGISTRY_URL}/{package}/latest"),
                &[("Accept", "application/json")],
                HTTP_TIMEOUT,
            )?;
            if response.status != 200 {
                bail!("npm registry answered {}", response.status);
            }
            let body: Value = serde_json::from_str(&response.body)
                .context("npm registry returned invalid JSON")?;
            registry_version(&body).context("npm registry returned no version")
        }
        Feed::Grok => {
            let channel = grok_channel(&grok_config());
            let response =
                crate::http::get(&format!("{GROK_FEED_URL}/{channel}"), &[], HTTP_TIMEOUT)?;
            if response.status != 200 {
                bail!("x.ai answered {}", response.status);
            }
            parse_version(response.body.lines().next().unwrap_or_default())
                .context("x.ai returned no version")
        }
    }
}

/// Grok's own `config.toml`; empty when it has none.
fn grok_config() -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return String::new();
    };
    std::fs::read_to_string(Path::new(&home).join(".grok/config.toml")).unwrap_or_default()
}

/// The release channel Grok follows: `[cli] channel` in its config, which
/// its installer writes for anything but stable.
fn grok_channel(config: &str) -> &'static str {
    let mut in_cli = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_cli = line == "[cli]";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if in_cli && key.trim() == "channel" {
            let value = value.split('#').next().unwrap_or_default();
            return match value.trim().trim_matches(['"', '\'']) {
                "alpha" => "alpha",
                "enterprise" => "enterprise",
                _ => "stable",
            };
        }
    }
    "stable"
}

/// Runs `program args` to completion within `timeout`: (success, stdout, stderr).
fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    run_while(program, args, timeout, || true)?.context("CLI stopped early")
}

/// `run`, asking `keep_going` between checks; `None` when it said no and
/// the CLI was killed.
fn run_while(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    mut keep_going: impl FnMut() -> bool,
) -> Result<Option<(bool, Vec<u8>, Vec<u8>)>> {
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", child_path(program))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start {}", program.display()))?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Err(err) = pipe.read_to_end(&mut out) {
                log::debug!("reading CLI output: {err}");
            }
            // The receiver is gone once the grace has passed.
            tx.send(out).ok();
        });
        rx
    };
    let stdout = drain(Box::new(child.stdout.take().context("no stdout")?));
    let stderr = drain(Box::new(child.stderr.take().context("no stderr")?));
    let deadline = Instant::now() + timeout;
    let stop = |child: &mut std::process::Child| {
        if let Err(err) = child.kill() {
            log::debug!("killing a CLI: {err}");
        }
        if let Err(err) = child.wait() {
            log::debug!("reaping a CLI: {err}");
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            bail!("Timed out after {}s.", timeout.as_secs());
        }
        if !keep_going() {
            stop(&mut child);
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let grace = Instant::now() + OUTPUT_GRACE;
    let collect = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(grace.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
    };
    Ok(Some((status.success(), collect(stdout), collect(stderr))))
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
    fn opencode_2_follows_its_own_package() {
        let v1 = Feed::Npm("opencode-ai");
        assert_eq!(release_line(v1, "1.19.3"), v1);
        assert_eq!(release_line(v1, "2.0.32"), Feed::Npm("@opencode/cli"));
        assert_eq!(release_line(Feed::Grok, "2.0.0"), Feed::Grok);
    }

    #[test]
    fn maps_only_harnesses_with_a_version_feed() {
        assert_eq!(
            feed(HarnessKind::Claude),
            Some(Feed::Npm("@anthropic-ai/claude-code"))
        );
        assert_eq!(feed(HarnessKind::Grok), Some(Feed::Grok));
        assert_eq!(feed(HarnessKind::Antigravity), None);
        assert!(is_updatable(HarnessKind::OpenCode));
        assert!(is_updatable(HarnessKind::Grok));
        assert!(!is_updatable(HarnessKind::Antigravity));
    }

    #[test]
    fn other_clis_update_where_startup_found_them() {
        let listed = Path::new("/opt/homebrew/bin/claude");
        assert_eq!(
            updatable(HarnessKind::Claude, listed).as_deref(),
            Some(listed)
        );
    }

    #[test]
    fn grok_follows_the_channel_its_installer_wrote() {
        assert_eq!(grok_channel(""), "stable");
        assert_eq!(
            grok_channel(
                "[cli]\ninstaller = \"internal\"\nchannel = \"alpha\"\n\n[ui]\nyolo = false\n"
            ),
            "alpha"
        );
        // Another table's key of the same name is not the CLI's.
        assert_eq!(
            grok_channel("[marketplace]\nchannel = \"alpha\"\n"),
            "stable"
        );
        assert_eq!(grok_channel("[cli]\nchannel = \"nightly\"\n"), "stable");
    }

    #[test]
    fn output_held_by_a_daemon_does_not_hang_the_run() {
        let started = Instant::now();
        let (ok, stdout, _) = run(
            Path::new("/bin/sh"),
            &["-c", "echo done; sleep 30 &"],
            Duration::from_secs(20),
        )
        .unwrap();
        assert!(ok);
        assert!(started.elapsed() < Duration::from_secs(10));
        // What it wrote before the daemon is still read when the pipe
        // closes on time; with the daemon holding it, it may be cut.
        assert!(stdout.is_empty() || stdout == b"done\n");
    }

    #[test]
    fn a_run_stops_when_told_to() {
        let started = Instant::now();
        let mut asked = 0;
        let finished = run_while(
            Path::new("/bin/sh"),
            &["-c", "sleep 30"],
            UPDATE_TIMEOUT,
            || {
                asked += 1;
                asked < 3
            },
        )
        .unwrap();
        assert!(finished.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn updates_only_through_each_cli_own_updater() {
        assert_eq!(update_args(HarnessKind::OpenCode), Some(&["upgrade"][..]));
        assert_eq!(update_args(HarnessKind::Claude), Some(&["update"][..]));
        assert_eq!(update_args(HarnessKind::Grok), Some(&["update"][..]));
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
