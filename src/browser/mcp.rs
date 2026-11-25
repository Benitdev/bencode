//! `bencode --browser-mcp <socket>`: the MCP server the agent CLIs start
//! (stdio, `mcp/server.rs`). It knows the tools; each call goes to the app
//! over `socket` and acts on the open browser tabs.

use std::path::Path;

use serde_json::{Value, json};

use crate::mcp::bridge::{self, ToolRequest};
use crate::mcp::server::{self, Reply, Server};

pub fn run(socket: &Path) -> i32 {
    server::run(&Browser { socket })
}

struct Browser<'a> {
    socket: &'a Path,
}

impl Server for Browser<'_> {
    fn name(&self) -> &'static str {
        super::BROWSER_SERVER
    }

    fn instructions(&self) -> &'static str {
        "These tools drive the browser tabs open in BenCode, the app the user is talking to \
        you from; the user sees the same page. Use browser_navigate and browser_tabs to open \
        and switch tabs, browser_snapshot to read the page and number its interactive elements \
        (including labels and Shadow DOM), browser_click / browser_hover / browser_type / \
        browser_select / browser_fill_form / browser_press_key / browser_scroll / browser_drag \
        to interact, browser_screenshot (full page or cropped to an element) and browser_inspect \
        (DOM + computed CSS) to inspect UI, browser_console / browser_network / browser_storage \
        to debug runtime errors, API calls and state, browser_wait to wait for text/selectors/\
        URL/network idle, and browser_dialog to handle JS alerts/confirms/prompts."
    }

    fn tools(&self) -> Value {
        tools()
    }

    fn call(&self, tool: &str, args: &Value) -> Reply {
        let request = ToolRequest {
            token: String::new(),
            computer_turn: None,
            tool: tool.to_string(),
            args: args.clone(),
        };
        let reply = bridge::call(self.socket, &request)
            .unwrap_or_else(|err| bridge::ToolReply::error(format!("{err:#}")));
        Reply {
            ok: reply.ok,
            text: reply.text,
            image: reply.image,
            image_mime: None,
        }
    }
}

fn element_args() -> Value {
    json!({
        "ref": { "type": "integer", "description": "The element's number from browser_snapshot." },
        "selector": { "type": "string", "description": "A CSS selector (searches open Shadow DOM too), when there is no number." },
    })
}

fn tools() -> Value {
    let mut click_args = element_args();
    click_args["doubleClick"] =
        json!({ "type": "boolean", "description": "Perform a double-click (dblclick)." });
    click_args["button"] = json!({
        "type": "string",
        "enum": ["left", "right"],
        "description": "Mouse button (default left; right fires contextmenu).",
    });
    click_args["modifiers"] = json!({
        "type": "array",
        "items": { "type": "string", "enum": ["Shift", "Control", "Alt", "Meta"] },
        "description": "Modifier keys held during the click.",
    });

    let mut type_args = element_args();
    type_args["text"] = json!({ "type": "string", "description": "The text to put in the field (replaces its value unless append is true)." });
    type_args["append"] =
        json!({ "type": "boolean", "description": "Append to existing text instead of replacing it." });
    type_args["submit"] =
        json!({ "type": "boolean", "description": "Press Enter after typing (submits a form)." });

    let mut press_args = element_args();
    press_args["key"] = json!({ "type": "string", "description": "Key name or modifier combo: Enter, Escape, Tab, Shift+Tab, Space, ArrowDown, Control+Enter, Meta+k, …" });

    let mut select_args = element_args();
    select_args["value"] = json!({ "type": "string", "description": "Option value or visible label to select." });
    select_args["values"] = json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Multiple option values or labels (for <select multiple>).",
    });

    let mut scroll_args = element_args();
    scroll_args["direction"] = json!({
        "type": "string",
        "enum": ["down", "up", "left", "right", "top", "bottom"],
        "description": "Scroll direction (default down).",
    });
    scroll_args["amount"] = json!({ "type": "number", "description": "Pixels to scroll (default ~80% of viewport/container)." });
    scroll_args["intoView"] = json!({ "type": "boolean", "description": "Scroll the target ref/selector into view instead of scrolling inside it." });

    let mut screenshot_args = element_args();
    screenshot_args["pad"] = json!({ "type": "number", "description": "CSS pixel padding around a cropped element (default 8)." });
    screenshot_args["maxWidth"] = json!({ "type": "integer", "description": "Max image width in pixels (default 1280, max 2560)." });

    let mut inspect_args = element_args();
    inspect_args["styles"] = json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Specific CSS properties to inspect (defaults to key layout, box, typography and color properties).",
    });

    json!([
        {
            "name": "browser_navigate",
            "description": "Open an http, https or file address in BenCode's browser tab (opening the tab if none is open) and wait for it to load. Bare hosts work: localhost:3000.",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"],
            },
        },
        {
            "name": "browser_tabs",
            "description": "Manage BenCode's open browser tabs: list all tabs with their ids/titles/URLs, open a new tab, switch to a tab by id, or close a tab.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "new", "select", "close"] },
                    "id": { "type": "integer", "description": "Tab id (for select or close; defaults to current tab for close)." },
                    "url": { "type": "string", "description": "Address to open when action is new (optional)." },
                },
                "required": ["action"],
            },
        },
        {
            "name": "browser_history",
            "description": "Go back, forward, or reload the page. hard_reload fetches every file again, skipping the cache: for a dev server's page left blank or stale after it rebuilt its files.",
            "inputSchema": {
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["back", "forward", "reload", "hard_reload"] } },
                "required": ["action"],
            },
        },
        {
            "name": "browser_snapshot",
            "description": "Read the page: title, address, viewport/scroll position, visible text, and interactive elements (with labels, ARIA/form states, and open Shadow DOM) numbered for browser_click/type/hover/select/inspect. Numbers change with every snapshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Optional CSS selector to scope the snapshot to a subtree (e.g. main, form, [role=dialog])." },
                    "maxText": { "type": "integer", "description": "Max characters of visible page text (default 12000, max 40000)." },
                },
            },
        },
        {
            "name": "browser_screenshot",
            "description": "A screenshot of what the browser tab shows, or cropped to a specific element when ref or selector is given.",
            "inputSchema": { "type": "object", "properties": screenshot_args },
        },
        {
            "name": "browser_click",
            "description": "Click (or double-click / right-click) an element, by its number from browser_snapshot or a CSS selector.",
            "inputSchema": { "type": "object", "properties": click_args },
        },
        {
            "name": "browser_hover",
            "description": "Hover the mouse over an element (triggers tooltips, hover menus, dropdowns), by its number from browser_snapshot or a CSS selector.",
            "inputSchema": { "type": "object", "properties": element_args() },
        },
        {
            "name": "browser_type",
            "description": "Type into a field, by its number from browser_snapshot or a CSS selector.",
            "inputSchema": { "type": "object", "properties": type_args, "required": ["text"] },
        },
        {
            "name": "browser_select",
            "description": "Select option(s) in a <select> dropdown by value or visible label.",
            "inputSchema": { "type": "object", "properties": select_args },
        },
        {
            "name": "browser_fill_form",
            "description": "Fill multiple form fields (inputs, textareas, checkboxes, radios, selects) in a single call, with optional submit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fields": {
                        "type": "array",
                        "description": "Fields to fill: each names ref or selector, plus value/values or checked.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "ref": { "type": "integer" },
                                "selector": { "type": "string" },
                                "value": { "type": ["string", "number", "boolean"] },
                                "values": { "type": "array", "items": { "type": "string" } },
                                "checked": { "type": "boolean" },
                            },
                        },
                    },
                    "submit": { "type": "boolean", "description": "Submit the form after filling all fields." },
                },
                "required": ["fields"],
            },
        },
        {
            "name": "browser_press_key",
            "description": "Press a key or modifier combo (Enter, Escape, Tab, Shift+Tab, Space, ArrowDown, Control+Enter, Meta+k, …) on the focused element, or on ref/selector when given.",
            "inputSchema": {
                "type": "object",
                "properties": press_args,
                "required": ["key"],
            },
        },
        {
            "name": "browser_scroll",
            "description": "Scroll the page or a scrollable container by direction/amount, or scroll a target element into view (intoView: true).",
            "inputSchema": { "type": "object", "properties": scroll_args },
        },
        {
            "name": "browser_drag",
            "description": "Drag and drop from a source element (fromRef/fromSelector) to a target element (toRef/toSelector).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fromRef": { "type": "integer", "description": "Source element number from browser_snapshot." },
                    "fromSelector": { "type": "string", "description": "Source CSS selector." },
                    "toRef": { "type": "integer", "description": "Target element number from browser_snapshot." },
                    "toSelector": { "type": "string", "description": "Target CSS selector." },
                },
            },
        },
        {
            "name": "browser_evaluate",
            "description": "Run a JavaScript expression in the page and return its value (a promise is awaited).",
            "inputSchema": {
                "type": "object",
                "properties": { "expression": { "type": "string" } },
                "required": ["expression"],
            },
        },
        {
            "name": "browser_inspect",
            "description": "Inspect one element's bounding box, visibility, attributes, outerHTML and computed CSS styles.",
            "inputSchema": { "type": "object", "properties": inspect_args },
        },
        {
            "name": "browser_console",
            "description": "The page's console messages and uncaught errors since it loaded, with optional level/search/limit filtering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "level": { "type": "string", "enum": ["error", "warn", "info", "log", "debug", "errors"], "description": "Filter by level (\"errors\" includes error + warn)." },
                    "search": { "type": "string", "description": "Case-insensitive substring filter." },
                    "limit": { "type": "integer", "description": "Return only the last N matching lines." },
                    "clear": { "type": "boolean", "description": "Empty the list after reading it." },
                },
            },
        },
        {
            "name": "browser_network",
            "description": "Inspect captured fetch/XHR requests since the page loaded (method, status, duration, URL) or pass id for a single request's request/response body previews.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Specific request #id from a previous browser_network list to inspect its full request/response preview." },
                    "url": { "type": "string", "description": "Case-insensitive URL substring filter." },
                    "method": { "type": "string", "description": "HTTP method filter (GET, POST, …)." },
                    "failedOnly": { "type": "boolean", "description": "Only show requests that failed or answered HTTP >= 400." },
                    "includeResources": { "type": "boolean", "description": "Also include static PerformanceResourceTiming entries (scripts, CSS, images)." },
                    "limit": { "type": "integer", "description": "Max entries to return (default 50, max 200)." },
                    "clear": { "type": "boolean", "description": "Clear the captured fetch/XHR buffer after reading." },
                },
            },
        },
        {
            "name": "browser_storage",
            "description": "Inspect or edit localStorage / sessionStorage, or list document cookies (sensitive values are redacted unless includeValues is true).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["local", "session", "cookies"], "description": "Storage type (default local)." },
                    "action": { "type": "string", "enum": ["list", "get", "set", "remove", "clear"], "description": "Operation (default list)." },
                    "key": { "type": "string" },
                    "value": { "type": "string" },
                    "prefix": { "type": "string", "description": "Filter keys by prefix when listing." },
                    "includeValues": { "type": "boolean", "description": "Reveal cookie values or token-like storage values." },
                },
            },
        },
        {
            "name": "browser_wait",
            "description": "Wait until text appears/disappears, a selector appears/disappears, the URL contains a substring, network becomes idle, or for a number of milliseconds.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Wait until this text shows on the page." },
                    "textGone": { "type": "string", "description": "Wait until this text disappears from the page." },
                    "selector": { "type": "string", "description": "Wait until an element matching this CSS selector is visible." },
                    "selectorGone": { "type": "string", "description": "Wait until no visible element matches this CSS selector." },
                    "url": { "type": "string", "description": "Wait until location.href contains this substring." },
                    "networkIdle": { "type": "boolean", "description": "Wait until no fetch/XHR requests are in flight for 500ms." },
                    "ms": { "type": "integer", "description": "How long to wait at most (default 10000, max 60000)." },
                },
            },
        },
        {
            "name": "browser_zoom",
            "description": "Get or change the browser tab's zoom percentage and report its viewport dimensions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "percent": { "type": "integer", "description": "Set zoom percent (25 to 500)." },
                    "action": { "type": "string", "enum": ["get", "in", "out", "reset"], "description": "Step zoom in/out, reset to 100%, or get current zoom." },
                },
            },
        },
        {
            "name": "browser_dialog",
            "description": "Arm automatic handling for JavaScript alert/confirm/prompt dialogs during automation, inspect dialogs that fired, or reset back to native dialogs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["status", "accept", "dismiss", "reset"] },
                    "promptText": { "type": "string", "description": "Text to return for window.prompt when action is accept." },
                },
                "required": ["action"],
            },
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(line: &str) -> Option<Value> {
        let browser = Browser {
            socket: Path::new("/nonexistent/bencode.sock"),
        };
        server::handle(&browser, line)
    }

    #[test]
    fn introduces_itself_as_the_browser() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        assert_eq!(reply["result"]["serverInfo"]["name"], "bencode-browser");
    }

    #[test]
    fn tools_are_listed_with_schemas() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 20, "expected >= 20 tools, got {}", tools.len());
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert!(
            tools
                .iter()
                .all(|t| t["name"].as_str().unwrap().starts_with("browser_"))
        );
        for expected in [
            "browser_navigate",
            "browser_tabs",
            "browser_snapshot",
            "browser_screenshot",
            "browser_click",
            "browser_hover",
            "browser_type",
            "browser_select",
            "browser_fill_form",
            "browser_scroll",
            "browser_drag",
            "browser_inspect",
            "browser_network",
            "browser_storage",
            "browser_wait",
            "browser_zoom",
            "browser_dialog",
        ] {
            assert!(
                tools.iter().any(|t| t["name"] == expected),
                "missing tool {expected}"
            );
        }
    }

    #[test]
    fn a_call_without_the_app_is_a_tool_error() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"browser_snapshot","arguments":{}}}"#)
            .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(
            reply["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not reachable")
        );
    }
}
