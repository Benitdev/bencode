//! `bencode --browser-mcp <socket>`: the MCP server the agent CLIs start
//! (stdio, newline-delimited JSON-RPC). It knows the tools; each call goes
//! to the app over `socket` (`bridge.rs`) and acts on the open browser tab.
//! Its stdout is the protocol, so it logs nothing there.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use super::bridge::{self, ToolRequest};

const PROTOCOL_VERSION: &str = "2025-06-18";

pub fn run(socket: &Path) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle(socket, &line) else {
            continue;
        };
        let written = serde_json::to_vec(&reply)
            .map_err(std::io::Error::other)
            .and_then(|mut out| {
                out.push(b'\n');
                stdout.write_all(&out)?;
                stdout.flush()
            });
        if written.is_err() {
            break;
        }
    }
    0
}

/// The answer to one message; `None` for notifications.
fn handle(socket: &Path, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {err}") },
            }));
        }
    };
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": super::MCP_SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": "These tools drive the browser tab open in BenCode, the app the user is \
                talking to you from; the user sees the same page. Use them to check a web app you \
                are working on: browser_navigate to its address (localhost dev servers work), \
                browser_snapshot to read the page and number its elements, then browser_click / \
                browser_type with those numbers, browser_screenshot to see it, browser_console for \
                its errors.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call_tool(socket, &params)),
        _ => Err(json!({ "code": -32601, "message": format!("unknown method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn call_tool(socket: &Path, params: &Value) -> Value {
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let reply = bridge::call(socket, &ToolRequest { tool, args })
        .unwrap_or_else(|err| bridge::ToolReply::error(format!("{err:#}")));
    let mut content = vec![json!({ "type": "text", "text": reply.text })];
    if let Some(image) = reply.image {
        content.push(json!({ "type": "image", "data": image, "mimeType": "image/png" }));
    }
    json!({ "content": content, "isError": !reply.ok })
}

fn element_args() -> Value {
    json!({
        "ref": { "type": "integer", "description": "The element's number from browser_snapshot." },
        "selector": { "type": "string", "description": "A CSS selector, when there is no number." },
    })
}

fn tools() -> Value {
    let mut type_args = element_args();
    type_args["text"] = json!({ "type": "string", "description": "The text to put in the field (replaces its value)." });
    type_args["submit"] =
        json!({ "type": "boolean", "description": "Press Enter after typing (submits a form)." });
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
            "name": "browser_history",
            "description": "Go back, forward, or reload the page.",
            "inputSchema": {
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["back", "forward", "reload"] } },
                "required": ["action"],
            },
        },
        {
            "name": "browser_snapshot",
            "description": "Read the page: its title, address, visible text, and its interactive elements numbered for browser_click and browser_type. Numbers change with every snapshot.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "browser_screenshot",
            "description": "A screenshot of what the browser tab shows.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "browser_click",
            "description": "Click an element, by its number from browser_snapshot or a CSS selector.",
            "inputSchema": { "type": "object", "properties": element_args() },
        },
        {
            "name": "browser_type",
            "description": "Type into a field, by its number from browser_snapshot or a CSS selector.",
            "inputSchema": { "type": "object", "properties": type_args, "required": ["text"] },
        },
        {
            "name": "browser_press_key",
            "description": "Press a key on the focused element: Enter, Escape, Tab, ArrowDown, … The page's key handlers get it; Enter also submits the form or clicks the focused button, and Tab moves focus. To put text in a field use browser_type.",
            "inputSchema": {
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"],
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
            "name": "browser_console",
            "description": "The page's console messages and uncaught errors since it loaded.",
            "inputSchema": {
                "type": "object",
                "properties": { "clear": { "type": "boolean", "description": "Empty the list after reading it." } },
            },
        },
        {
            "name": "browser_wait",
            "description": "Wait until some text shows on the page, or for a number of milliseconds.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "ms": { "type": "integer", "description": "How long to wait at most (default 10000)." },
                },
            },
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(line: &str) -> Option<Value> {
        handle(Path::new("/nonexistent/bencode.sock"), line)
    }

    #[test]
    fn initialize_echoes_the_clients_version() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)
            .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "bencode-browser");
    }

    #[test]
    fn notifications_get_no_answer() {
        assert!(ask(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn tools_are_listed_with_schemas() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert!(tools.iter().any(|t| t["name"] == "browser_navigate"));
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

    #[test]
    fn unknown_methods_are_errors() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }
}
