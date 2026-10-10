//! The in-app browser (BenCode's own, after Codex desktop's): its tabs in
//! the file pane, what their pages report, the element picker and
//! screenshots that go to the composer, and the agents' browser tools
//! (`browser/mcp.rs`), answered here on the UI thread.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use base64::Engine as _;
use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{
    AsyncApp, Context, Entity, Focusable as _, FutureExt as _, Subscription, WeakEntity, Window,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::BenCodeApp;
use super::file_pane::PaneTab;
use crate::browser::bridge::{self, ToolCall, ToolReply};
use crate::browser::bridge::ToolRequest;
use crate::browser::page::{NO_RESULT, Page, PageEvent, PageEvents};
use crate::browser::{McpLaunch, agent_may_open, is_web_url, normalize_url, scripts};

/// How long the agent's `browser_navigate` waits for the page to load.
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a script may take before its tool gives up.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `browser_evaluate` waits for a promise.
const PROMISE_TIMEOUT: Duration = Duration::from_secs(30);
/// What a click or a key gets to settle before its tool answers.
const SETTLE: Duration = Duration::from_millis(350);
/// How often a load is asked whether it gave up (`watch_load`).
const LOAD_POLL: Duration = Duration::from_millis(250);
/// The widest screenshot an agent gets; wider ones are scaled down.
const AGENT_SCREENSHOT_WIDTH: u32 = 1280;

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
    /// Answered when a load that starts after them finishes.
    load_waiters: Vec<LoadWaiter>,
    /// Counts the loads that finished, and the watches started: a watch
    /// reads both to tell its load's end from another's.
    loads_finished: u64,
    load_watch: u64,
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
            load_waiters: Vec::new(),
            loads_finished: 0,
            load_watch: 0,
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
    /// Where the agents' MCP server reaches this app; None when the
    /// socket could not be opened.
    socket: Option<PathBuf>,
    /// Set while a dialog covers the window: the page hides under it.
    pub obscured: Rc<Cell<bool>>,
    /// The tab the file pane draws this frame; the others hide.
    pub drawn: Cell<Option<u64>>,
    next_eval: u64,
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
            socket,
            obscured: Rc::new(Cell::new(false)),
            drawn: Cell::new(None),
            next_eval: 1,
            _subscriptions: subscriptions,
        }
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
fn outline_text(outline: &Value) -> String {
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
    format!(
        "Title: {}\nURL: {}\n\nInteractive elements:\n{elements}\n\nText:\n{}",
        field("title"),
        field("url"),
        field("text")
    )
}

/// A script tool's `{ ok, error, … }` as a reply.
fn script_reply(result: Result<Value, String>, done: impl FnOnce(&Value) -> String) -> ToolReply {
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
fn scaled_png(png: Vec<u8>, max_width: u32) -> anyhow::Result<Vec<u8>> {
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

/// A snapshot cut down to a picked element (the `picked` message's `rect`
/// and `viewport`), or as it is when the element's box cannot be read.
fn cropped_to_pick(png: Vec<u8>, picked: &Value) -> Vec<u8> {
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
            8.0,
        )?;
        let mut out = std::io::Cursor::new(Vec::new());
        image.crop_imm(x, y, w, h).write_to(&mut out, format).ok()?;
        Some(out.into_inner())
    };
    let cropped = cropped();
    cropped.unwrap_or(png)
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
            self.watch_load(id, true, cx);
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

    fn browser_page(&self, id: u64) -> Option<Rc<Page>> {
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
        self.watch_load(id, true, cx);
        cx.notify();
    }

    pub(crate) fn browser_back(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(page) = self.browser_page(id) {
            page.back();
            self.watch_load(id, false, cx);
        }
    }

    pub(crate) fn browser_forward(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(page) = self.browser_page(id) {
            page.forward();
            self.watch_load(id, false, cx);
        }
    }

    pub(crate) fn browser_reload(&mut self, id: u64, cx: &mut Context<Self>) {
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
            page.reload();
            self.watch_load(id, true, cx);
            cx.notify();
        }
    }

    /// Follows a load just asked for until it finishes or gives up. wry
    /// passes on a load that starts and one that finishes, never one that
    /// fails (nothing listens at the address), so the view is asked.
    /// `expected`: the load must happen (an address, a reload); Back and
    /// Forward may only move inside the page.
    fn watch_load(&mut self, id: u64, expected: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        tab.load_watch += 1;
        let (watch, finished) = (tab.load_watch, tab.loads_finished);
        cx.spawn(async move |this, cx| {
            let (mut seen, mut idle) = (expected, 0);
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
                        this.browser_load_stalled(id, cx);
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
    fn browser_load_stalled(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.browser.tabs.get_mut(&id) else {
            return;
        };
        let at = tab.page.as_ref().and_then(|page| page.url());
        let arrived = at.is_some_and(|at| same_address(&at, &tab.url));
        tab.loading = false;
        tab.load_failed = !arrived;
        for waiter in std::mem::take(&mut tab.load_waiters) {
            if waiter.done.send(arrived).is_err() {
                log::debug!("browser load waited for by nobody");
            }
        }
        cx.notify();
    }

    pub(crate) fn browser_devtools(&mut self, id: u64) {
        if let Some(page) = self.browser_page(id) {
            page.open_devtools();
        }
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
            let saved = executor
                .spawn(async move {
                    let png = match &picked {
                        Some(picked) => cropped_to_pick(png, picked),
                        None => png,
                    };
                    crate::ui::composer::attachments::write_pasted_images(vec![(png, "png")], stamp)
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
            // ⌘V and ⌘A.
            PageEvent::Focused => {
                if window.focused(cx).is_some() {
                    window.blur(cx);
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
        McpLaunch::new(socket)
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
    fn wait_for_load(&mut self, id: u64) -> oneshot::Receiver<bool> {
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
    fn tool_tab(
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

    fn handle_tool_call(
        &mut self,
        mut call: ToolCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ToolRequest { tool, args } = std::mem::take(&mut call.request);
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
        let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
        let address = match (tool == "browser_navigate").then(|| text("url")) {
            None => None,
            Some(None) => return call.answer(ToolReply::error("browser_navigate needs a url.")),
            Some(Some(url)) => {
                let url = normalize_url(&url);
                if !agent_may_open(&url) {
                    return call.answer(ToolReply::error(format!(
                        "browser_navigate opens http, https and file addresses, not {url}."
                    )));
                }
                Some(url)
            }
        };
        let (id, opened) = match self.tool_tab(address.as_deref(), window, cx) {
            Ok(tab) => tab,
            Err(why) => return call.answer(ToolReply::error(why)),
        };
        let this = cx.entity().downgrade();
        match tool.as_str() {
            "browser_navigate" => {
                let loaded = self.wait_for_load(id);
                // A tab opened above is already loading it.
                if let Some(url) = address.filter(|_| !opened) {
                    self.browser_go(id, &url, cx);
                }
                cx.spawn(async move |_, cx| {
                    let reply = await_load(loaded, &this, id, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_history" => {
                let action = text("action");
                if !matches!(action.as_deref(), Some("back" | "forward" | "reload")) {
                    return call.answer(ToolReply::error("action must be back, forward or reload."));
                }
                let loaded = self.wait_for_load(id);
                match action.as_deref() {
                    Some("back") => self.browser_back(id, cx),
                    Some("forward") => self.browser_forward(id, cx),
                    _ => self.browser_reload(id, cx),
                }
                cx.spawn(async move |_, cx| {
                    // A step inside the page (a hash, `pushState`) loads
                    // nothing to wait for.
                    cx.background_executor().timer(SETTLE).await;
                    let loading = this
                        .update(cx, |this, _| {
                            this.browser_page(id).is_some_and(|p| p.is_loading())
                        })
                        .unwrap_or(false);
                    let reply = if loading {
                        await_load(loaded, &this, id, cx).await
                    } else {
                        where_now(&this, id, "Now at", cx)
                    };
                    call.answer(reply);
                })
                .detach();
            }
            "browser_snapshot" => {
                cx.spawn(async move |_, cx| {
                    let reply = match eval(&this, id, scripts::outline(), cx).await {
                        Ok(outline) => ToolReply::text(outline_text(&outline)),
                        Err(err) => ToolReply::error(err),
                    };
                    call.answer(reply);
                })
                .detach();
            }
            "browser_screenshot" => {
                cx.notify();
                cx.spawn(async move |_, cx| {
                    let reply = screenshot(&this, id, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_click" | "browser_type" => {
                let reference = args.get("ref").and_then(Value::as_u64);
                let selector = text("selector");
                let js = if tool == "browser_click" {
                    scripts::click(reference, selector.as_deref())
                } else {
                    let typed = text("text").unwrap_or_default();
                    scripts::type_text(reference, selector.as_deref(), &typed, flag("submit"))
                };
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, js, cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    let reply = script_reply(result, |value| {
                        let what = |key| value.get(key).and_then(Value::as_str);
                        match (what("clicked"), what("typed")) {
                            (Some(el), _) => format!("Clicked {el}."),
                            (_, Some(el)) => format!("Typed into {el}."),
                            _ => "Done.".into(),
                        }
                    });
                    call.answer(reply);
                })
                .detach();
            }
            "browser_press_key" => {
                let Some(key) = text("key") else {
                    return call.answer(ToolReply::error("browser_press_key needs a key."));
                };
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, scripts::press_key(&key), cx).await;
                    if result.is_ok() {
                        cx.background_executor().timer(SETTLE).await;
                    }
                    call.answer(script_reply(result, |value| {
                        let effect = value.get("effect").and_then(Value::as_str).unwrap_or("");
                        format!("Pressed {key}{effect}.")
                    }));
                })
                .detach();
            }
            "browser_evaluate" => {
                let Some(expression) = text("expression") else {
                    return call.answer(ToolReply::error("browser_evaluate needs an expression."));
                };
                let eval_id = self.browser.next_eval;
                self.browser.next_eval += 1;
                cx.spawn(async move |_, cx| {
                    let reply = evaluate(&this, id, &expression, eval_id, cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            "browser_console" => {
                let clear = flag("clear");
                cx.spawn(async move |_, cx| {
                    let result = eval(&this, id, scripts::console(clear), cx).await;
                    call.answer(script_reply(result, |value| {
                        let lines: Vec<&str> = value
                            .get("lines")
                            .and_then(Value::as_array)
                            .map(|l| l.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        if lines.is_empty() {
                            "(no console messages)".into()
                        } else {
                            lines.join("\n")
                        }
                    }));
                })
                .detach();
            }
            "browser_wait" => {
                let ms = args
                    .get("ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(10_000)
                    .min(60_000);
                let wanted = text("text");
                cx.spawn(async move |_, cx| {
                    let reply = wait(&this, id, wanted, Duration::from_millis(ms), cx).await;
                    call.answer(reply);
                })
                .detach();
            }
            other => call.answer(ToolReply::error(format!("Unknown browser tool {other}."))),
        }
    }
}

/// Runs a script in tab `id` and decodes what it evaluated to.
async fn eval(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    js: String,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel();
    let started = this.update(cx, |this, _| {
        let page = this.browser_page(id)?;
        page.eval(&js, move |result| {
            if tx.send(result).is_err() {
                log::debug!("browser script answered after its caller left");
            }
        });
        Some(())
    });
    if !matches!(started, Ok(Some(()))) {
        return Err("The browser tab closed.".into());
    }
    match rx
        .with_timeout(SCRIPT_TIMEOUT, cx.background_executor())
        .await
    {
        Ok(Ok(result)) => result.map_err(|err| format!("{err:#}")),
        Ok(Err(_)) => Err("The browser tab closed.".into()),
        Err(_) => Err("The page did not answer in time (is it still loading?).".into()),
    }
}

/// What a tab did not load says, to the agent and in the pane.
pub fn load_failure(url: &str) -> String {
    format!("{url} did not load. Is its server running?")
}

/// Where tab `id` is, as a tool's answer (`verb`: "Loaded", "Now at").
fn where_now(this: &WeakEntity<BenCodeApp>, id: u64, verb: &str, cx: &mut AsyncApp) -> ToolReply {
    let state = this
        .update(cx, |this, _| {
            this.browser
                .tabs
                .get(&id)
                .map(|tab| (tab.title.clone(), tab.url.clone(), tab.load_failed))
        })
        .ok()
        .flatten();
    match state {
        None => ToolReply::error("The browser tab closed."),
        Some((_, url, true)) => ToolReply::error(load_failure(&url)),
        // The title can come after the load finishes.
        Some((title, url, false)) if title.trim().is_empty() => {
            ToolReply::text(format!("{verb} {url}"))
        }
        Some((title, url, false)) => ToolReply::text(format!("{verb}: {title}\nURL: {url}")),
    }
}

/// Waits for a load started by a tool, then says where the tab is.
async fn await_load(
    loaded: oneshot::Receiver<bool>,
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    cx: &mut AsyncApp,
) -> ToolReply {
    match loaded
        .with_timeout(LOAD_TIMEOUT, cx.background_executor())
        .await
    {
        // `where_now` tells a load that failed from the tab's state.
        Ok(Ok(_)) => where_now(this, id, "Loaded", cx),
        Ok(Err(_)) => ToolReply::error("The browser tab closed."),
        Err(_) => {
            let url = this
                .update(cx, |this, _| {
                    this.browser.tabs.get(&id).map(|tab| tab.url.clone())
                })
                .ok()
                .flatten()
                .unwrap_or_default();
            ToolReply::error(format!(
                "{url} did not finish loading in {}s. \
                 browser_snapshot shows what the tab has now.",
                LOAD_TIMEOUT.as_secs()
            ))
        }
    }
}

async fn screenshot(this: &WeakEntity<BenCodeApp>, id: u64, cx: &mut AsyncApp) -> ToolReply {
    // A tab shown just now needs a frame to be placed before WebKit draws it.
    let shown = this
        .update(cx, |this, _| {
            this.browser_page(id).is_some_and(|p| p.is_shown())
        })
        .unwrap_or(false);
    if !shown {
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
    }
    let (tx, rx) = oneshot::channel();
    let started = this.update(cx, |this, _| {
        let page = this.browser_page(id)?;
        page.snapshot(Some(AGENT_SCREENSHOT_WIDTH), move |png| {
            if tx.send(png).is_err() {
                log::debug!("browser snapshot after its caller left");
            }
        });
        Some(())
    });
    if !matches!(started, Ok(Some(()))) {
        return ToolReply::error("The browser tab closed.");
    }
    let png = match rx
        .with_timeout(SCRIPT_TIMEOUT, cx.background_executor())
        .await
    {
        Ok(Ok(Ok(png))) => png,
        Ok(Ok(Err(err))) => return ToolReply::error(format!("{err:#}")),
        Ok(Err(_)) => return ToolReply::error("The browser tab closed."),
        Err(_) => return ToolReply::error("WebKit took no snapshot in time."),
    };
    let encoded = cx
        .background_executor()
        .spawn(async move {
            scaled_png(png, AGENT_SCREENSHOT_WIDTH)
                .map(|png| base64::engine::general_purpose::STANDARD.encode(png))
        })
        .await;
    match encoded {
        Ok(image) => ToolReply {
            ok: true,
            text: "Screenshot of the browser tab.".into(),
            image: Some(image),
        },
        Err(err) => ToolReply::error(format!("The screenshot could not be read: {err:#}")),
    }
}

async fn evaluate(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    expression: &str,
    eval_id: u64,
    cx: &mut AsyncApp,
) -> ToolReply {
    let shown = |value: &Value| {
        value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("undefined")
            .to_string()
    };
    // In the script itself first, which a strict page allows; `eval` when
    // that does not parse (statements, a mistake `eval` can name).
    let mut first = eval(this, id, scripts::evaluate(expression, eval_id, true), cx).await;
    if first.as_ref().is_err_and(|err| err == NO_RESULT) {
        first = eval(this, id, scripts::evaluate(expression, eval_id, false), cx).await;
    }
    let pending =
        matches!(&first, Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true));
    if !pending {
        return script_reply(first, shown);
    }
    let deadline = std::time::Instant::now() + PROMISE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
        let result = eval(this, id, scripts::eval_result(eval_id), cx).await;
        let still =
            matches!(&result, Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true));
        if !still {
            return script_reply(result, shown);
        }
    }
    ToolReply::error(format!(
        "The promise did not settle in {}s.",
        PROMISE_TIMEOUT.as_secs()
    ))
}

async fn wait(
    this: &WeakEntity<BenCodeApp>,
    id: u64,
    wanted: Option<String>,
    limit: Duration,
    cx: &mut AsyncApp,
) -> ToolReply {
    let Some(wanted) = wanted else {
        cx.background_executor().timer(limit).await;
        return ToolReply::text(format!("Waited {}ms.", limit.as_millis()));
    };
    let deadline = std::time::Instant::now() + limit;
    loop {
        let found = eval(this, id, scripts::has_text(&wanted), cx)
            .await
            .ok()
            .and_then(|v| v.get("found").and_then(Value::as_bool))
            .unwrap_or(false);
        if found {
            return ToolReply::text(format!("\"{wanted}\" is on the page."));
        }
        if std::time::Instant::now() >= deadline {
            return ToolReply::error(format!(
                "\"{wanted}\" did not show in {}ms.",
                limit.as_millis()
            ));
        }
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
    }
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
