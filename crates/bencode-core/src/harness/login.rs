//! A provider's own browser sign-in (MonoCode
//! `integrations/harness/core/auth.ts`): `claude auth login` or
//! `codex login` runs as a child that opens the browser and stores the
//! credential itself; BenCode waits for it to exit. The link it prints is
//! handed back as a `LoginPrompt`, so a browser that did not open, or one on
//! another device, can still finish: Claude then takes the code that page
//! shows on stdin. Blocking, for up to ten minutes: run it on a background
//! executor.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
        HarnessKind::Antigravity | HarnessKind::Grok | HarnessKind::OpenCode => None,
    }
}

/// What `login` returns when `LoginCancel::cancel` stopped it.
pub const CANCELLED: &str = "Sign-in cancelled.";

/// The sign-in link the CLI printed, and its stdin for the code.
#[derive(Clone)]
pub struct LoginPrompt {
    pub url: String,
    /// The CLI reads a pasted code (Claude); Codex only takes its callback.
    pub accepts_code: bool,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl LoginPrompt {
    /// Hands the CLI the code its sign-in page showed.
    pub fn submit_code(&self, code: &str) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| "The sign-in has already ended.".to_string())?;
        writeln!(stdin, "{}", code.trim())
            .and_then(|()| stdin.flush())
            .map_err(|err| format!("Could not send the code: {err}"))
    }
}

/// Stops a `login` that is still waiting.
#[derive(Clone, Default)]
pub struct LoginCancel(Arc<AtomicBool>);

impl LoginCancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Signs `harness` in, into `account`'s profile when given. `on_prompt`
/// gets the sign-in link once the CLI prints one.
pub fn login(
    harness: HarnessKind,
    account: Option<&AccountProfile>,
    cancel: &LoginCancel,
    on_prompt: impl FnOnce(LoginPrompt) + Send + 'static,
) -> Result<(), String> {
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
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(account) = account {
        account.apply(&mut command);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("Could not start {title} sign-in: {err}"))?;
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let prompt_stdin = stdin.clone();
    let accepts_code = harness == HarnessKind::Claude;
    // Both pipes are drained on their own threads so a chatty CLI cannot
    // block on a full pipe.
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut on_prompt = Some(on_prompt);
        for line in stdout
            .into_iter()
            .flat_map(|stdout| BufReader::new(stdout).lines())
        {
            let Ok(line) = line else { break };
            if let Some(url) = sign_in_url(&line)
                && let Some(on_prompt) = on_prompt.take()
            {
                on_prompt(LoginPrompt {
                    url,
                    accepts_code,
                    stdin: prompt_stdin.clone(),
                });
            }
        }
    });
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
            Ok(None) if started.elapsed() < LOGIN_TIMEOUT && !cancel.cancelled() => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                if let Err(err) = child.kill() {
                    log::debug!("{title} sign-in already exited: {err}");
                }
                if let Err(err) = child.wait() {
                    log::debug!("{title} sign-in wait: {err}");
                }
                stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
                return Err(if cancel.cancelled() {
                    CANCELLED.to_string()
                } else {
                    format!("{title} sign-in timed out. Please try again.")
                });
            }
            Err(err) => return Err(format!("Could not start {title} sign-in: {err}")),
        }
    };
    // A code sent from here on would go nowhere.
    stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
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
        HarnessKind::Antigravity | HarnessKind::Grok | HarnessKind::OpenCode => None,
    }
}

/// The link in a line such as Claude's "If the browser didn't open, visit:
/// https://…" or Codex's "navigate to this URL to authenticate: https://…".
fn sign_in_url(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|word| word.starts_with("https://") || word.starts_with("http://"))
        .map(str::to_string)
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
        let refused =
            login(HarnessKind::OpenCode, None, &LoginCancel::default(), |_| {}).unwrap_err();
        assert_eq!(
            refused,
            "OpenCode does not offer a single browser sign-in flow."
        );
    }

    #[test]
    fn finds_the_link_the_cli_prints() {
        assert_eq!(
            sign_in_url(
                "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&state=x"
            ),
            Some("https://claude.com/cai/oauth/authorize?code=true&state=x".to_string())
        );
        assert_eq!(sign_in_url("Opening browser to sign in…"), None);
        assert_eq!(sign_in_url("Paste code here if prompted > "), None);
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
