//! The in-app browser (BenCode's own, after Codex desktop's): its tabs in
//! the file pane, what their pages report, the element picker and
//! screenshots that go to the composer, and the agents' browser tools
//! (`browser/mcp.rs`), answered here on the UI thread.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use bencode_ui::forms::{InputEvent, TextInput};
use gpui::{Context, Entity, Focusable as _, Subscription, Window};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::BenCodeApp;
use super::file_pane::PaneTab;
use crate::browser::bridge::{self, ToolCall, ToolReply};
use crate::browser::page::{Page, PageEvent, PageEvents};
use crate::browser::{McpLaunch, is_web_url, mcp_launch, normalize_url, scripts};

/// How long the agent's `browser_navigate` waits for the page to load.
pub(super) const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a script may take before its tool gives up.
pub(super) const SCRIPT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `browser_evaluate` waits for a promise.
pub(super) const PROMISE_TIMEOUT: Duration = Duration::from_secs(30);
/// What a click or a key gets to settle before its tool answers.
pub(super) const SETTLE: Duration = Duration::from_millis(350);
/// The page zooms, in percent (Chrome's steps).
const ZOOM_STEPS: [u16; 17] = [
    25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500,
];
pub const ZOOM_DEFAULT: u16 = 100;
/// How often an open inspector is asked who has the keyboard.
const KEYS_POLL: Duration = Duration::from_millis(250);
/// How often a load is asked whether it gave up (`watch_load`).
const LOAD_POLL: Duration = Duration::from_millis(250);
/// The widest screenshot an agent gets; wider ones are scaled down.
pub(super) const AGENT_SCREENSHOT_WIDTH: u32 = 1280;

/// What started a load `watch_load` follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadKind {
    /// An address: it loads, unless it only moves inside the page (a hash).
    Address,
    /// A reload: it loads the page again, so ending at the same address
    /// with no finish is a failure (WebKit keeps showing the page before).
    Reload,
    /// Back or Forward: may only move inside the page.
    History,
}

pub struct BrowserTab {
    /// None when WebKit made no view (`error` says why).
    pub page: Option<Rc<Page>>,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub error: Option<String>,
    /// The last load gave up (nothing answered at `url`); the pane says so
    /// in the page's place.
    pub load_failed: bool,
    /// The page's zoom, in percent.
    pub zoom: u16,
    /// Answered when a load that starts after them finishes.
    load_waiters: Vec<LoadWaiter>,
    /// Counts the loads that finished, and the watches started: a watch
    /// reads both to tell its load's end from another's.
    loads_finished: u64,
    load_watch: u64,
    /// Counts the inspector's openings, so one `watch_inspector_keys` runs.
    inspector_watch: u64,
}

impl BrowserTab {
    fn new(page: Option<Rc<Page>>, url: String, error: Option<String>) -> Self {
        Self {
            page,
            loading: url != "about:blank",
            url,
            title: String::new(),
            can_back: false,
            can_forward: false,
            error,
            load_failed: false,
            zoom: ZOOM_DEFAULT,
            load_waiters: Vec::new(),
            loads_finished: 0,
            load_watch: 0,
            inspector_watch: 0,
        }
    }

    /// The tab strip's label: the title once there is one.
    pub fn label(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
        match self.url.as_str() {
            "" | "about:blank" => "New Tab".into(),
            url => url
                .split("://")
                .nth(1)
                .unwrap_or(url)
                .trim_end_matches('/')
                .to_string(),
        }
    }
}

struct LoadWaiter {
    /// Whether the page loaded.
    done: oneshot::Sender<bool>,
    /// The page started loading since the waiter came: the next finish is
    /// its load's, not one already under way.
    started: bool,
}

pub struct BrowserState {
    pub tabs: HashMap<u64, BrowserTab>,
    next_id: u64,
    /// The address bar, shared by the tabs (it shows the active one's).
    pub url_input: Entity<TextInput>,
    events: PageEvents,
    /// The tab the agents' tools act on: the last one shown.
    pub current: Option<u64>,
    /// The tab whose element picker is on.
    pub picking: Option<u64>,
    /// A site's sign-in the user is asked (`browser_sign_in.rs`).
    pub sign_in: Option<super::browser_sign_in::BrowserSignIn>,
    /// Where the agents' MCP server reaches this app; None when the
    /// socket could not be opened.
    socket: Option<PathBuf>,
    /// Set while a dialog or a menu covers the window: the page hides
    /// under it.
    pub obscured: Rc<Cell<bool>>,
    /// The tab the file pane draws this frame; the others hide.
    pub drawn: Cell<Option<u64>>,
    pub(super) next_eval: u64,
    _subscriptions: Vec<Subscription>,
}

impl BrowserState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let url_input = super::text_input(window, cx, "Search or enter address");
        let (events, mut event_rx) = mpsc::unbounded_channel::<(u64, PageEvent)>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some((id, event)) = event_rx.recv().await {
                let landed = this.update_in(cx, |this, window, cx| {
                    this.on_page_event(id, event, window, cx)
                });
                if landed.is_err() {
                    break;
                }
            }
        })
        .detach();

        let (calls, mut call_rx) = mpsc::unbounded_channel::<ToolCall>();
        let socket = match bridge::listen(calls) {
            Ok(path) => Some(path),
            Err(err) => {
                log::error!("browser tools are off: {err:#}");
                None
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            while let Some(call) = call_rx.recv().await {
                let landed = this.update_in(cx, |this, window, cx| {
                    this.handle_tool_call(call, window, cx)
                });
                if landed.is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut subscriptions = vec![cx.subscribe(
            &url_input,
            |this: &mut BenCodeApp, input, event: &InputEvent, cx| {
                if *event == InputEvent::Submit {
                    let text = input.read(cx).text().to_string();
                    if let Some(id) = this.active_browser_id() {
                        this.browser_go(id, &normalize_url(&text), cx);
                    }
                }
            },
        )];
        if let Some(path) = socket.clone() {
            subscriptions.push(cx.on_app_quit(move |_, _| {
                bridge::remove(&path);
                async {}
            }));
        }
        Self {
            tabs: HashMap::new(),
            next_id: 1,
            url_input,
            events,
            current: None,
            picking: None,
            sign_in: None,
            socket,
            obscured: Rc::new(Cell::new(false)),
            drawn: Cell::new(None),
            next_eval: 1,
            _subscriptions: subscriptions,
        }
    }

    /// The left edge (window coordinates) of the page drawn this frame,
    /// from where it was last put.
    pub fn drawn_page_left(&self) -> Option<gpui::Pixels> {
        let page = self.tabs.get(&self.drawn.get()?)?.page.as_ref()?;
        Some(page.bounds()?.origin.x)
    }

    /// Hides every page but the one drawn this frame.
    pub fn hide_undrawn(&self) {
        let drawn = self.drawn.get();
        for (id, tab) in &self.tabs {
            if Some(*id) != drawn
                && let Some(page) = &tab.page
            {
                page.hide();
            }
        }
    }
}

/// Where a picked element lies in a snapshot `image` pixels wide, from its
/// box in a viewport `viewport` CSS pixels wide, with `pad` CSS pixels
/// around it; None when nothing of it shows.
pub fn crop_rect(
    image: (u32, u32),
    viewport: f64,
    rect: (f64, f64, f64, f64),
    pad: f64,
) -> Option<(u32, u32, u32, u32)> {
    if viewport <= 0.0 {
        return None;
    }
    let scale = f64::from(image.0) / viewport;
    let (x, y, w, h) = rect;
    let left = ((x - pad) * scale).max(0.0);
    let top = ((y - pad) * scale).max(0.0);
    let right = ((x + w + pad) * scale).min(f64::from(image.0));
    let bottom = ((y + h + pad) * scale).min(f64::from(image.1));
    if right - left < 1.0 || bottom - top < 1.0 {
        return None;
    }
    Some((
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

/// A page zoom a step from `zoom`: in (`true`) or out, stopping at the ends.
pub fn stepped_zoom(zoom: u16, zoom_in: bool) -> u16 {
    let next = if zoom_in {
        ZOOM_STEPS.iter().find(|step| **step > zoom)
    } else {
        ZOOM_STEPS.iter().rev().find(|step| **step < zoom)
    };
    next.copied().unwrap_or(zoom)
}

/// Whether two addresses are one page's: WebKit writes `http://a.dev` as
/// `http://a.dev/`.
fn same_address(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// What the composer gets with a picked element's image.
pub fn element_context(selector: &str, url: &str, html: &str) -> String {
    const MAX_HTML: usize = 1200;
    let html = html.trim();
    let html = match html.char_indices().nth(MAX_HTML) {
        Some((cut, _)) => format!("{}…", &html[..cut]),
        None => html.to_string(),
    };
    format!("Browser element `{selector}` on {url}:\n```html\n{html}\n```")
}

/// The agent's `browser_snapshot`, as text.
pub(super) fn outline_text(outline: &Value) -> String {
    let field = |key: &str| outline.get(key).and_then(Value::as_str).unwrap_or("");
    let elements: Vec<&str> = outline
        .get("elements")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let elements = if elements.is_empty() {
        "(none)".to_string()
    } else {
        elements.join("\n")
    };
    let meta = match (outline.get("viewport"), outline.get("scroll")) {
        (Some(vp), Some(sc)) => {
            let n = |o: &Value, k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0);
            format!(
                "\nViewport: {}x{} · Scroll: ({}/{}, {}/{})",
                n(vp, "width"),
                n(vp, "height"),
                n(sc, "x"),
                n(sc, "maxX"),
                n(sc, "y"),
                n(sc, "maxY"),
            )
        }
        _ => String::new(),
    };
    format!(
        "Title: {}\nURL: {}{meta}\n\nInteractive elements:\n{elements}\n\nText:\n{}",
        field("title"),
        field("url"),
        field("text")
    )
}

/// A script tool's `{ ok, error, … }` as a reply.
pub(super) fn script_reply(
    result: Result<Value, String>,
    done: impl FnOnce(&Value) -> String,
) -> ToolReply {
    match result {
        Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true) => {
            ToolReply::text(done(&value))
        }
        Ok(value) => ToolReply::error(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("The script failed.")
                .to_string(),
        ),
        Err(err) => ToolReply::error(err),
    }
}

/// Scales a PNG down to `max_width` (the agent's copy). The header says
/// whether there is anything to do.
pub(super) fn scaled_png(png: Vec<u8>, max_width: u32) -> anyhow::Result<Vec<u8>> {
    let format = image::ImageFormat::Png;
    let (width, _) =
        image::ImageReader::with_format(std::io::Cursor::new(&png), format).into_dimensions()?;
    if width <= max_width {
        return Ok(png);
    }
    let image = image::load_from_memory_with_format(&png, format)?;
    let height =
        (u64::from(image.height()) * u64::from(max_width) / u64::from(image.width())).max(1) as u32;
    let scaled = image.resize_exact(max_width, height, image::imageops::FilterType::Triangle);
    let mut out = std::io::Cursor::new(Vec::new());
    scaled.write_to(&mut out, format)?;
    Ok(out.into_inner())
}

/// A snapshot cut down to an element (`rect` + `viewport`), with `pad` CSS
/// pixels around it, or as it is when the element's box cannot be read.
pub(super) fn cropped_to_rect(png: Vec<u8>, picked: &Value, pad: f64) -> Vec<u8> {
    let cropped = || {
        let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64);
        let rect = picked.get("rect")?;
        let viewport = number(picked.get("viewport")?, "width")?;
        let format = image::ImageFormat::Png;
        let image = image::load_from_memory_with_format(&png, format).ok()?;
        let (x, y, w, h) = crop_rect(
            (image.width(), image.height()),
            viewport,
            (
                number(rect, "x")?,
                number(rect, "y")?,
                number(rect, "width")?,
                number(rect, "height")?,
            ),
            pad,
        )?;
        let mut out = std::io::Cursor::new(Vec::new());
        image.crop_imm(x, y, w, h).write_to(&mut out, format).ok()?;
        Some(out.into_inner())
    };
    cropped().unwrap_or(png)
}

/// A snapshot cut down to a picked element (the `picked` message's `rect`
/// and `viewport`), or as it is when the element's box cannot be read.
fn cropped_to_pick(png: Vec<u8>, picked: &Value) -> Vec<u8> {
    cropped_to_rect(png, picked, 8.0)
}

impl BenCodeApp {
    /// Opens a browser tab at `url` (blank, address bar focused, without).
    pub(crate) fn open_browser(
        &mut self,
        url: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.browser.next_id;
        self.browser.next_id += 1;
        let url = url
            .map(normalize_url)
            .unwrap_or_else(|| "about:blank".into());
        let (page, error) = match Page::build(id, &url, window, self.browser.events.clone()) {
            Ok(page) => (Some(Rc::new(page)), None),
            Err(err) => {
                log::error!("browser: {err:#}");
                (None, Some(format!("{err:#}")))
            }
        };
        self.browser
            .tabs
            .insert(id, BrowserTab::new(page, url.clone(), error));
        if url != "about:blank" {
            self.watch_load(id, LoadKind::Address, cx);
        }
        // Opening it fills the address bar (`settle_browser_tabs`).
        self.open_pane_tab(PaneTab::Browser { id }, true, cx);
        if url == "about:blank" {
            let focus = self.browser.url_input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        id
    }

    /// A link to a dev server: the current browser tab goes there, or a
    /// new one opens.
    pub(crate) fn open_url_in_browser(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .browser
            .current
            .filter(|id| self.browser.tabs.contains_key(id))
        {
            Some(id) => {
                self.open_pane_tab(PaneTab::Browser { id }, true, cx);
                self.browser_go(id, &normalize_url(url), cx);
            }
            None => {
                self.open_browser(Some(url), window, cx);
            }
        }
    }

    /// The footer's Browser button: the last browser tab, or a new one.
    pub(crate) fn show_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.agent_tab() {
            Some(id) => self.open_pane_tab(PaneTab::Browser { id }, true, cx),
            None => {
                self.open_browser(None, window, cx);
            }
        }
    }

    /// The active pane tab's browser, if it is one.
    pub(crate) fn active_browser_id(&self) -> Option<u64> {
        match self.file_pane.active() {
            Some(PaneTab::Browser { id }) => Some(*id),
            _ => None,
        }
    }

    pub(super) fn browser_page(&self, id: u64) -> Option<Rc<Page>> {
        self.browser.tabs.get(&id)?.page.clone()
    }

    /// Puts `url` in the address bar; a blank page leaves it empty.
    fn show_address(&mut self, url: &str, cx: &mut Context<Self>) {
        let text = if url == "about:blank" { "" } else { url };
        let text = text.to_string();
        self.browser
            .url_input
            .update(cx, |input, cx| input.set_text(text, cx));
    }

    /// After pane tabs changed: closed tabs drop their pages, the active
    /// one becomes the agents' and fills the address bar.
    pub(crate) fn settle_browser_tabs(&mut self, cx: &mut Context<Self>) {
        let pane = &self.file_pane;
        self.browser
            .tabs
            .retain(|id, _| pane.get(&PaneTab::Browser { id: *id }.key()).is_some());
        if self
            .browser
            .picking
            .is_some_and(|id| !self.browser.tabs.contains_key(&id))
        {
            self.browser.picking = None;
        }
        if let Some(id) = self.active_browser_id() {
            if self.browser.current != Some(id) {
                self.browser.current = Some(id);
                let url = self
                    .browser
                    .tabs
                    .get(&id)
                    .map(|t| t.url.clone())
                    .unwrap_or_default();
                self.show_address(&url, cx);
            }
        } else if self
            .browser
            .current
            .is_some_and(|id| !self.browser.tabs.contains_key(&id))
        {
            self.browser.current = self.browser.tabs.keys().max().copied();
        }
    }

    pub(crate) fn browser_go(&mut self, id: u64, url: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.browser.tabs.get_mut(&id) {
            tab.url = url.to_string();
            tab.loading = true;
            tab.load_failed = false;
            if let Some(page) = &tab.page {
                page.load(url);
            }
        }
        self.watch_load(id, LoadKind::Address, cx);
        cx.notify();
    }

    pub(crate) fn browser_back(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(page) = self.browser_page(id) {
            page.back();
            self.watch_load(id, LoadKind::History, cx);
        }
    }

    pub(crate) fn browser_forward(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(page) = self.browser_page(id) {
            page.forward();
            self.watch_load(id, LoadKind::History, cx);
        }
    }

    /// Reloads tab `id`. `from_origin` (⌘⇧R, ⇧-click) asks the server for
    /// every file again: a dev server's page a plain reload keeps broken,
    /// on files it rebuilt under the same address.
    pub(crate) fn browser_reload(&mut self, id: u64, from_origin: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        // WebKit would reload the page before the one that did not load.
        if tab.load_failed {
            let url = tab.url.clone();
            return self.browser_go(id, &url, cx);
        }
        if let Some(page) = &tab.page {
            tab.loading = true;
            if from_origin {
                page.reload_from_origin();
            } else {
                page.reload();
            }
            self.watch_load(id, LoadKind::Reload, cx);
            cx.notify();
        }
    }

    /// Follows a load just asked for until it finishes or gives up. wry
    /// passes on a load that starts and one that finishes, never one that
    /// fails (nothing listens at the address), so the view is asked.
    fn watch_load(&mut self, id: u64, kind: LoadKind, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        tab.load_watch += 1;
        let (watch, finished) = (tab.load_watch, tab.loads_finished);
        cx.spawn(async move |this, cx| {
            let (mut seen, mut idle) = (kind != LoadKind::History, 0);
            loop {
                cx.background_executor().timer(LOAD_POLL).await;
                let over = this.update(cx, |this, cx| {
                    let Some(tab) = this.browser.tabs.get(&id) else {
                        return true;
                    };
                    if tab.load_watch != watch || tab.loads_finished != finished {
                        return true;
                    }
                    if tab.page.as_ref().is_some_and(|page| page.is_loading()) {
                        (seen, idle) = (true, 0);
                        return false;
                    }
                    // Twice, so a finish still in the events' channel lands.
                    idle += 1;
                    if idle < 2 {
                        return false;
                    }
                    if seen {
                        this.browser_load_stalled(id, kind, cx);
                    }
                    true
                });
                if !matches!(over, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// A watched load ended with no finish. It failed, unless the view is
    /// where it was sent: a move inside the page (a hash) loads nothing.
    /// A reload has no such move: the view is where it was either way.
    fn browser_load_stalled(&mut self, id: u64, kind: LoadKind, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        let at = tab.page.as_ref().and_then(|page| page.url());
        let arrived = kind != LoadKind::Reload && at.is_some_and(|at| same_address(&at, &tab.url));
        tab.loading = false;
        tab.load_failed = !arrived;
        for waiter in std::mem::take(&mut tab.load_waiters) {
            if waiter.done.send(arrived).is_err() {
                log::debug!("browser load waited for by nobody");
            }
        }
        cx.notify();
    }

    /// Sets tab `id`'s page zoom, in percent.
    pub(crate) fn set_browser_zoom(&mut self, id: u64, zoom: u16, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        if tab.zoom == zoom {
            return;
        }
        tab.zoom = zoom;
        if let Some(page) = &tab.page {
            page.zoom(f64::from(zoom) / 100.0);
        }
        cx.notify();
    }

    /// One zoom step in or out for tab `id`.
    pub(crate) fn step_browser_zoom(&mut self, id: u64, zoom_in: bool, cx: &mut Context<Self>) {
        if let Some(zoom) = self.browser.tabs.get(&id).map(|tab| tab.zoom) {
            self.set_browser_zoom(id, stepped_zoom(zoom, zoom_in), cx);
        }
    }

    /// The browser tab ⌘+ ⌘− ⌘0 zoom instead of the interface: the shown
    /// one, while its page has the keyboard.
    pub(crate) fn browser_with_keys(&self) -> Option<u64> {
        let id = self.active_browser_id()?;
        let page = self.browser.tabs.get(&id)?.page.as_ref()?;
        (page.is_shown() && page.has_keys()).then_some(id)
    }

    /// The bug button: Web Inspector, opened or closed.
    pub(crate) fn browser_devtools(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.browser_page(id) else {
            return;
        };
        let opening = !page.devtools_open();
        page.toggle_devtools();
        if opening {
            self.watch_inspector_keys(id, window, cx);
        }
    }

    /// A docked inspector is a native view like the page, with the keyboard
    /// when pressed in, but it tells the app nothing (`PageEvent::Focused`
    /// is the page's). While it is open, an input GPUI still holds as
    /// focused, which would take the inspector's ⌘V, is let go of.
    fn watch_inspector_keys(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        /// The polls an inspector gets to show up: WebKit opens it a
        /// moment after it is asked to.
        const PATIENCE: u32 = 12;
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        tab.inspector_watch += 1;
        let watch = tab.inspector_watch;
        cx.spawn_in(window, async move |this, cx| {
            let (mut seen, mut polls) = (false, 0);
            loop {
                cx.background_executor().timer(KEYS_POLL).await;
                polls += 1;
                let open = this.update_in(cx, |this, window, cx| {
                    let Some(tab) = this.browser.tabs.get(&id) else {
                        return false;
                    };
                    let Some(page) = tab.page.as_ref() else {
                        return false;
                    };
                    if tab.inspector_watch != watch {
                        return false;
                    }
                    if !page.devtools_open() {
                        // Closed, maybe by its own button with the keys:
                        // they come back to the window.
                        if seen {
                            page.release_keys();
                        }
                        return !seen && polls < PATIENCE;
                    }
                    seen = true;
                    if page.box_has_keys() && !this.focus_handle.is_focused(window) {
                        window.focus(&this.focus_handle, cx);
                    }
                    true
                });
                if !matches!(open, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The pick button: outline elements under the pointer until one is
    /// clicked (or Esc, or the button again).
    pub(crate) fn toggle_browser_picker(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(page) = self.browser_page(id) else {
            return;
        };
        self.browser.picking = if self.browser.picking == Some(id) {
            None
        } else {
            Some(id)
        };
        page.eval(&scripts::picker(), |result| {
            if let Err(err) = result {
                log::warn!("browser: the element picker did not start: {err:#}");
            }
        });
        cx.notify();
    }

    /// Esc while the picker is on and the keyboard is BenCode's, not the
    /// page's (the pick button keeps it): the page's own Esc never fires.
    /// A dialog over the page takes the key first.
    pub(crate) fn close_browser_picker(&mut self, cx: &mut Context<Self>) -> bool {
        if self.browser.obscured.get() {
            return false;
        }
        match self.browser.picking {
            Some(id) => {
                self.toggle_browser_picker(id, cx);
                // A closed tab has no page to stop.
                self.browser.picking = None;
                true
            }
            None => false,
        }
    }

    /// The camera button: what the page shows goes to the composer.
    pub(crate) fn browser_screenshot_to_chat(&mut self, id: u64, cx: &mut Context<Self>) {
        self.snapshot_to_chat(id, None, cx);
    }

    /// Snapshots tab `id` and attaches it to the focused thread's composer;
    /// a picked element (`picked` message, prompt text) crops it to the
    /// element and adds the text to the draft.
    fn snapshot_to_chat(&mut self, id: u64, pick: Option<(Value, String)>, cx: &mut Context<Self>) {
        let Some(page) = self.browser_page(id) else {
            return;
        };
        if self.selected_session_id.is_none() {
            self.composer_error = Some("Open a thread to send it the page.".into());
            cx.notify();
            return;
        }
        let (tx, rx) = oneshot::channel();
        page.snapshot(None, move |png| {
            if tx.send(png).is_err() {
                log::debug!("browser snapshot after its caller left");
            }
        });
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let png = match rx.await {
                Ok(Ok(png)) => png,
                Ok(Err(err)) => {
                    log::warn!("browser: {err:#}");
                    let landed = this.update(cx, |this, cx| {
                        this.composer_error = Some(format!("No screenshot: {err:#}"));
                        cx.notify();
                    });
                    if let Err(err) = landed {
                        log::debug!("browser snapshot after app drop: {err:#}");
                    }
                    return;
                }
                Err(_) => return,
            };
            let stamp = super::now_ms();
            let (picked, context) = pick.unzip();
            let dir = match this.update(cx, |this, _| this.selected_session_id.clone()) {
                Ok(Some(id)) => crate::thread_files::attachments_dir(&id),
                Ok(None) => crate::ui::composer::attachments::paste_temp_dir(),
                Err(err) => {
                    log::debug!("browser snapshot after app drop: {err:#}");
                    return;
                }
            };
            let saved = executor
                .spawn(async move {
                    let png = match &picked {
                        Some(picked) => cropped_to_pick(png, picked),
                        None => png,
                    };
                    crate::ui::composer::attachments::write_pasted_images(
                        vec![(png, "png")],
                        stamp,
                        &dir,
                    )
                })
                .await;
            let landed = this.update(cx, |this, cx| {
                if let Some(context) = context {
                    this.add_to_chat(&context, cx);
                }
                this.attach_paths(saved, cx);
            });
            if let Err(err) = landed {
                log::debug!("browser snapshot after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// What a page reported. The app redraws only when it changed
    /// something: a page can send these as fast as it likes.
    fn on_page_event(
        &mut self,
        id: u64,
        event: PageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.browser.tabs.contains_key(&id) {
            return;
        }
        match event {
            PageEvent::Url(url) => self.browser_at(id, url, window, cx),
            PageEvent::Moved => {
                if let Some(url) = self.browser_page(id).and_then(|page| page.url()) {
                    self.browser_at(id, url, window, cx);
                }
            }
            PageEvent::Title(title) => {
                if let Some(tab) = self.browser.tabs.get_mut(&id)
                    && tab.title != title
                {
                    tab.title = title;
                    cx.notify();
                }
            }
            PageEvent::Loading(loading) => self.browser_loading(id, loading, cx),
            PageEvent::SignIn(asked) => self.ask_browser_sign_in(id, asked, window, cx),
            // Never a `file:` or `javascript:` address a site could not
            // have gone to itself.
            PageEvent::NewWindow(url) => {
                if is_web_url(&url) {
                    self.browser_go(id, &url, cx);
                } else {
                    log::debug!("browser: no window for {url}");
                }
            }
            // An input GPUI still holds as focused would take the page's
            // ⌘V and ⌘A: focus goes to the app's root, where the commands
            // still answer.
            PageEvent::Focused => {
                if !self.focus_handle.is_focused(window) {
                    window.focus(&self.focus_handle, cx);
                }
            }
            PageEvent::Message(message) => {
                // Any site can post these: only a pick the user started counts.
                if self.browser.picking != Some(id) {
                    return log::debug!("browser: page message outside a pick");
                }
                match message.get("type").and_then(Value::as_str) {
                    Some("picked") => {
                        let field = |key: &str| {
                            message
                                .get(key)
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string()
                        };
                        // The address is the view's, not the message's.
                        let url = self
                            .browser_page(id)
                            .and_then(|page| page.url())
                            .unwrap_or_default();
                        let context = element_context(&field("selector"), &url, &field("html"));
                        // The click gave the page the keys; what comes next
                        // is typed in the composer.
                        if let Some(page) = self.browser_page(id) {
                            page.release_keys();
                        }
                        self.snapshot_to_chat(id, Some((message, context)), cx);
                    }
                    Some("pickCancelled") => {}
                    other => return log::debug!("browser: unknown page message {other:?}"),
                }
                self.browser.picking = None;
                cx.notify();
            }
        }
    }

    /// Tab `id` is at `url` (WebKit's word).
    fn browser_at(&mut self, id: u64, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        let (can_back, can_forward) = tab
            .page
            .as_ref()
            .map_or((false, false), |p| (p.can_go_back(), p.can_go_forward()));
        if tab.url == url && tab.can_back == can_back && tab.can_forward == can_forward {
            return;
        }
        (tab.can_back, tab.can_forward) = (can_back, can_forward);
        // The view is somewhere: not at the address that did not load.
        tab.load_failed &= tab.url == url;
        tab.url = url.clone();
        // What the user is typing stays.
        let editing = self
            .browser
            .url_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !editing && self.active_browser_id() == Some(id) {
            self.show_address(&url, cx);
        }
        cx.notify();
    }

    /// A load of tab `id` started (its document is in) or finished.
    fn browser_loading(&mut self, id: u64, loading: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        tab.loading = loading;
        if loading {
            tab.load_failed = false;
            for waiter in &mut tab.load_waiters {
                waiter.started = true;
            }
            // A new document has no picker running.
            if self.browser.picking == Some(id) {
                self.browser.picking = None;
            }
        } else {
            tab.loads_finished += 1;
            let (finished, waiting) = std::mem::take(&mut tab.load_waiters)
                .into_iter()
                .partition(|waiter| waiter.started);
            tab.load_waiters = waiting;
            for waiter in finished {
                if waiter.done.send(true).is_err() {
                    log::debug!("browser load waited for by nobody");
                }
            }
        }
        cx.notify();
    }

    /// How a turn's agent reaches the browser: only while a browser tab is
    /// open, so the tools cost no tokens otherwise.
    pub(crate) fn browser_mcp_launch(&self) -> Option<McpLaunch> {
        if self.browser.tabs.is_empty() {
            return None;
        }
        let socket = self.browser.socket.as_ref()?;
        mcp_launch(socket)
            .map_err(|err| log::warn!("browser tools are off for this turn: {err:#}"))
            .ok()
    }

    /// The tab an agent's tool acts on: the one the user last showed.
    fn agent_tab(&self) -> Option<u64> {
        self.browser
            .current
            .filter(|id| self.browser.tabs.contains_key(id))
    }

    /// A waiter for the next load of tab `id`: whether it finished. Page
    /// events come through a channel, so one made right after a load starts
    /// sees it.
    pub(super) fn wait_for_load(&mut self, id: u64) -> oneshot::Receiver<bool> {
        let (done, rx) = oneshot::channel();
        if let Some(tab) = self.browser.tabs.get_mut(&id) {
            tab.load_waiters.push(LoadWaiter {
                done,
                started: false,
            });
        }
        rx
    }

    /// The tab a tool call acts on, shown to the user. `address`: the call
    /// is `browser_navigate`, which opens a tab there when none is open
    /// (`true` with the tab: it is loading the address already).
    pub(super) fn tool_tab(
        &mut self,
        address: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(u64, bool), String> {
        let (id, opened) = match (self.agent_tab(), address) {
            (Some(id), _) => (id, false),
            (None, Some(url)) => (self.open_browser(Some(url), window, cx), true),
            (None, None) => {
                return Err(
                    "No browser tab is open in BenCode. Open one with browser_navigate.".into(),
                );
            }
        };
        if self.browser_page(id).is_none() {
            let why = self.browser.tabs.get(&id).and_then(|t| t.error.clone());
            return Err(why.unwrap_or_else(|| "The browser tab has no page.".into()));
        }
        // The user sees what the agent does.
        if self.active_browser_id() != Some(id) {
            self.open_pane_tab(PaneTab::Browser { id }, true, cx);
        }
        Ok((id, opened))
    }
}

/// What a tab did not load says, to the agent and in the pane.
pub fn load_failure(url: &str) -> String {
    format!("{url} did not load. Is its server running?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_scale_css_pixels_to_the_snapshot() {
        // A 2x snapshot of a 500px-wide viewport.
        assert_eq!(
            crop_rect((1000, 800), 500.0, (100.0, 50.0, 40.0, 20.0), 8.0),
            Some((184, 84, 112, 72))
        );
        // Clamped to the image.
        assert_eq!(
            crop_rect((1000, 800), 500.0, (-20.0, 390.0, 60.0, 40.0), 0.0),
            Some((0, 780, 80, 20))
        );
        assert_eq!(
            crop_rect((1000, 800), 500.0, (600.0, 0.0, 10.0, 10.0), 0.0),
            None
        );
        assert_eq!(
            crop_rect((1000, 800), 0.0, (0.0, 0.0, 10.0, 10.0), 0.0),
            None
        );
    }

    #[test]
    fn zoom_steps_through_the_list_and_stops_at_its_ends() {
        assert_eq!(stepped_zoom(100, true), 110);
        assert_eq!(stepped_zoom(100, false), 90);
        assert_eq!(stepped_zoom(500, true), 500);
        assert_eq!(stepped_zoom(25, false), 25);
        // From a zoom that is no step, to the nearest one that way.
        assert_eq!(stepped_zoom(105, true), 110);
        assert_eq!(stepped_zoom(105, false), 100);
    }

    #[test]
    fn a_trailing_slash_is_the_same_address() {
        assert!(same_address("http://a.dev/", "http://a.dev"));
        assert!(same_address("http://a.dev/x#b", "http://a.dev/x#b"));
        assert!(!same_address("http://a.dev/x", "http://a.dev/y"));
    }

    #[test]
    fn element_context_is_a_fenced_snippet() {
        let context = element_context(
            "#go",
            "http://localhost:3000/",
            "<button id=\"go\">Go</button>",
        );
        assert_eq!(
            context,
            "Browser element `#go` on http://localhost:3000/:\n```html\n<button id=\"go\">Go</button>\n```"
        );
        let long = "x".repeat(2000);
        assert!(element_context("a", "u", &long).ends_with("…\n```"));
    }

    #[test]
    fn outlines_read_as_text() {
        let outline = serde_json::json!({
            "title": "Login",
            "url": "http://localhost:3000/login",
            "text": "Welcome",
            "elements": ["[1] button \"Sign in\""],
        });
        assert_eq!(
            outline_text(&outline),
            "Title: Login\nURL: http://localhost:3000/login\n\nInteractive elements:\n[1] button \"Sign in\"\n\nText:\nWelcome"
        );
    }

    #[test]
    fn tabs_are_labelled_by_title_then_address() {
        let mut tab = BrowserTab::new(None, "http://localhost:3000/".into(), None);
        assert_eq!(tab.label(), "localhost:3000");
        tab.title = "Dashboard".into();
        assert_eq!(tab.label(), "Dashboard");
        tab.title.clear();
        tab.url = "about:blank".into();
        assert_eq!(tab.label(), "New Tab");
    }
}
