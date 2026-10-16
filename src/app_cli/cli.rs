//! The CLI's side of `bencode app` (MonoCode `control_cli.rs` in app mode):
//! parses `ACTION [--json JSON | --input FILE|-] [--request-id ID]`, sends
//! the call to the app and prints its one-line JSON answer. The exit code is
//! 0 only when the answer is `ok`.

use std::io::Read;
use std::path::Path;

use bencode_core::harness::{APP_SOCKET_ENV, APP_TOKEN_ENV};
use serde_json::{Value, json};

use super::artifacts::ACTIONS;
use super::server::MAX_REQUEST;

const USAGE: &str = r#"BenCode app access — for a Mono's own conversation.

Usage: {cli} ACTION [--json JSON | --input FILE|-] [--request-id ID]

Actions, with the JSON object each one takes:
  artifacts.list  {"kind":"document","limit":30,"offset":0}
                  Saved artifact titles, newest first. kind is optional.
  artifacts.read  {"id":"..."}  Full content of one artifact.
  artifacts.write {"kind":"document","title":"PR review",
                   "summary":"Merge queue and blockers",
                   "body":"<complete Markdown>"}
                  Save a document and attach its card below your chat reply.
                  kind "html" saves a visual instead: a complete,
                  self-contained HTML page the user opens beside the chat.
                  Reply briefly; do not repeat the body in chat. Returns
                  metadata only. To revise, pass {"id":"...","kind":"...",
                  "body":"<updated content>"}; an omitted title stays
                  unchanged. Reuse --request-id on retries.

The output is one JSON line: {"ok":true,"result":...} or {"ok":false,"error":"..."}.
Input must be a JSON object; unknown fields are rejected rather than ignored.
--json takes it inline, --input FILE reads a file, --input - reads stdin.
Keep the same --request-id when retrying a call after an uncertain result.
BenCode sets BENCODE_APP_SOCKET and BENCODE_APP_TOKEN for the Mono's turn
only. They are already in your environment; never print them.
"#;

fn help() -> String {
    USAGE.replace("{cli}", &super::command())
}

#[derive(Debug, PartialEq)]
enum Parsed {
    Help,
    Call(String, Value, String),
}

pub fn run(args: Vec<String>) -> i32 {
    let (action, input, request_id) = match parse_args(&args) {
        Ok(Parsed::Help) => {
            println!("{}", help());
            return 0;
        }
        Ok(Parsed::Call(action, input, request_id)) => (action, input, request_id),
        Err(error) => {
            println!("{}", json!({ "ok": false, "error": error }));
            return 1;
        }
    };
    let (Ok(socket), Ok(token)) = (std::env::var(APP_SOCKET_ENV), std::env::var(APP_TOKEN_ENV))
    else {
        println!(
            "{}",
            json!({ "ok": false, "error": "No BenCode app connection. Run this from a Mono's turn in BenCode." })
        );
        return 1;
    };
    let line = json!({ "token": token, "action": action, "input": input, "requestId": request_id });
    match super::server::call(Path::new(&socket), &line) {
        Ok(reply) if reply.get("ok").and_then(Value::as_bool) == Some(true) => {
            println!("{reply}");
            0
        }
        Ok(reply) => {
            println!("{}", with_retry_hint(reply, &request_id));
            1
        }
        Err((error, sent)) => {
            // The call may have reached the app even though its answer was
            // lost: hand back the request ID so a retry cannot repeat it.
            let mut reply = json!({ "ok": false, "error": error });
            if sent {
                reply["requestId"] = json!(request_id);
                reply["retryWith"] = json!(format!("--request-id {request_id}"));
            }
            println!("{reply}");
            1
        }
    }
}

/// MonoCode `with_retry_hint`: a refused call never reached the executor,
/// and suggesting the same ID again sends agents in loops.
fn with_retry_hint(mut reply: Value, request_id: &str) -> Value {
    if reply.get("retryable").and_then(Value::as_bool) == Some(false) {
        return reply;
    }
    if let Some(object) = reply.as_object_mut() {
        object
            .entry("requestId")
            .or_insert_with(|| json!(request_id));
        object
            .entry("retryWith")
            .or_insert_with(|| json!(format!("--request-id {request_id}")));
    }
    reply
}

fn read_capped(source: impl Read) -> Result<String, String> {
    let mut raw = String::new();
    source
        .take(MAX_REQUEST as u64 + 1)
        .read_to_string(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > MAX_REQUEST {
        return Err("Input exceeds 256 KiB".into());
    }
    Ok(raw)
}

/// MonoCode `parse_args_for` in app mode.
fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let is_help = |value: &str| matches!(value, "help" | "--help" | "-h");
    let Some(action) = args.first() else {
        return Ok(Parsed::Help);
    };
    if is_help(action) {
        return Ok(Parsed::Help);
    }
    if !ACTIONS.contains(&action.as_str()) {
        return Err(format!(
            "Unknown action: {action}. Use one of: {}. Run app --help.",
            ACTIONS.join(", ")
        ));
    }
    let mut input = None;
    let mut request_id = super::random_hex(16);
    let mut index = 1;
    while index < args.len() {
        let flag = &args[index];
        if is_help(flag) {
            return Ok(Parsed::Help);
        }
        if !flag.starts_with("--") {
            return Err(format!(
                "Unexpected argument: {flag}. Pass the JSON object as --json '<JSON>'."
            ));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("Missing value for {flag}. Run app --help for the argument list."))?;
        match flag.as_str() {
            "--request-id" => request_id = value.clone(),
            "--json" | "--input" => {
                if input.is_some() {
                    return Err("Supply only one input".into());
                }
                let raw = if flag == "--json" {
                    if value.len() > MAX_REQUEST {
                        return Err("Input exceeds 256 KiB".into());
                    }
                    value.clone()
                } else if value == "-" {
                    read_capped(std::io::stdin())?
                } else {
                    read_capped(std::fs::File::open(value).map_err(|e| e.to_string())?)?
                };
                let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
                    format!(
                        "Invalid JSON: {e}. Pass one JSON object, e.g. --json '{{\"id\":\"...\"}}'."
                    )
                })?;
                if !parsed.is_object() {
                    return Err("Input must be a JSON object".into());
                }
                input = Some(parsed);
            }
            _ => {
                return Err(format!(
                    "Unknown option: {flag}. Supported: --json, --input, --request-id."
                ));
            }
        }
        index += 2;
    }
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("App request IDs may contain only letters, digits, - and _".into());
    }
    Ok(Parsed::Call(
        action.clone(),
        input.unwrap_or_else(|| json!({})),
        request_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn calls_take_json_and_a_request_id() {
        let parsed = parse_args(&args(&[
            "artifacts.read",
            "--json",
            r#"{"id":"a"}"#,
            "--request-id",
            "r-1",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Parsed::Call("artifacts.read".into(), json!({ "id": "a" }), "r-1".into())
        );
        let Parsed::Call(_, input, id) = parse_args(&args(&["artifacts.list"])).unwrap() else {
            panic!("a call");
        };
        assert_eq!(input, json!({}));
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn help_and_mistakes() {
        assert_eq!(parse_args(&args(&[])).unwrap(), Parsed::Help);
        assert_eq!(
            parse_args(&args(&["artifacts.write", "--help"])).unwrap(),
            Parsed::Help
        );
        assert!(parse_args(&args(&["sessions.start"])).is_err());
        assert!(parse_args(&args(&["artifacts.read", "x"])).is_err());
        assert!(parse_args(&args(&["artifacts.read", "--json", "[1]"])).is_err());
        assert!(parse_args(&args(&["artifacts.read", "--request-id", "a b"])).is_err());
        assert!(help().contains("artifacts.write"));
    }

    #[test]
    fn refused_calls_get_no_retry_hint() {
        let refused = json!({ "ok": false, "error": "x", "retryable": false });
        assert_eq!(with_retry_hint(refused.clone(), "r"), refused);
        let failed = with_retry_hint(json!({ "ok": false, "error": "x" }), "r");
        assert_eq!(failed["retryWith"], "--request-id r");
    }
}
