//! MonoCode `gitText.ts` and `claudeGit.ts`: commit messages and pull
//! request text written by Claude (Haiku, as MonoCode's text model) from
//! the staged diff or the branch's range. One `claude -p` per request.
//!
//! Runs a process and blocks; use the background executor.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::sync::{range_context, staged_context};

/// MonoCode `TEXT_MODEL` and `GIT_TIMEOUT_MS`.
const TEXT_MODEL: &str = "claude-haiku-4-5";
const TIMEOUT: Duration = Duration::from_secs(90);

/// MonoCode `limitSection`.
fn limit(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let cut: String = value.chars().take(max).collect();
    format!("{cut}\n\n[truncated]")
}

/// MonoCode `buildCommitMessagePrompt`.
pub fn commit_prompt(branch: Option<&str>, summary: &str, patch: &str) -> String {
    [
        "You write concise git commit messages.".to_string(),
        "Return a JSON object with keys: subject, body.".into(),
        "Do not call tools. Reply with JSON only.".into(),
        "Rules:".into(),
        "- subject must be imperative, <= 72 chars, and no trailing period".into(),
        "- body can be empty string or short bullet points".into(),
        "- capture the primary user-visible or developer-visible change".into(),
        String::new(),
        format!("Branch: {}", branch.unwrap_or("(detached)")),
        String::new(),
        "Staged files:".into(),
        limit(summary, 6_000),
        String::new(),
        "Staged patch:".into(),
        limit(patch, 40_000),
    ]
    .join("\n")
}

/// MonoCode `buildPrContentPrompt`.
pub fn pr_prompt(base: &str, head: &str, commits: &str, stat: &str, patch: &str) -> String {
    [
        "You write source control change request content.".to_string(),
        "Return a JSON object with keys: title, body.".into(),
        "Do not call tools. Reply with JSON only.".into(),
        "Rules:".into(),
        "- title should be concise and specific".into(),
        "- body must be markdown and include headings '## Summary' and '## Testing'".into(),
        "- under Summary, provide short bullet points".into(),
        "- under Testing, include bullet points with concrete checks or 'Not run' where appropriate"
            .into(),
        String::new(),
        format!("Base branch: {base}"),
        format!("Head branch: {head}"),
        String::new(),
        "Commits:".into(),
        limit(commits, 12_000),
        String::new(),
        "Diff stat:".into(),
        limit(stat, 12_000),
        String::new(),
        "Diff patch:".into(),
        limit(patch, 40_000),
    ]
    .join("\n")
}

/// MonoCode `extractJsonObject`: the balanced `{…}` starting at the first
/// brace.
fn extract_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let (mut depth, mut in_string, mut escaping) = (0usize, false, false);
    for (i, c) in raw[start..].char_indices() {
        if in_string {
            if escaping {
                escaping = false;
            } else if c == '\\' {
                escaping = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// MonoCode `parseJsonObject`: the first object with a text key, skipping
/// braces in prose before it.
pub fn parse_object(raw: &str) -> Option<Map<String, Value>> {
    const KEYS: [&str; 5] = ["subject", "title", "message", "body", "branch"];
    let raw = raw.trim();
    let mut from = 0;
    let mut fallback = None;
    while let Some(offset) = raw[from..].find('{') {
        let start = from + offset;
        let Some(json) = extract_object(&raw[start..]) else {
            break;
        };
        from = start + 1;
        if let Ok(Value::Object(rec)) = serde_json::from_str::<Value>(json) {
            if KEYS.iter().any(|k| rec.get(*k).is_some_and(Value::is_string)) {
                return Some(rec);
            }
            fallback.get_or_insert(rec);
        }
    }
    fallback
}

fn field<'a>(rec: &'a Map<String, Value>, key: &str) -> &'a str {
    rec.get(key).and_then(Value::as_str).unwrap_or("")
}

/// MonoCode `sanitizeCommitSubject`.
fn sanitize_subject(raw: &str) -> String {
    let line = raw.trim().lines().next().unwrap_or("").trim();
    let line = line.trim_end_matches('.').trim();
    line.chars().take(72).collect::<String>().trim_end().to_string()
}

/// MonoCode `parseCommitMessage` + `formatCommitMessage`.
pub fn parse_commit_message(raw: &str) -> Option<String> {
    let rec = parse_object(raw)?;
    let subject = sanitize_subject(
        [field(&rec, "subject"), field(&rec, "title"), field(&rec, "message")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or(""),
    );
    if subject.is_empty() {
        return None;
    }
    let body = match rec.get("body") {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        _ => String::new(),
    };
    Some(if body.is_empty() {
        subject
    } else {
        format!("{subject}\n\n{body}")
    })
}

/// MonoCode `parsePrContent`: (title, body).
pub fn parse_pr_content(raw: &str) -> Option<(String, String)> {
    let rec = parse_object(raw)?;
    let title = field(&rec, "title").trim().lines().next()?.trim().to_string();
    (!title.is_empty()).then(|| (title, field(&rec, "body").trim().to_string()))
}

/// Runs `claude -p` in `cwd` until it answers, `cancel` is set, or the
/// timeout passes.
fn run_claude(cwd: &str, prompt: &str, cancel: &AtomicBool) -> Result<String, String> {
    let claude = crate::harness::resolver::HarnessResolver::resolve_claude()
        .ok_or_else(|| "Claude Code CLI not found. Install it to generate text.".to_string())?;
    let mut child = Command::new(claude)
        .current_dir(cwd)
        .args([
            "-p",
            prompt,
            "--model",
            TEXT_MODEL,
            "--output-format",
            "text",
            "--max-turns",
            "1",
            "--disallowedTools",
            "Bash,Edit,Write,Read,Glob,Grep,WebFetch,WebSearch",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Could not start Claude Code: {err}"))?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill().map_err(|err| err.to_string())?;
            let _ = child.wait();
            return Err("Cancelled".into());
        }
        if started.elapsed() > TIMEOUT {
            child.kill().map_err(|err| err.to_string())?;
            let _ = child.wait();
            return Err("Claude Code took too long to answer.".into());
        }
        match child.try_wait().map_err(|err| err.to_string())? {
            Some(status) => {
                let mut out = String::new();
                let mut err = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    stdout.read_to_string(&mut out).map_err(|e| e.to_string())?;
                }
                if let Some(mut stderr) = child.stderr.take() {
                    stderr.read_to_string(&mut err).map_err(|e| e.to_string())?;
                }
                if !status.success() && out.trim().is_empty() {
                    return Err(if err.trim().is_empty() {
                        "Claude Code returned no text.".into()
                    } else {
                        err.trim().to_string()
                    });
                }
                return Ok(out);
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// MonoCode `generateCommitMessage`.
pub fn generate_commit_message(cwd: &str, cancel: Arc<AtomicBool>) -> Result<String, String> {
    let context = staged_context(cwd)?;
    let prompt = commit_prompt(context.branch.as_deref(), &context.summary, &context.patch);
    let output = run_claude(cwd, &prompt, &cancel)?;
    parse_commit_message(&output).ok_or_else(|| {
        let snippet: String = output
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(240)
            .collect();
        if snippet.is_empty() {
            "Could not generate a commit message. Claude Code returned no text.".into()
        } else {
            format!("Could not generate a commit message. Model replied: {snippet}")
        }
    })
}

/// MonoCode `generatePrContent`: title, body, base and head; the commits
/// stand in when Claude cannot write them.
pub struct PrContent {
    pub title: String,
    pub body: String,
    pub base: String,
    pub head: String,
}

pub fn generate_pr_content(cwd: &str) -> Result<PrContent, String> {
    let range = range_context(cwd)?;
    let prompt = pr_prompt(
        &range.base,
        &range.head,
        &range.commit_summary,
        &range.diff_summary,
        &range.diff_patch,
    );
    let parsed = match run_claude(cwd, &prompt, &AtomicBool::new(false)) {
        Ok(output) => parse_pr_content(&output),
        Err(err) => {
            log::debug!("pr content: {err}");
            None
        }
    };
    let first_commit = range.commit_summary.lines().next().unwrap_or("").trim();
    let title = parsed
        .as_ref()
        .map(|(t, _)| t.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| (!first_commit.is_empty()).then(|| first_commit.to_string()))
        .unwrap_or_else(|| format!("Update {}", range.head));
    let body = parsed
        .map(|(_, b)| b)
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| range.commit_summary.trim().to_string());
    Ok(PrContent {
        title,
        body,
        base: range.base,
        head: range.head,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_messages_parse_from_chatty_replies() {
        let raw = "Sure {not json} here:\n```json\n{\"subject\": \"Add graph view.\", \"body\": [\"- lanes\", \"- refs\"]}\n```";
        assert_eq!(
            parse_commit_message(raw).as_deref(),
            Some("Add graph view\n\n- lanes\n- refs")
        );
        assert_eq!(
            parse_commit_message(r#"{"title":"Fix it","body":""}"#).as_deref(),
            Some("Fix it")
        );
        assert!(parse_commit_message("no json").is_none());
        let long = format!(r#"{{"subject":"{}"}}"#, "x".repeat(100));
        assert_eq!(parse_commit_message(&long).unwrap().len(), 72);
    }

    #[test]
    fn pr_content_needs_a_title() {
        assert_eq!(
            parse_pr_content(r###"{"title":"Add X","body":"## Summary"}"###),
            Some(("Add X".into(), "## Summary".into()))
        );
        assert!(parse_pr_content(r#"{"body":"x"}"#).is_none());
    }

    #[test]
    fn prompts_cap_their_sections() {
        let prompt = commit_prompt(None, "a.rs | 2", &"y".repeat(50_000));
        assert!(prompt.contains("Branch: (detached)"));
        assert!(prompt.ends_with("[truncated]"));
    }
}
