//! One authenticated GET for the usage endpoints, through the system `curl`
//! (MonoCode uses `ureq`; BenCode keeps no HTTP client of its own). The
//! request, token included, travels on curl's stdin, never on argv.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

pub struct Response {
    pub status: u16,
    pub body: String,
}

/// Blocking; `timeout` bounds the whole transfer.
pub fn get(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Response> {
    let mut child = Command::new("curl")
        // -q: no ~/.curlrc, so nothing there can reshape the output.
        .args(["-q", "--silent", "--show-error", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start curl")?;
    let config = request_config(url, headers, timeout);
    child
        .stdin
        .take()
        .context("curl stdin unavailable")?
        .write_all(config.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        bail!("{}", detail.trim().trim_start_matches("curl: "));
    }
    split_status(&String::from_utf8_lossy(&output.stdout)).context("curl reported no HTTP status")
}

fn request_config(url: &str, headers: &[(&str, &str)], timeout: Duration) -> String {
    let mut config = format!(
        "url = {}\nmax-time = {}\nwrite-out = \"\\n%{{http_code}}\"\n",
        quoted(url),
        timeout.as_secs().max(1)
    );
    for (name, value) in headers {
        config.push_str(&format!("header = {}\n", quoted(&format!("{name}: {value}"))));
    }
    config
}

/// A curl config value: double-quoted, with `\` and `"` escaped. Control
/// characters cannot appear in a URL or header, so they are dropped.
fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars().filter(|ch| !ch.is_control()) {
        if matches!(ch, '\\' | '"') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// The body and the status code `write-out` appended on its own last line.
fn split_status(stdout: &str) -> Option<Response> {
    let (body, status) = stdout.rsplit_once('\n')?;
    Some(Response {
        status: status.trim().parse().ok().filter(|code| *code > 0)?,
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_quotes_values_and_keeps_secrets_off_argv() {
        let config = request_config(
            "https://example.com/usage",
            &[("Authorization", "Bearer a\"b\\c\nd")],
            Duration::from_secs(10),
        );
        assert_eq!(
            config,
            "url = \"https://example.com/usage\"\nmax-time = 10\nwrite-out = \"\\n%{http_code}\"\n\
             header = \"Authorization: Bearer a\\\"b\\\\cd\"\n"
        );
    }

    #[test]
    fn status_is_the_last_line() {
        let ok = split_status("{\"a\":1}\n200").unwrap();
        assert_eq!((ok.status, ok.body.as_str()), (200, "{\"a\":1}"));
        let empty = split_status("\n401").unwrap();
        assert_eq!((empty.status, empty.body.as_str()), (401, ""));
        let multiline = split_status("line one\nline two\n500").unwrap();
        assert_eq!(multiline.body, "line one\nline two");
        assert!(split_status("no status").is_none());
        assert!(split_status("body\n000").is_none());
    }
}
