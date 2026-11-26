//! Local macOS Computer Use. The MCP child relays the authenticated call
//! to the app, which owns the desktop grant and macOS permissions.

mod ax;
mod input;
mod screen;
mod windows;

use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context as _, Result, bail};
use bencode_core::mcp::{COMPUTER_SERVER, COMPUTER_TURN_ENV, McpLaunch};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp::bridge::{self, ToolReply, ToolRequest};
use crate::mcp::server::{self, Reply, Server};

const ARG: &str = "--computer-use-mcp";

pub fn launch(socket: &Path) -> Result<McpLaunch> {
    let exe = std::env::current_exe().context("cannot find BenCode's executable")?;
    Ok(McpLaunch {
        name: COMPUTER_SERVER.into(),
        command: exe.to_string_lossy().into_owned(),
        args: vec![ARG.into(), socket.to_string_lossy().into_owned()],
    })
}

pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(ARG.as_ref()) {
        return None;
    }
    let Some(socket) = args.next() else {
        eprintln!("{ARG} requires the BenCode tool socket");
        return Some(2);
    };
    let turn = std::env::var(COMPUTER_TURN_ENV).unwrap_or_default();
    Some(server::run(&DesktopServer {
        socket: Path::new(&socket),
        turn: &turn,
    }))
}

struct DesktopServer<'a> {
    socket: &'a Path,
    turn: &'a str,
}

impl Server for DesktopServer<'_> {
    fn name(&self) -> &'static str {
        COMPUTER_SERVER
    }

    fn instructions(&self) -> &'static str {
        "Use these tools for native macOS apps (use the in-app browser tools for web pages). \
         The user approves desktop access for this turn.\n\
         Work fast and accurately:\n\
         1. Prefer computer_ui (Accessibility outline + control) and keyboard shortcuts over pixel clicking. \
            Call computer_ui with action='outline' (optionally with query or interactiveOnly=true; windowId defaults to the active external window) to inspect controls and their bounds, then use verb='press'/'setText'/'focus'/'click'/'doubleClick'.\n\
         2. Batch predictable steps with computer_batch (e.g. click/ui -> type -> key Return -> wait -> outline or screenshot) so you finish multi-step interactions in one turn instead of taking a screenshot after every action.\n\
         3. When you need visual inspection or pixel clicking, prefer computer_screenshot with windowId (it returns a clickable screenshotId with sharper detail and smaller payload than the whole display). Mouse coordinates are pixels in the referenced screenshot (origin at top left).\n\
         4. Open apps by bundle ID with computer_open_app or via Spotlight (Cmd+Space). Key presses and text go to the foreground window."
    }

    fn tools(&self) -> Value {
        json!([
            {
                "name": "computer_displays",
                "description": "List active displays, their IDs and their desktop bounds in logical points.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "computer_screenshot",
                "description": "Capture a display or a single window (default JPEG, maxSize 1280, cursor visible). Both display and on-screen window captures return a screenshotId usable with computer_mouse; prefer windowId when working in one app for sharper, smaller images. Defaults to the main display.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "displayId": { "type": "integer", "minimum": 1 },
                        "windowId": { "type": "integer", "minimum": 1, "description": "Capture this window instead of a display (preferred when acting on one window)." },
                        "maxSize": { "type": "integer", "minimum": 640, "maximum": 2560, "default": 1280,
                            "description": "Max pixel length of the longest side (default 1280)." },
                        "format": { "type": "string", "enum": ["jpeg", "png"], "default": "jpeg",
                            "description": "Image format. JPEG is much faster and smaller; use PNG only when exact lossless pixels are needed." }
                    }
                }
            },
            {
                "name": "computer_mouse",
                "description": "Move, click, hold (down), release (up), drag, or scroll using pixel coordinates from a display or window screenshotId. Move while a button is held to drag across calls. For a single-call drag, toScreenshotId can target another display or window. Up can omit coordinates and release at the current pointer.",
                "inputSchema": {
                    "type": "object",
                    "required": ["action"],
                    "properties": {
                        "action": { "type": "string", "enum": ["move", "click", "down", "up", "drag", "scroll"] },
                        "screenshotId": { "type": "integer" },
                        "x": { "type": "number", "minimum": 0 },
                        "y": { "type": "number", "minimum": 0 },
                        "button": { "type": "string", "enum": ["left", "right", "middle"], "default": "left" },
                        "count": { "type": "integer", "minimum": 1, "maximum": 3, "default": 1 },
                        "toScreenshotId": { "type": "integer", "description": "Defaults to screenshotId; capture both displays/windows before dragging between them." },
                        "toX": { "type": "number", "minimum": 0 },
                        "toY": { "type": "number", "minimum": 0 },
                        "deltaX": { "type": "integer", "minimum": -2000, "maximum": 2000, "description": "Positive scrolls right." },
                        "deltaY": { "type": "integer", "minimum": -2000, "maximum": 2000, "description": "Positive scrolls down." },
                        "modifiers": { "type": "array",
                            "items": { "type": "string", "enum": ["Cmd", "Ctrl", "Alt", "Shift"] } }
                    }
                }
            },
            {
                "name": "computer_key",
                "description": "Press a key or chord in the foreground window. Use down/up with one key to hold it across calls (e.g. Shift or Space), or repeat to repeat a held key. Letter and punctuation keys use macOS virtual key codes.",
                "inputSchema": {
                    "type": "object", "required": ["key"],
                    "properties": {
                        "key": { "type": "string", "maxLength": 80 },
                        "action": { "type": "string", "enum": ["press", "down", "up", "repeat"], "default": "press" }
                    }
                }
            },
            {
                "name": "computer_type",
                "description": "Type Unicode text into the focused control. Newlines and tabs act as Return and Tab key presses.",
                "inputSchema": {
                    "type": "object", "required": ["text"],
                    "properties": { "text": { "type": "string", "maxLength": 4000 } }
                }
            },
            {
                "name": "computer_windows",
                "description": "List normal windows and their owner, windowId and desktop bounds in points, from front to back.",
                "inputSchema": { "type": "object", "properties": {
                    "includeHidden": { "type": "boolean", "default": false },
                    "pid": { "type": "integer", "minimum": 1 }
                } }
            },
            {
                "name": "computer_window",
                "description": "Focus, minimize, restore, move, resize, toggle fullscreen, or close a window. The app and macOS may restrict specific operations.",
                "inputSchema": {
                    "type": "object", "required": ["windowId", "action"],
                    "properties": {
                        "windowId": { "type": "integer", "minimum": 1 },
                        "action": { "type": "string", "enum": ["focus", "minimize", "restore", "move", "resize", "fullscreen", "close"] },
                        "x": { "type": "number" }, "y": { "type": "number" },
                        "width": { "type": "number", "minimum": 1 }, "height": { "type": "number", "minimum": 1 },
                        "enabled": { "type": "boolean", "description": "Required for fullscreen." }
                    }
                }
            },
            {
                "name": "computer_ui",
                "description": "Preferred way to inspect and drive native macOS controls via Accessibility. 'outline' returns meaningful elements with refs and [x,y,w,h] bounds (skipping empty layout wrappers); 'control' invokes press, focus, increment, decrement, setText, click, or doubleClick on an element ref without needing a screenshot. Omit windowId to target the frontmost external window.",
                "inputSchema": {
                    "type": "object", "required": ["action"],
                    "properties": {
                        "windowId": { "type": "integer", "minimum": 1, "description": "Target window ID. Defaults to the frontmost external window for outline (or frontmost window for control)." },
                        "action": { "type": "string", "enum": ["outline", "control"] },
                        "query": { "type": "string", "maxLength": 200, "description": "Optional case-insensitive filter on label, role, identifier or value (outline only)." },
                        "interactiveOnly": { "type": "boolean", "default": false, "description": "Only return actionable controls (outline only)." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 250, "default": 150, "description": "Max elements to return (outline only)." },
                        "element": { "type": "string", "description": "Element ref from the current outline. Required for control." },
                        "verb": { "type": "string", "enum": ["press", "focus", "increment", "decrement", "setText", "click", "doubleClick"], "description": "Required for control. Use click/doubleClick when AXPress is unsupported." },
                        "text": { "type": "string", "maxLength": 4000, "description": "Required for setText." }
                    }
                }
            },
            {
                "name": "computer_open_app",
                "description": "Open or bring an installed app to the foreground by its macOS bundle identifier, e.g. com.apple.calculator.",
                "inputSchema": {
                    "type": "object", "required": ["bundleId"],
                    "properties": { "bundleId": { "type": "string", "maxLength": 255 } }
                }
            },
            {
                "name": "computer_batch",
                "description": "Execute a sequence of 1-20 desktop steps in a single turn (stops on first error). Use this to combine actions like click/ui -> type -> key -> wait -> outline/screenshot without extra round-trips.",
                "inputSchema": {
                    "type": "object",
                    "required": ["steps"],
                    "properties": {
                        "steps": {
                            "type": "array",
                            "description": "1 to 20 steps to execute in sequence.",
                            "items": {
                                "type": "object",
                                "required": ["tool"],
                                "properties": {
                                    "tool": {
                                        "type": "string",
                                        "enum": ["mouse", "key", "type", "ui", "window", "open_app", "wait", "screenshot", "windows"],
                                        "description": "Step tool name (short name or full computer_* name). 'wait' pauses for args.ms (50-5000)."
                                    },
                                    "args": {
                                        "type": "object",
                                        "description": "Arguments matching the corresponding computer_* tool, or { \"ms\": 300 } for wait."
                                    }
                                }
                            }
                        }
                    }
                }
            }
        ])
    }

    fn call(&self, tool: &str, args: &Value) -> Reply {
        if self.turn.is_empty() {
            return Reply::error(
                "Computer Use is available only in the BenCode turn that launched it.",
            );
        }
        let request = ToolRequest {
            token: String::new(),
            computer_turn: Some(self.turn.into()),
            tool: tool.into(),
            args: args.clone(),
        };
        match bridge::call(self.socket, &request) {
            Ok(ToolReply {
                ok: true,
                text,
                image: Some(image),
                image_mime,
            }) => Reply::text_with_image(
                text,
                image,
                image_mime.unwrap_or_else(|| "image/png".into()),
            ),
            Ok(ToolReply { ok: true, text, .. }) => Reply::text(text),
            Ok(ToolReply {
                ok: false, text, ..
            }) => Reply::error(text),
            Err(err) => Reply::error(format!("Could not reach BenCode for Computer Use: {err:#}")),
        }
    }
}

const DEFAULT_SCREENSHOT_MAX_SIZE: u32 = 1280;
const MAX_BATCH_STEPS: usize = 20;
const MIN_WAIT_MS: u64 = 50;
const MAX_WAIT_MS: u64 = 5_000;
const MAX_BATCH_WAIT_MS: u64 = 15_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Screenshot {
    display_id: Option<u32>,
    window_id: Option<u32>,
    max_size: Option<u32>,
    #[serde(default)]
    format: screen::ImageFormat,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenApp {
    bundle_id: String,
}

#[derive(Deserialize)]
struct Key {
    key: String,
    #[serde(default)]
    action: input::KeyAction,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
struct Wait {
    ms: u64,
}

#[derive(Deserialize)]
struct BatchArgs {
    steps: Vec<RawBatchStep>,
}

#[derive(Deserialize)]
struct RawBatchStep {
    tool: String,
    #[serde(default)]
    args: Value,
}

pub struct BatchStep {
    tool: String,
    action: Action,
}

pub enum Action {
    Displays,
    Screenshot {
        display: Option<u32>,
        window: Option<u32>,
        max_size: u32,
        format: screen::ImageFormat,
    },
    Windows(windows::WindowList),
    Window(windows::WindowAction),
    Ui(windows::UiAction),
    Mouse(input::Mouse),
    Key(input::Chord),
    Type(String),
    OpenApp(String),
    Wait(std::time::Duration),
    Batch(Vec<BatchStep>),
}

impl Action {
    /// Whether the action goes to whatever is in front (or under the
    /// pointer) rather than to a display, window or element it names.
    pub fn follows_focus(&self) -> bool {
        match self {
            Self::Mouse(_) | Self::Key(_) | Self::Type(_) => true,
            Self::Ui(ui) => ui.follows_focus(),
            Self::Batch(steps) => steps.iter().any(|step| step.action.follows_focus()),
            _ => false,
        }
    }

    pub fn parse(tool: &str, args: Value) -> Result<Self> {
        Ok(match tool {
            "computer_displays" => Self::Displays,
            "computer_screenshot" => {
                let Screenshot {
                    display_id,
                    window_id,
                    max_size,
                    format,
                } = serde_json::from_value(args)?;
                let max_size = max_size.unwrap_or(DEFAULT_SCREENSHOT_MAX_SIZE);
                if !(640..=2560).contains(&max_size) {
                    bail!("maxSize must be between 640 and 2560.");
                }
                if display_id.is_some() && window_id.is_some() {
                    bail!("Choose a displayId or windowId.");
                }
                Self::Screenshot {
                    display: display_id,
                    window: window_id,
                    max_size,
                    format,
                }
            }
            "computer_windows" => Self::Windows(serde_json::from_value(args)?),
            "computer_window" => Self::Window(windows::WindowAction::parse(args)?),
            "computer_ui" => Self::Ui(windows::UiAction::parse(args)?),
            "computer_mouse" => Self::Mouse(input::Mouse::parse(args)?),
            "computer_key" => {
                let Key { key, action } = serde_json::from_value(args)?;
                Self::Key(input::Chord::parse(&key, action)?)
            }
            "computer_type" => {
                let Text { text } = serde_json::from_value::<Text>(args)?;
                if text.chars().count() > 4000
                    || text
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
                {
                    bail!(
                        "Enter up to 4000 characters of text. Only tab and newline control characters are supported."
                    );
                }
                Self::Type(text)
            }
            "computer_open_app" => {
                let OpenApp { bundle_id } = serde_json::from_value::<OpenApp>(args)?;
                if bundle_id.len() > 255
                    || !bundle_id.contains('.')
                    || !bundle_id.split('.').all(|part| {
                        part.starts_with(|c: char| c.is_ascii_alphanumeric())
                            && part
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    })
                {
                    bail!("Provide a macOS bundle identifier, for example com.apple.calculator.");
                }
                Self::OpenApp(bundle_id)
            }
            "computer_batch" => Self::parse_batch(args)?,
            _ => bail!("Unknown Computer Use tool {tool}."),
        })
    }

    fn parse_batch(args: Value) -> Result<Self> {
        let BatchArgs { steps: raw_steps } = serde_json::from_value(args)?;
        if raw_steps.is_empty() || raw_steps.len() > MAX_BATCH_STEPS {
            bail!("computer_batch requires between 1 and {MAX_BATCH_STEPS} steps.");
        }
        let mut steps = Vec::with_capacity(raw_steps.len());
        let mut total_wait_ms: u64 = 0;
        for (idx, raw) in raw_steps.into_iter().enumerate() {
            let step_args = if raw.args.is_null() {
                json!({})
            } else {
                raw.args
            };
            let normalized = raw.tool.strip_prefix("computer_").unwrap_or(&raw.tool);
            let action = match normalized {
                "wait" => {
                    let Wait { ms } = serde_json::from_value(step_args)
                        .with_context(|| format!("Step {} (wait) requires ms.", idx + 1))?;
                    if !(MIN_WAIT_MS..=MAX_WAIT_MS).contains(&ms) {
                        bail!(
                            "Step {} (wait): ms must be between {MIN_WAIT_MS} and {MAX_WAIT_MS}.",
                            idx + 1
                        );
                    }
                    total_wait_ms = total_wait_ms.saturating_add(ms);
                    if total_wait_ms > MAX_BATCH_WAIT_MS {
                        bail!(
                            "Total wait time in computer_batch cannot exceed {MAX_BATCH_WAIT_MS}ms."
                        );
                    }
                    Self::Wait(std::time::Duration::from_millis(ms))
                }
                "batch" => bail!("Step {}: nested computer_batch is not allowed.", idx + 1),
                other => {
                    let canonical = match other {
                        "openApp" | "open_app" => "computer_open_app",
                        "mouse" => "computer_mouse",
                        "key" => "computer_key",
                        "type" => "computer_type",
                        "ui" => "computer_ui",
                        "window" => "computer_window",
                        "windows" => "computer_windows",
                        "screenshot" => "computer_screenshot",
                        "displays" => "computer_displays",
                        _ => bail!("Step {}: unknown batch step tool {:?}.", idx + 1, raw.tool),
                    };
                    Self::parse(canonical, step_args)
                        .with_context(|| format!("Step {} ({}): invalid args", idx + 1, raw.tool))?
                }
            };
            steps.push(BatchStep {
                tool: normalized.to_string(),
                action,
            });
        }
        Ok(Self::Batch(steps))
    }
}

#[derive(Default)]
pub struct Desktop {
    // Serializes system-level input across desktop grants, including a cancelled drag's release.
    frames: Mutex<screen::Frames>,
}

#[derive(Default)]
pub struct Grant {
    revoked: AtomicBool,
    held: Mutex<input::Held>,
}

impl Grant {
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.release_input();
    }

    fn release_input(&self) {
        // If an input call owns the lock, it releases after dropping its guard.
        let mut held = match self.held.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(err)) => err.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        held.release_all();
    }

    fn input<T>(&self, action: impl FnOnce(&mut input::Held) -> Result<T>) -> Result<T> {
        let mut held = self
            .held
            .lock()
            .map_err(|_| anyhow::anyhow!("Computer Use input worker failed."))?;
        let result = self.check().and_then(|()| action(&mut held));
        drop(held);
        if self.revoked.load(Ordering::Acquire) {
            self.release_input();
        }
        result
    }

    fn check(&self) -> Result<()> {
        if self.revoked.load(Ordering::Acquire) {
            bail!("This turn's desktop access has ended.");
        }
        Ok(())
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl Desktop {
    pub fn perform(&self, turn: u64, grant: &Grant, action: Action) -> ToolReply {
        match self.execute(turn, grant, action) {
            Ok(reply) => reply,
            Err(err) => ToolReply::error(format!("{err:#}")),
        }
    }

    fn execute(&self, turn: u64, grant: &Grant, action: Action) -> Result<ToolReply> {
        match action {
            // Opening an app and waiting touch no frame and send no input: run outside the lock.
            Action::OpenApp(bundle_id) => {
                grant.check()?;
                open_app(&bundle_id, grant)?;
                grant.check()?;
                Ok(ToolReply::text(format!(
                    "Opened {bundle_id}. Use computer_ui (outline) for native controls, or computer_screenshot (with windowId) if you need to see it."
                )))
            }
            Action::Wait(duration) => {
                sleep_checked(duration, grant)?;
                Ok(ToolReply::text(format!("Waited {}ms.", duration.as_millis())))
            }
            Action::Batch(steps) => self.execute_batch(turn, grant, steps),
            other => {
                let mut frames = self
                    .frames
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Computer Use worker failed."))?;
                grant.check()?;
                let reply = match other {
                    Action::Displays => screen::displays()?,
                    Action::Screenshot {
                        window: Some(window),
                        max_size,
                        format,
                        ..
                    } => frames.capture_window(turn, window, max_size, format)?,
                    Action::Screenshot {
                        display,
                        max_size,
                        format,
                        ..
                    } => frames.capture(turn, display, max_size, format)?,
                    Action::Windows(request) => {
                        windows::list(request.include_hidden, request.pid)?
                    }
                    Action::Window(request) => request.perform(grant)?,
                    Action::Ui(request) => request.perform(grant)?,
                    Action::Mouse(mouse) => {
                        grant.input(|held| mouse.perform(turn, &frames, held, grant))?;
                        ToolReply::text("Mouse action completed.")
                    }
                    Action::Key(chord) => {
                        grant.input(|held| chord.perform(held, grant))?;
                        ToolReply::text("Keyboard action completed.")
                    }
                    Action::Type(text) => {
                        grant.input(|held| input::type_text(&text, held, grant))?;
                        ToolReply::text("Text typed.")
                    }
                    Action::OpenApp(_) | Action::Wait(_) | Action::Batch(_) => {
                        unreachable!("handled before locking frames")
                    }
                };
                grant.check()?;
                Ok(reply)
            }
        }
    }

    fn execute_batch(
        &self,
        turn: u64,
        grant: &Grant,
        steps: Vec<BatchStep>,
    ) -> Result<ToolReply> {
        let total = steps.len();
        let mut results = Vec::with_capacity(total);
        let mut last_image: Option<(String, Option<String>)> = None;
        for (idx, step) in steps.into_iter().enumerate() {
            let reply = self
                .execute(turn, grant, step.action)
                .with_context(|| format!("Step {}/{total} ({}) failed", idx + 1, step.tool))?;
            if let Some(image) = reply.image {
                last_image = Some((image, reply.image_mime));
            }
            let value = serde_json::from_str::<Value>(&reply.text)
                .unwrap_or_else(|_| Value::String(reply.text));
            results.push(json!({
                "step": idx + 1,
                "tool": step.tool,
                "result": value,
            }));
        }
        let summary = json!({
            "completed": total,
            "results": results,
        })
        .to_string();
        Ok(match last_image {
            Some((image, mime)) => {
                ToolReply::image(summary, image, mime.unwrap_or_else(|| "image/jpeg".into()))
            }
            None => ToolReply::text(summary),
        })
    }
}

fn sleep_checked(duration: std::time::Duration, grant: &Grant) -> Result<()> {
    let deadline = std::time::Instant::now() + duration;
    loop {
        grant.check()?;
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(());
        }
        std::thread::sleep((deadline - now).min(std::time::Duration::from_millis(50)));
    }
}

/// How long `open -b` may take before the call gives up on it.
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn open_app(bundle_id: &str, grant: &Grant) -> Result<()> {
    let mut child = std::process::Command::new("/usr/bin/open")
        .args(["-b", bundle_id])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Could not open the app.")?;
    let deadline = std::time::Instant::now() + OPEN_TIMEOUT;
    let stop_child = |child: &mut std::process::Child| {
        if let Err(err) = child.kill() {
            log::warn!("Computer Use: could not stop a stuck open: {err}");
        }
        if let Err(err) = child.wait() {
            log::warn!("Computer Use: could not reap a stuck open: {err}");
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait().context("Could not open the app.")? {
            break status;
        }
        if let Err(err) = grant.check() {
            stop_child(&mut child);
            return Err(err);
        }
        if std::time::Instant::now() >= deadline {
            stop_child(&mut child);
            bail!(
                "macOS did not open {bundle_id} in time. Take a screenshot: it may be asking for confirmation."
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    if !status.success() {
        bail!(
            "Could not open {bundle_id}. Check the bundle ID and whether the application is installed."
        );
    }
    Ok(())
}

pub fn request_screen_permission() {
    screen::request_permission();
}

/// The Accessibility grant covers both the Accessibility API (windows and
/// controls) and posting input; macOS tracks them as separate services, so
/// ask for each, one prompt at a time: event posting once the API is trusted.
pub fn request_input_permission() {
    if ax::request_permission() {
        input::request_permission();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_requires_a_turn_credential_and_revocation_stops_queued_actions() {
        let server = DesktopServer {
            socket: Path::new("/unused.sock"),
            turn: "",
        };
        assert!(
            server
                .tools()
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| { tool["name"] == "computer_screenshot" })
        );
        assert!(!server.call("computer_displays", &json!({})).ok);

        // What waits on the approval is replayed only if focus cannot misdirect it.
        let parse = |tool, args| Action::parse(tool, args).unwrap();
        assert!(!parse("computer_displays", json!({})).follows_focus());
        assert!(!parse("computer_screenshot", json!({})).follows_focus());
        assert!(
            !parse(
                "computer_open_app",
                json!({"bundleId": "com.apple.calculator"})
            )
            .follows_focus()
        );
        // Apple's own Image Capture uses an underscore in its bundle ID.
        assert!(
            Action::parse(
                "computer_open_app",
                json!({"bundleId": "com.apple.Image_Capture"})
            )
            .is_ok()
        );
        assert!(parse("computer_type", json!({"text": "hi"})).follows_focus());
        assert!(parse("computer_key", json!({"key": "Return"})).follows_focus());
        assert!(parse("computer_ui", json!({"action": "outline"})).follows_focus());
        assert!(!parse("computer_ui", json!({"windowId": 5, "action": "outline"})).follows_focus());
        assert!(
            parse(
                "computer_ui",
                json!({"windowId": 5, "action": "control", "element": "0:abc", "verb": "click"})
            )
            .follows_focus()
        );
        assert!(
            !parse(
                "computer_ui",
                json!({"windowId": 5, "action": "control", "element": "0:abc", "verb": "press"})
            )
            .follows_focus()
        );

        let grant = Grant::default();
        grant.revoke();
        let reply = Desktop::default().perform(1, &grant, Action::Displays);
        assert!(!reply.ok);
        assert!(reply.text.contains("access has ended"));
    }

    #[test]
    fn batch_validates_steps_wait_limits_and_focus_tracking() {
        let ok = Action::parse(
            "computer_batch",
            json!({
                "steps": [
                    {"tool": "open_app", "args": {"bundleId": "com.apple.calculator"}},
                    {"tool": "wait", "args": {"ms": 200}},
                    {"tool": "computer_screenshot", "args": {"format": "png", "maxSize": 1000}}
                ]
            }),
        )
        .unwrap();
        assert!(!ok.follows_focus());

        let focused = Action::parse(
            "computer_batch",
            json!({
                "steps": [
                    {"tool": "wait", "args": {"ms": 100}},
                    {"tool": "type", "args": {"text": "hello"}},
                    {"tool": "key", "args": {"key": "Return"}}
                ]
            }),
        )
        .unwrap();
        assert!(focused.follows_focus());

        // Reject empty, nested, invalid wait, and excessive total wait.
        assert!(Action::parse("computer_batch", json!({"steps": []})).is_err());
        assert!(
            Action::parse(
                "computer_batch",
                json!({"steps": [{"tool": "batch", "args": {"steps": []}}]})
            )
            .is_err()
        );
        assert!(
            Action::parse(
                "computer_batch",
                json!({"steps": [{"tool": "wait", "args": {"ms": 10}}]})
            )
            .is_err()
        );
        assert!(
            Action::parse(
                "computer_batch",
                json!({
                    "steps": [
                        {"tool": "wait", "args": {"ms": 5000}},
                        {"tool": "wait", "args": {"ms": 5000}},
                        {"tool": "wait", "args": {"ms": 5000}},
                        {"tool": "wait", "args": {"ms": 1000}}
                    ]
                })
            )
            .is_err()
        );
    }

    #[test]
    fn batch_runs_waits_and_stops_immediately_when_revoked() {
        let grant = Grant::default();
        let action = Action::parse(
            "computer_batch",
            json!({"steps": [{"tool": "wait", "args": {"ms": 60}}]}),
        )
        .unwrap();
        let reply = Desktop::default().perform(1, &grant, action);
        assert!(reply.ok, "{}", reply.text);
        let parsed: Value = serde_json::from_str(&reply.text).unwrap();
        assert_eq!(parsed["completed"], 1);

        grant.revoke();
        let action = Action::parse(
            "computer_batch",
            json!({"steps": [{"tool": "wait", "args": {"ms": 5000}}]}),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let reply = Desktop::default().perform(1, &grant, action);
        assert!(!reply.ok);
        assert!(start.elapsed() < std::time::Duration::from_millis(200));
    }
}
