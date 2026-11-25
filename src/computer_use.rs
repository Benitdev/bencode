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
        "Use these tools for native macOS apps. The user approves desktop access for this turn.          Open an app by bundle ID or use Spotlight with Cmd+Space. Capture the display before          interacting and after any change you need to verify. Mouse coordinates are pixels in          the returned screenshot (origin at the top left), and the call needs its screenshotId.          Key presses and text go to the foreground window. Use the in-app browser tools for web pages."
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
                "description": "Capture a display or a single window as PNG. Display capture returns a screenshotId for mouse actions. Window images are for inspection. Defaults to the main display.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "displayId": { "type": "integer", "minimum": 1 },
                        "windowId": { "type": "integer", "minimum": 1, "description": "Capture this window instead of a display." },
                        "maxSize": { "type": "integer", "minimum": 640, "maximum": 2560, "default": 1600,
                            "description": "Max pixel length of the longest side." }
                    }
                }
            },
            {
                "name": "computer_mouse",
                "description": "Move, click, hold (down), release (up), drag, or scroll using screenshot pixel coordinates. Move while a button is held to drag across calls. For a single-call drag, toScreenshotId can target another display. Up can omit coordinates and release at the current pointer.",
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
                        "toScreenshotId": { "type": "integer", "description": "Defaults to screenshotId; capture both displays before dragging between them." },
                        "toX": { "type": "number", "minimum": 0 },
                        "toY": { "type": "number", "minimum": 0 },
                        "deltaX": { "type": "integer", "minimum": -2000, "maximum": 2000, "description": "Positive scrolls right." },
                        "deltaY": { "type": "integer", "minimum": -2000, "maximum": 2000, "description": "Positive scrolls down." },
                        "modifiers": { "type": "array", "uniqueItems": true,
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
                "description": "Read a window’s native Accessibility outline or use a ref from the outline to press, focus, increment, decrement or set text on a control.",
                "inputSchema": {
                    "type": "object", "required": ["windowId", "action"],
                    "properties": {
                        "windowId": { "type": "integer", "minimum": 1 },
                        "action": { "type": "string", "enum": ["outline", "control"] },
                        "element": { "type": "string", "description": "Element ref from the current outline. Required for control." },
                        "verb": { "type": "string", "enum": ["press", "focus", "increment", "decrement", "setText"], "description": "Required for control." },
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
            }) => Reply::text_with_image(text, image, "image/png"),
            Ok(ToolReply { ok: true, text, .. }) => Reply::text(text),
            Ok(ToolReply {
                ok: false, text, ..
            }) => Reply::error(text),
            Err(err) => Reply::error(format!("Could not reach BenCode for Computer Use: {err:#}")),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Screenshot {
    display_id: Option<u32>,
    window_id: Option<u32>,
    max_size: Option<u32>,
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

pub enum Action {
    Displays,
    Screenshot {
        display: Option<u32>,
        window: Option<u32>,
        max_size: u32,
    },
    Windows(windows::WindowList),
    Window(windows::WindowAction),
    Ui(windows::UiAction),
    Mouse(input::Mouse),
    Key(input::Chord),
    Type(String),
    OpenApp(String),
}

impl Action {
    /// Whether the action goes to whatever is in front (or under the
    /// pointer) rather than to a display, window or element it names.
    pub fn follows_focus(&self) -> bool {
        matches!(self, Self::Mouse(_) | Self::Key(_) | Self::Type(_))
    }

    pub fn parse(tool: &str, args: Value) -> Result<Self> {
        Ok(match tool {
            "computer_displays" => Self::Displays,
            "computer_screenshot" => {
                let Screenshot {
                    display_id,
                    window_id,
                    max_size,
                } = serde_json::from_value(args)?;
                let max_size = max_size.unwrap_or(1600);
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
                            && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                {
                    bail!("Provide a macOS bundle identifier, for example com.apple.calculator.");
                }
                Self::OpenApp(bundle_id)
            }
            _ => bail!("Unknown Computer Use tool {tool}."),
        })
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
        let mut frames = self
            .frames
            .lock()
            .map_err(|_| anyhow::anyhow!("Computer Use worker failed."))?;
        grant.check()?;
        let reply = match action {
            Action::Displays => screen::displays()?,
            Action::Screenshot {
                window: Some(window),
                max_size,
                ..
            } => screen::capture_window(window, max_size)?,
            Action::Screenshot {
                display, max_size, ..
            } => frames.capture(turn, display, max_size)?,
            Action::Windows(request) => windows::list(request.include_hidden, request.pid)?,
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
            Action::OpenApp(bundle_id) => {
                let status = std::process::Command::new("/usr/bin/open")
                    .args(["-b", &bundle_id])
                    .output()
                    .context("Could not open the app.")?;
                if !status.status.success() {
                    bail!(
                        "Could not open {bundle_id}. Check the bundle ID and whether the application is installed."
                    );
                }
                ToolReply::text(format!(
                    "Opened {bundle_id}. Take a screenshot to check its window."
                ))
            }
        };
        grant.check()?;
        Ok(reply)
    }
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
        assert!(parse("computer_type", json!({"text": "hi"})).follows_focus());
        assert!(parse("computer_key", json!({"key": "Return"})).follows_focus());

        let grant = Grant::default();
        grant.revoke();
        let reply = Desktop::default().perform(1, &grant, Action::Displays);
        assert!(!reply.ok);
        assert!(reply.text.contains("access has ended"));
    }
}
