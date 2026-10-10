//! One browser tab's native view: a WKWebView through wry, a child of the
//! window's view (as Ely's `WebView`, which only shows an address; this one
//! navigates, runs scripts and takes snapshots). What the page does comes
//! back as `PageEvent`s on the app's channel, tagged with the tab's id.

use std::cell::Cell;
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use gpui::{Bounds, Pixels};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebViewBuilder};

use super::scripts;

/// What the page reports. Only `Url`, `Title` and `Loading` come from
/// WebKit; the rest is the page's own word, which any site can forge.
#[derive(Debug)]
pub enum PageEvent {
    /// The address a load is at.
    Url(String),
    /// The page says its address changed without a load (`pushState`, a
    /// hash); `Page::url` has the address.
    Moved,
    Title(String),
    Loading(bool),
    /// A link or script asked for a new window; the tab goes there instead.
    NewWindow(String),
    /// The user pressed in the page: it has the keyboard now.
    Focused,
    /// A message from BenCode's scripts in the page (`picked`, …).
    Message(Value),
}

/// What `eval` fails with when the script threw, did not parse, or
/// evaluated to nothing.
pub const NO_RESULT: &str = "the script threw or returned nothing";

pub type PageEvents = UnboundedSender<(u64, PageEvent)>;

pub struct Page {
    view: wry::WebView,
    /// What the view was last given, so a redraw changes nothing native
    /// when nothing moved.
    placed: Cell<Option<(Bounds<Pixels>, bool)>>,
}

impl Page {
    /// A hidden view loading `url`; the pane places and shows it.
    pub fn build(id: u64, url: &str, window: &gpui::Window, events: PageEvents) -> Result<Self> {
        let send = move |event: PageEvent| {
            if events.send((id, event)).is_err() {
                log::debug!("browser tab {id}: event after the app left");
            }
        };
        let (on_load, on_title, on_window, on_message) =
            (send.clone(), send.clone(), send.clone(), send);
        let view = WebViewBuilder::new()
            .with_url(url)
            .with_visible(false)
            .with_devtools(true)
            .with_back_forward_navigation_gestures(true)
            .with_initialization_script(scripts::PRELUDE)
            .with_initialization_script_for_main_only(scripts::FOCUS, false)
            .with_on_page_load_handler(move |event, url| {
                on_load(PageEvent::Loading(matches!(event, PageLoadEvent::Started)));
                on_load(PageEvent::Url(url));
            })
            .with_document_title_changed_handler(move |title| on_title(PageEvent::Title(title)))
            .with_new_window_req_handler(move |url, _| {
                on_window(PageEvent::NewWindow(url));
                NewWindowResponse::Deny
            })
            .with_ipc_handler(move |request| {
                let Ok(message) = serde_json::from_str::<Value>(request.body()) else {
                    return;
                };
                match message.get("type").and_then(Value::as_str) {
                    Some("moved") => on_message(PageEvent::Moved),
                    Some("focused") => on_message(PageEvent::Focused),
                    _ => on_message(PageEvent::Message(message)),
                }
            })
            .build_as_child(window)
            .context("the web view could not be created")?;
        Ok(Self {
            view,
            placed: Cell::new(None),
        })
    }

    pub fn load(&self, url: &str) {
        if let Err(err) = self.view.load_url(url) {
            log::warn!("browser: could not load {url}: {err}");
        }
    }

    pub fn back(&self) {
        if let Err(err) = self.view.go_back() {
            log::warn!("browser: back failed: {err}");
        }
    }

    pub fn forward(&self) {
        if let Err(err) = self.view.go_forward() {
            log::warn!("browser: forward failed: {err}");
        }
    }

    pub fn reload(&self) {
        if let Err(err) = self.view.reload() {
            log::warn!("browser: reload failed: {err}");
        }
    }

    /// Where the view is, as WebKit has it.
    pub fn url(&self) -> Option<String> {
        self.view.url().ok()
    }

    /// Whether WebKit is loading. wry reports a load that starts and one
    /// that finishes, never one that fails (a server that is not running):
    /// this going false with no finish is that failure.
    pub fn is_loading(&self) -> bool {
        #[cfg(target_os = "macos")]
        return super::snapshot::is_loading(&self.view);
        #[cfg(not(target_os = "macos"))]
        false
    }

    pub fn can_go_back(&self) -> bool {
        self.view.can_go_back().unwrap_or(false)
    }

    pub fn can_go_forward(&self) -> bool {
        self.view.can_go_forward().unwrap_or(false)
    }

    pub fn open_devtools(&self) {
        self.view.open_devtools();
    }

    /// Puts the view over `bounds` (window coordinates), or hides it.
    pub fn place(&self, bounds: Bounds<Pixels>, visible: bool) {
        if self.placed.get() == Some((bounds, visible)) {
            return;
        }
        let shown = self.placed.get().is_some_and(|(_, shown)| shown);
        self.placed.set(Some((bounds, visible)));
        if visible {
            let rect = Rect {
                position: LogicalPosition::new(
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                )
                .into(),
                size: LogicalSize::new(f32::from(bounds.size.width), f32::from(bounds.size.height))
                    .into(),
            };
            if let Err(err) = self.view.set_bounds(rect) {
                log::warn!("browser: could not place the page: {err}");
            }
        }
        if visible != shown {
            if let Err(err) = self.view.set_visible(visible) {
                log::warn!("browser: could not show or hide the page: {err}");
            }
            if !visible {
                self.release_keys();
            }
        }
    }

    /// Hides the view (its tab is not drawn this frame).
    pub fn hide(&self) {
        if let Some((bounds, true)) = self.placed.get() {
            self.place(bounds, false);
        }
    }

    pub fn is_shown(&self) -> bool {
        self.placed.get().is_some_and(|(_, shown)| shown)
    }

    /// Where the view was last put (window coordinates), shown or not.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.placed.get().map(|(bounds, _)| bounds)
    }

    /// Gives the keyboard back to the window (a press anywhere GPUI draws).
    pub fn release_keys(&self) {
        if let Err(err) = self.view.focus_parent() {
            log::debug!("browser: keys stay with the page: {err}");
        }
    }

    /// Runs `js`; `done` gets the JSON string it evaluated to, or an error
    /// when it threw or returned nothing.
    pub fn eval(&self, js: &str, done: impl FnOnce(Result<Value>) + Send + 'static) {
        let done = Mutex::new(Some(done));
        let landed = self.view.evaluate_script_with_callback(js, move |raw| {
            let Some(done) = done.lock().ok().and_then(|mut d| d.take()) else {
                return;
            };
            done(decode_result(&raw));
        });
        if let Err(err) = landed {
            log::warn!("browser: script not run: {err}");
        }
    }

    /// A PNG of what the page shows, at the screen's resolution or no
    /// wider than `max_width` pixels.
    pub fn snapshot(&self, max_width: Option<u32>, done: impl FnOnce(Result<Vec<u8>>) + 'static) {
        #[cfg(target_os = "macos")]
        {
            let limit = max_width.zip(self.bounds()).map(|(max, bounds)| {
                super::snapshot::Limit {
                    view_width: f64::from(f32::from(bounds.size.width)),
                    max_pixels: max,
                }
            });
            super::snapshot::take(&self.view, limit, done);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = max_width;
            done(Err(anyhow::anyhow!("Snapshots need macOS for now.")));
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        self.release_keys();
    }
}

/// wry hands back the script's value as JSON; BenCode's scripts evaluate
/// to a JSON string, so the value is decoded twice.
fn decode_result(raw: &str) -> Result<Value> {
    if raw.is_empty() {
        anyhow::bail!(NO_RESULT);
    }
    let outer: Value = serde_json::from_str(raw).context("unreadable script result")?;
    match outer {
        Value::String(inner) => serde_json::from_str(&inner).context("unreadable script result"),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_decoded_twice() {
        let raw = serde_json::to_string(r#"{"ok":true,"value":"1"}"#).unwrap();
        assert_eq!(
            decode_result(&raw).unwrap(),
            serde_json::json!({ "ok": true, "value": "1" })
        );
        assert_eq!(decode_result("3").unwrap(), serde_json::json!(3));
        assert!(decode_result("").is_err());
    }
}
