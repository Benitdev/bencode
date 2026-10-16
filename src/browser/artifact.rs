//! An `html` artifact's page (MonoCode `HtmlArtifactFrame`, and
//! `artifacts.rs`'s `serve_html`): a WKWebView in a box of its own, like a
//! browser tab's (`container.rs`), that loads the page from BenCode's own
//! scheme. The response's CSP sandboxes it the way MonoCode's frame is: it
//! may run inline scripts and load public styles, scripts, images and
//! fonts, but cannot fetch, frame, submit forms or go anywhere else. It is
//! transparent, so it sits on the panel's own background.
//!
//! The page gets the app's theme as CSS variables (`--background`,
//! `--foreground`, `--accent`, `--chart-1`…), which the app hands over with
//! the page (MonoCode posts them to its frame), and a bootstrap that hands
//! web links to the app. What a page posts is not trusted: a link opens only
//! while the page has the keyboard, that is after the user pressed in it.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use gpui::{Bounds, Pixels};
use tokio::sync::mpsc::UnboundedSender;
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, WebViewBuilder};

/// The scheme the page is served from.
const SCHEME: &str = "bencodeartifact";

/// MonoCode `HTML_CSP`.
const CSP: &str = "sandbox allow-scripts; default-src 'none'; \
    script-src 'unsafe-inline' https:; style-src 'unsafe-inline' https:; \
    img-src data: blob: https:; font-src data: https:; media-src data: blob: https:; \
    worker-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";

/// MonoCode `HTML_BOOTSTRAP`, less what its frame needed: BenCode hands the
/// theme over with the page, and the view scrolls itself, so no height.
const BOOTSTRAP: &str = r#"<script>(function(){document.addEventListener("click",function(e){var a=e.target&&e.target.closest?e.target.closest("a[href]"):null,u;if(!a)return;try{u=new URL(a.getAttribute("href"),document.baseURI);}catch(x){return;}if(!/^https?:$/.test(u.protocol))return;e.preventDefault();if(e.isTrusted&&window.ipc)window.ipc.postMessage(JSON.stringify({type:"open-link",url:u.href}));},true);})();</script>"#;

/// What the page asks of the app.
#[derive(Debug)]
pub enum ArtifactEvent {
    /// A web link the user clicked, for their browser.
    OpenLink(String),
}

pub struct ArtifactPage {
    view: wry::WebView,
    #[cfg(target_os = "macos")]
    container: super::container::Container,
    /// What the scheme serves: the page shown now.
    served: Arc<Mutex<String>>,
    /// What the page was built from (artifact id, its save, the theme),
    /// so a redraw loads nothing again.
    shown: std::cell::RefCell<Option<(String, i64, u64)>>,
    placed: Cell<Option<(Bounds<Pixels>, bool)>>,
}

impl ArtifactPage {
    /// A hidden, empty view; `show` gives it a page and the reader places it.
    pub fn build(window: &gpui::Window, events: UnboundedSender<ArtifactEvent>) -> Result<Self> {
        let served = Arc::new(Mutex::new(String::new()));
        let page = served.clone();
        #[cfg(target_os = "macos")]
        let container = super::container::Container::new(window)?;
        #[cfg(target_os = "macos")]
        let parent = &container;
        #[cfg(not(target_os = "macos"))]
        let parent = window;
        let view = WebViewBuilder::new()
            .with_visible(cfg!(target_os = "macos"))
            .with_transparent(true)
            .with_custom_protocol(SCHEME.into(), move |_, request| {
                // Only the page shown, only fetched.
                let body = if request.method() == wry::http::Method::GET {
                    page.lock().map(|page| page.clone()).unwrap_or_default()
                } else {
                    String::new()
                };
                let status = if body.is_empty() {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::OK
                };
                Response::builder()
                    .status(status)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Content-Security-Policy", CSP)
                    .header("Cache-Control", "no-store")
                    .header("X-Content-Type-Options", "nosniff")
                    .body(body.into_bytes().into())
                    .unwrap_or_else(|_| Response::new(Vec::new().into()))
            })
            // The page stays where it is: a link goes to the user's browser
            // through the bootstrap, and nothing else may leave.
            .with_navigation_handler(|url| url.starts_with(&format!("{SCHEME}://")))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_ipc_handler(move |request| {
                let Ok(message) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                if message.get("type").and_then(|v| v.as_str()) == Some("open-link")
                    && let Some(url) = message.get("url").and_then(|v| v.as_str())
                    && crate::browser::is_web_url(url)
                    && events.send(ArtifactEvent::OpenLink(url.to_string())).is_err()
                {
                    log::debug!("artifact page: link after the app left");
                }
            })
            .build_as_child(parent)
            .context("the artifact's view could not be created")?;
        #[cfg(target_os = "macos")]
        container.fill(&view);
        Ok(Self {
            view,
            #[cfg(target_os = "macos")]
            container,
            served,
            shown: std::cell::RefCell::new(None),
            placed: Cell::new(None),
        })
    }

    /// Loads artifact `id`'s page as saved at `version`, with the theme
    /// `theme_key` names, unless that is what it shows.
    pub fn show(&self, id: &str, version: i64, theme_key: u64, page: impl FnOnce() -> String) {
        let key = (id.to_string(), version, theme_key);
        if self.shown.borrow().as_ref() == Some(&key) {
            return;
        }
        match self.served.lock() {
            Ok(mut served) => *served = page(),
            Err(err) => {
                log::error!("artifact page: {err}");
                return;
            }
        }
        // A new address each time, so WebKit loads it rather than its cache.
        let url = format!("{SCHEME}://localhost/{id}?v={version}-{theme_key}");
        if let Err(err) = self.view.load_url(&url) {
            log::warn!("artifact page: could not load {id}: {err}");
            return;
        }
        *self.shown.borrow_mut() = Some(key);
    }

    /// Puts the view over `bounds` (window coordinates), or hides it.
    pub fn place(&self, bounds: Bounds<Pixels>, visible: bool) {
        if self.placed.get() == Some((bounds, visible)) {
            return;
        }
        let shown = self.is_shown();
        self.placed.set(Some((bounds, visible)));
        #[cfg(target_os = "macos")]
        {
            if visible {
                self.container.set_frame(bounds);
            }
            if visible != shown {
                self.container.set_hidden(!visible);
                if !visible {
                    self.release_keys();
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (bounds, shown);
            if let Err(err) = self.view.set_visible(visible) {
                log::warn!("artifact page: could not show or hide: {err}");
            }
        }
    }

    /// Hides the view (the reader is not drawn this frame).
    pub fn hide(&self) {
        if let Some((bounds, true)) = self.placed.get() {
            self.place(bounds, false);
        }
    }

    pub fn is_shown(&self) -> bool {
        self.placed.get().is_some_and(|(_, shown)| shown)
    }

    /// Whether the page has the keyboard (the user pressed in it last).
    pub fn has_keys(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.container.has_keys();
        #[cfg(not(target_os = "macos"))]
        false
    }

    /// Gives the keyboard back to the window (a press anywhere GPUI draws).
    pub fn release_keys(&self) {
        #[cfg(target_os = "macos")]
        self.container.release_keys();
    }
}

impl Drop for ArtifactPage {
    fn drop(&mut self) {
        self.release_keys();
    }
}

/// The page served for an artifact's `body`: MonoCode `with_bootstrap`,
/// with the theme's variables (`theme_css`, a `:root` rule) ahead of the
/// bootstrap.
pub fn page_html(body: &str, theme_css: &str) -> String {
    let head = format!(
        "<style>{theme_css}html{{color:var(--foreground);font-family:var(--font-sans)}}body{{margin:0}}</style>{BOOTSTRAP}"
    );
    // ASCII lowercasing keeps byte offsets, so they index into `body`.
    let lower = body.to_ascii_lowercase();
    let at = tag_end(&lower, "<head")
        .or_else(|| tag_end(&lower, "<html"))
        .or_else(|| {
            let start = lower.len() - lower.trim_start().len();
            lower[start..]
                .starts_with("<!doctype")
                .then(|| lower[start..].find('>').map(|end| start + end + 1))
                .flatten()
        })
        .unwrap_or(0);
    let mut page = String::with_capacity(body.len() + head.len());
    page.push_str(&body[..at]);
    page.push_str(&head);
    page.push_str(&body[at..]);
    page
}

/// MonoCode `tag_end`: the byte just past the first opening `name` tag,
/// skipping longer names such as `<header>` for `<head`.
fn tag_end(lower: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let start = from + found;
        let next = start + name.len();
        match lower.as_bytes().get(next) {
            Some(b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') => {
                return lower[next..].find('>').map(|end| next + end + 1);
            }
            _ => from = next,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bootstrap_goes_first_in_head_and_never_before_the_doctype() {
        let page = page_html("<!DOCTYPE html><html><HEAD><title>x</title></HEAD></html>", "");
        assert!(page.starts_with("<!DOCTYPE html><html><HEAD><style>"));
        let page = page_html("<!doctype html>\n<body><header>x</header></body>", "");
        assert!(page.starts_with("<!doctype html><style>"));
        let page = page_html("<html lang=\"en\"><body></body></html>", "");
        assert!(page.starts_with("<html lang=\"en\"><style>"));
        let page = page_html("<div>fragment</div>", ":root{--accent:red;}");
        assert!(page.starts_with("<style>:root{--accent:red;}"));
        assert!(page.ends_with("</script><div>fragment</div>"));
    }
}
