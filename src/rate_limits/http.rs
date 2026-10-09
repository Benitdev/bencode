//! One authenticated request for the usage endpoints and the trackers'
//! REST APIs, through the system `curl` (MonoCode uses `ureq`; BenCode
//! keeps no HTTP client of its own). The request, token included, travels
//! on curl's stdin, never on argv.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

pub struct Response {
    pub status: u16,
    pub body: String,
}

/// What to send besides a plain GET.
#[derive(Clone, Copy, Default)]
pub struct Send<'a> {
    /// `None` is GET (or POST once there is a form).
    pub method: Option<&'a str>,
    /// Sent URL-encoded as the body.
    pub form: &'a [(&'a str, &'a str)],
    /// Sent as written (JSON); the caller names its content type.
    pub body: Option<&'a str>,
    /// Follow redirects (GitHub's release download links redirect).
    pub follow_redirects: bool,
}

/// Blocking; `timeout` bounds the whole transfer.
pub fn get(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Response> {
    send(url, headers, Send::default(), timeout)
}

/// Blocking; `timeout` bounds the whole transfer.
pub fn send(
    url: &str,
    headers: &[(&str, &str)],
    send: Send,
    timeout: Duration,
) -> Result<Response> {
    let mut child = Command::new("curl")
        // -q: no ~/.curlrc, so nothing there can reshape the output.
        .args(["-q", "--silent", "--show-error", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start curl")?;
    let config = request_config(url, headers, send, timeout);
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

fn request_config(url: &str, headers: &[(&str, &str)], send: Send, timeout: Duration) -> String {
    // globoff: `[` and `{` in a URL are sent as written.
    let mut config = format!(
        "globoff\nurl = {}\nmax-time = {}\nwrite-out = \"\\n%{{http_code}}\"\n",
        quoted(url),
        timeout.as_secs().max(1)
    );
    for (name, value) in headers {
        config.push_str(&format!(
            "header = {}\n",
            quoted(&format!("{name}: {value}"))
        ));
    }
    if send.follow_redirects {
        config.push_str("location\n");
    }
    if let Some(method) = send.method {
        config.push_str(&format!("request = {}\n", quoted(method)));
    }
    for (name, value) in send.form {
        // `name=content`: curl encodes the content, not the name.
        config.push_str(&format!(
            "data-urlencode = {}\n",
            quoted_data(&format!("{name}={value}"))
        ));
    }
    if let Some(body) = send.body {
        // data-raw: a leading `@` is not a file name.
        config.push_str(&format!("data-raw = {}\n", quoted_data(body)));
    }
    config
}

/// A form value in a curl config: like [`quoted`], but line breaks and
/// tabs are kept as curl's escapes rather than dropped.
fn quoted_data(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
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
            Send::default(),
            Duration::from_secs(10),
        );
        assert_eq!(
            config,
            "globoff\nurl = \"https://example.com/usage\"\nmax-time = 10\nwrite-out = \"\\n%{http_code}\"\n\
             header = \"Authorization: Bearer a\\\"b\\\\cd\"\n"
        );
    }

    #[test]
    fn forms_are_url_encoded_with_their_line_breaks() {
        let config = request_config(
            "https://example.com/c",
            &[],
            Send {
                method: Some("PATCH"),
                form: &[("content", "a \"b\"\nc")],
                body: None,
                follow_redirects: false,
            },
            Duration::from_secs(5),
        );
        assert!(config.contains("request = \"PATCH\"\n"));
        assert!(config.ends_with("data-urlencode = \"content=a \\\"b\\\"\\nc\"\n"));
    }

    #[test]
    fn redirects_are_followed_when_asked() {
        let follow = Send {
            follow_redirects: true,
            ..Default::default()
        };
        let config = request_config("https://example.com/f", &[], follow, Duration::from_secs(5));
        assert!(config.contains("\nlocation\n"));
        let plain = request_config(
            "https://example.com/f",
            &[],
            Send::default(),
            Duration::from_secs(5),
        );
        assert!(!plain.contains("location"));
    }

    #[test]
    fn a_json_body_is_sent_as_written() {
        let send = Send {
            body: Some("{\"project\":\"p\"}"),
            ..Default::default()
        };
        let config = request_config("https://example.com/q", &[], send, Duration::from_secs(5));
        assert!(config.ends_with("data-raw = \"{\\\"project\\\":\\\"p\\\"}\"\n"));
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
