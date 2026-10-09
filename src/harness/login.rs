//! A provider's own browser sign-in (MonoCode
//! `integrations/harness/core/auth.ts`): `claude auth login` or
//! `codex login` runs as a child that opens the browser and stores the
//! credential itself; BenCode only waits for it to exit. Blocking, for up
//! to ten minutes: run it on a background executor.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::harness::HarnessKind;
use crate::harness::accounts::AccountProfile;
use crate::harness::resolver::HarnessResolver;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_DETAIL_CHARS: usize = 240;

/// Account-level login commands that can run without an interactive
/// provider picker.
fn login_args(harness: HarnessKind) -> Option<&'static [&'static str]> {
    match harness {
        HarnessKind::Claude => Some(&["auth", "login"]),
        HarnessKind::Codex => Some(&["login"]),
        HarnessKind::Antigravity | HarnessKind::OpenCode => None,
    }
}

/// Signs `harness` in, into `account`'s profile when given.
pub fn login(harness: HarnessKind, account: Option<&AccountProfile>) -> Result<(), String> {
    let title = harness.label();
    let (Some(args), Some(program)) = (login_args(harness), resolve(harness)) else {
        return Err(format!(
            "{title} does not offer a single browser sign-in flow."
        ));
    };
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(account) = account {
        account.apply(&mut command);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("Could not start {title} sign-in: {err}"))?;
    // Drained on its own thread so a chatty CLI cannot block on a full pipe.
    let stderr = child.stderr.take();
    let last_line = std::thread::spawn(move || {
        let mut last = String::new();
        for line in stderr
            .into_iter()
            .flat_map(|stderr| BufReader::new(stderr).lines())
        {
            match line {
                Ok(line) if !line.trim().is_empty() => last = line.trim().to_string(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        last
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < LOGIN_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                if let Err(err) = child.kill() {
                    log::debug!("{title} sign-in already exited: {err}");
                }
                if let Err(err) = child.wait() {
                    log::debug!("{title} sign-in wait: {err}");
                }
                return Err(format!("{title} sign-in timed out. Please try again."));
            }
            Err(err) => return Err(format!("Could not start {title} sign-in: {err}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let detail = safe_detail(&last_line.join().unwrap_or_default());
    Err(if !detail.is_empty() {
        detail
    } else {
        match status.code() {
            Some(code) => format!("{title} sign-in exited with code {code}."),
            None => format!("{title} sign-in exited unexpectedly."),
        }
    })
}

fn resolve(harness: HarnessKind) -> Option<PathBuf> {
    match harness {
        HarnessKind::Claude => HarnessResolver::resolve_claude(),
        HarnessKind::Codex => HarnessResolver::resolve_codex(),
        HarnessKind::Antigravity | HarnessKind::OpenCode => None,
    }
}

/// The CLI's last error line without its sign-in URL (which carries a
/// one-time code), shortened for the popover.
fn safe_detail(value: &str) -> String {
    let text = value
        .split_whitespace()
        .map(|word| {
            if word.starts_with("http://") || word.starts_with("https://") {
                "sign-in link"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() > MAX_DETAIL_CHARS {
        let head: String = text.chars().take(MAX_DETAIL_CHARS - 3).collect();
        format!("{head}…")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claude_and_codex_sign_in_from_one_command() {
        assert_eq!(
            login_args(HarnessKind::Claude),
            Some(&["auth", "login"][..])
        );
        assert_eq!(login_args(HarnessKind::Codex), Some(&["login"][..]));
        assert!(login_args(HarnessKind::OpenCode).is_none());
        let refused = login(HarnessKind::OpenCode, None).unwrap_err();
        assert_eq!(
            refused,
            "OpenCode does not offer a single browser sign-in flow."
        );
    }

    #[test]
    fn error_detail_hides_links_and_is_bounded() {
        assert_eq!(
            safe_detail("Error:  open https://claude.ai/oauth?code=abc   to continue"),
            "Error: open sign-in link to continue"
        );
        let long = safe_detail(&"word ".repeat(100));
        assert_eq!(long.chars().count(), 238);
        assert!(long.ends_with('…'));
    }
}
