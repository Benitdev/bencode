//! The in-app browser (BenCode's own, after Codex desktop's; MonoCode has
//! none): a WKWebView, through wry, laid over a file-pane tab. Agents reach
//! it through `bencode --browser-mcp`, an MCP server over stdio that relays
//! each tool call to the app over a Unix socket (`bridge.rs`).
//!
//! The page is a native view: it draws above everything GPUI paints and
//! GPUI's clipping does not reach it, so the app hides it while a dialog is
//! open or its tab is not drawn (`app/browser.rs`).

pub mod bridge;
#[cfg(target_os = "macos")]
mod container;
pub mod mcp;
pub mod page;
pub mod scripts;
#[cfg(target_os = "macos")]
mod snapshot;

const MCP_ARG: &str = "--browser-mcp";

/// The MCP server's name, as the agents list its tools (`mcp__bencode-browser__…`).
pub const MCP_SERVER_NAME: &str = "bencode-browser";

/// Runs the MCP server when `main` was started as one; returns the exit
/// code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(MCP_ARG) {
        return None;
    }
    let socket = args.next()?;
    Some(mcp::run(std::path::Path::new(&socket)))
}

/// How an agent CLI starts the browser's MCP server for this app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
}

impl McpLaunch {
    pub fn new(socket: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        let exe = std::env::current_exe().context("cannot find BenCode's executable")?;
        Ok(Self {
            command: exe.to_string_lossy().into_owned(),
            args: vec![MCP_ARG.to_string(), socket.to_string_lossy().into_owned()],
        })
    }
}

/// What the address bar's text means: an address as typed, a bare host
/// (`localhost:3000`, `example.com`) or a search.
pub fn normalize_url(input: &str) -> String {
    let text = input.trim();
    if text.is_empty() {
        return "about:blank".into();
    }
    let has_scheme = text.contains("://")
        || ["about:", "data:", "file:", "javascript:"]
            .iter()
            .any(|scheme| text.starts_with(scheme));
    if has_scheme {
        return text.to_string();
    }
    if text.starts_with('/') {
        return format!("file://{text}");
    }
    let host = host_of(text);
    if is_local_host(host) || host.ends_with(".local") {
        return format!("http://{text}");
    }
    let looks_like_host = !text.contains(char::is_whitespace) && host.contains('.');
    if looks_like_host {
        return format!("https://{text}");
    }
    format!("https://www.google.com/search?q={}", encode_query(text))
}

/// Percent-encodes a search for a query string (spaces as `+`).
fn encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Whether a link should open in the in-app browser rather than the
/// default one: a dev server on this machine.
pub fn is_local_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    is_local_host(host_of(rest))
}

/// A site's address: what a page may open a window at.
pub fn is_web_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// What an agent's `browser_navigate` may open: a site, or a file (it can
/// read those anyway). Not `javascript:` or `data:`.
pub fn agent_may_open(url: &str) -> bool {
    is_web_url(url) || url.starts_with("file://") || url == "about:blank"
}

/// The host of an address without its scheme: `a.dev:3000/x` is `a.dev`.
fn host_of(rest: &str) -> &str {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    }
}

fn is_local_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]") || host.ends_with(".localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_keep_their_scheme() {
        assert_eq!(normalize_url(" https://a.dev/x "), "https://a.dev/x");
        assert_eq!(normalize_url("about:blank"), "about:blank");
        assert_eq!(normalize_url(""), "about:blank");
    }

    #[test]
    fn local_hosts_get_http_and_others_https() {
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(
            normalize_url("127.0.0.1:8080/api?x=1"),
            "http://127.0.0.1:8080/api?x=1"
        );
        assert_eq!(normalize_url("app.localhost"), "http://app.localhost");
        assert_eq!(
            normalize_url("example.com/docs"),
            "https://example.com/docs"
        );
        assert_eq!(normalize_url("/tmp/a.html"), "file:///tmp/a.html");
    }

    #[test]
    fn anything_else_is_a_search() {
        assert_eq!(
            normalize_url("rust gpui & wry"),
            "https://www.google.com/search?q=rust+gpui+%26+wry"
        );
        assert_eq!(
            normalize_url("localhost"),
            "http://localhost",
            "a bare localhost is a host"
        );
    }

    #[test]
    fn agents_open_sites_and_files_only() {
        assert!(agent_may_open(&normalize_url("localhost:3000")));
        assert!(agent_may_open(&normalize_url("/tmp/a.html")));
        assert!(agent_may_open("https://example.com"));
        assert!(!agent_may_open("javascript:alert(1)"));
        assert!(!agent_may_open("data:text/html,<p>x"));
        assert!(!is_web_url("file:///tmp/a.html"));
    }

    #[test]
    fn local_urls_are_dev_servers() {
        assert!(is_local_url("http://localhost:5173/"));
        assert!(is_local_url("http://127.0.0.1:8000"));
        assert!(is_local_url("https://app.localhost/x"));
        assert!(!is_local_url("https://example.com"));
        assert!(!is_local_url("file:///tmp/a.html"));
        assert!(!is_local_url("http://localhost.evil.com"));
    }
}
