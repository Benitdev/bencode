//! An MCP server over stdio (newline-delimited JSON-RPC), as BenCode
//! offers agents its own: `bencode --browser-mcp` (`browser/mcp.rs`) and
//! `bencode --backlog-mcp` (`backlog/mcp.rs`). A server names its tools
//! and answers their calls; this answers the protocol around them. Its
//! stdout is the protocol, so nothing else may write there.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// What a tool call answers.
pub struct Reply {
    pub ok: bool,
    pub text: String,
    /// A PNG, base64.
    pub image: Option<String>,
}

impl Reply {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
            image: None,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
            image: None,
        }
    }
}

pub trait Server {
    /// The name the agent knows it by (`mcp__<name>__…`).
    fn name(&self) -> &'static str;
    /// When to use the tools, for the agent.
    fn instructions(&self) -> &'static str;
    /// The tools' names, descriptions and input schemas.
    fn tools(&self) -> Value;
    fn call(&self, tool: &str, args: &Value) -> Reply;
}

/// Answers messages on stdin until it closes; the exit code.
pub fn run(server: &impl Server) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle(server, &line) else {
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
pub fn handle(server: &impl Server, line: &str) -> Option<Value> {
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
            "serverInfo": { "name": server.name(), "version": env!("CARGO_PKG_VERSION") },
            "instructions": server.instructions(),
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": server.tools() })),
        "tools/call" => Ok(call_tool(server, &params)),
        _ => Err(json!({ "code": -32601, "message": format!("unknown method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn call_tool(server: &impl Server, params: &Value) -> Value {
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let reply = server.call(tool, &args);
    let mut content = vec![json!({ "type": "text", "text": reply.text })];
    if let Some(image) = reply.image {
        content.push(json!({ "type": "image", "data": image, "mimeType": "image/png" }));
    }
    json!({ "content": content, "isError": !reply.ok })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Server for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn instructions(&self) -> &'static str {
            "Echoes."
        }
        fn tools(&self) -> Value {
            json!([{ "name": "echo", "inputSchema": { "type": "object" } }])
        }
        fn call(&self, tool: &str, args: &Value) -> Reply {
            match tool {
                "echo" => Reply::text(args["text"].as_str().unwrap_or_default()),
                _ => Reply::error(format!("unknown tool {tool}")),
            }
        }
    }

    fn ask(line: &str) -> Option<Value> {
        handle(&Echo, line)
    }

    #[test]
    fn initialize_echoes_the_clients_version() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)
            .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "echo");
        assert_eq!(reply["result"]["instructions"], "Echoes.");
    }

    #[test]
    fn notifications_get_no_answer() {
        assert!(ask(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn calls_reach_the_server_and_its_errors_are_tool_errors() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}"#)
            .unwrap();
        assert_eq!(reply["result"]["content"][0]["text"], "hi");
        assert_eq!(reply["result"]["isError"], false);
        let reply =
            ask(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope"}}"#)
                .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }

    #[test]
    fn unknown_methods_and_bad_json_are_errors() {
        let reply = ask(r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(ask("{").unwrap()["error"]["code"], -32700);
    }
}
