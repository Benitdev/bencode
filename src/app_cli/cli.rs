//! The CLI's side of `bencode app` (MonoCode `control_cli.rs` in app mode,
//! its `APP_USAGE`):
//! parses `ACTION [--json JSON | --input FILE|-] [--request-id ID]`, sends
//! the call to the app and prints its one-line JSON answer. The exit code is
//! 0 only when the answer is `ok`.

use std::io::Read;
use std::path::Path;

use bencode_core::harness::{APP_SOCKET_ENV, APP_TOKEN_ENV};
use serde_json::{Value, json};

use super::server::MAX_REQUEST;

const USAGE: &str = r#"BenCode app access — for a Mono's own conversation and its habits' runs.

Usage: {cli} ACTION [--json JSON | --input FILE|-] [--request-id ID]

A Mono works on several projects: add "project":"<path or name>" to the
sessions.*, worktrees.* and folders.* actions to choose which one. It may be
left out when the Mono has a single project.

Actions:
  models.list    {}  Available providers, models, settings and permission modes.
  sessions.list  {}  Project sessions with IDs, busy status, hasDraft and archived.
  sessions.read  {"sessionId":"...","before":"<turnId>","limit":3,"maxChars":1200}
                  Read up to 3 recent user/assistant exchanges. Tools and
                  reasoning are omitted. Omit before for the newest page;
                  pass nextBefore from a result for older exchanges. maxChars
                  caps each message (200-6000, default 1200).
  sessions.send  {"sessionId":"...","prompt":"...","notifyOnComplete":true}
                  Submit a follow-up to an idle session in this project.
                  A busy session is rejected. Reuse --request-id on retries.
                  Optional notifyOnComplete:true asks for a completion report
                  in the calling Mono's chat. It waits until that Mono is idle.
  sessions.draft {"sessionId":"...","prompt":"..."}
                  Save an unsent draft in an idle project session. Existing
                  drafts are preserved; send or remove one in BenCode first.
                  Reuse --request-id on retries.
  sessions.start {"prompt":"...","harness":"codex","model":"codex:...",
                  "effort":"high","workspaceMode":"current",
                  "worktreeCwd":"<path>","draft":false}
                  Start a session in the background with the prompt. Set
                  draft:true to save the prompt unsent; no agent turn runs.
                  Otherwise the turn is submitted.
                  Submitted sessions notify the calling Mono by default when
                  this turn completes, fails or is cancelled. The Mono reviews
                  it and reports back once idle. Set notifyOnComplete:false
                  when the user asks not to receive a report. Drafts do not
                  notify; notifyOnComplete:true cannot be combined with draft:true.
                  Sessions monitored during the same Mono turn form one group:
                  their results arrive together after every session stops.
                  The Mono reviews the whole group and gives one combined report.
                  Rejected launches or follow-ups return a CLI error without
                  a later completion report. When a Mono successfully stops,
                  archives or deletes a monitored session, its pending report
                  for that session is dismissed; acknowledge the action in
                  the current reply. Other sessions' reports are kept.
                  Returns after creation/acceptance, not agent completion;
                  use its ID with folders.move immediately. Optional model,
                  effort, modelSettings, permission mode and workspace choice
                  use composer values. Set worktreeCwd to a path from
                  worktrees.list to choose a specific existing checkout, or
                  workspaceMode:"worktree" and optional worktreeBase to make
                  a new worktree with an automatic branch name. Omit
                  runtimeMode to inherit this session's permission mode; set
                  it to override. Run models.list for allowed IDs. cwd is
                  your project; no attachments.
  sessions.stop {"sessionId":"..."}
                  Stop a session's current turn and pause its queued messages.
                  The conversation and checkout are kept. Idle sessions are
                  unchanged. Reuse --request-id on retries.
  sessions.archive {"sessionId":"..."}
                  Stop the session if running, save its conversation, and
                  archive it. It can be restored from BenCode's archive.
                  Reuse --request-id on retries.
  sessions.delete {"sessionId":"..."}
                  Stop the session if running and permanently delete its
                  saved conversation. Worktrees are kept.
                  Reuse --request-id on retries.
                  stop, archive and delete cannot target the calling session,
                  Mono chats, habit runs or orchestration workers. Sessions
                  must belong to the chosen project.
  worktrees.list {}  Working copies in this project, with paths and branches.
  worktrees.create {"branch":"feature/name","base":"HEAD","existing":false}
                  Create a worktree on a named new branch from base (a branch
                  or ref). Set existing:true and omit base to use an existing
                  local branch. Pass the returned path as sessions.start's
                  worktreeCwd to start there.
  folders.list   {}  Folders in your current project.
  folders.move   {"sessionId":"...","folderId":"..."}
                  Or use "newFolderName":"Research" to create a folder.
  notes.list     {"limit":30,"offset":0}  Titles and short previews only.
  notes.read     {"id":"..."}  Full body of one note.
  notes.write    {"title":"Plan","body":"Markdown","tags":["work"]}
                  Create a note linked to this session and project. Omit title
                  to derive it from the body. Use {"id":"...","body":"..."}
                  to edit an existing note; title and tags are also optional.
                  Omitted fields stay unchanged. Reuse --request-id on retries.
  artifacts.list {"kind":"document","limit":30,"offset":0}
                  Mono or habit only. Saved artifact titles, newest first.
  artifacts.read {"id":"..."}  Full content of one artifact.
  artifacts.write {"kind":"document","title":"PR review",
                   "summary":"Merge queue and blockers",
                   "body":"<complete Markdown>"}
                  Save a document and attach its card below your chat reply.
                  kind "html" saves a visual instead: a complete,
                  self-contained HTML page the user opens beside the chat.
                  Artifacts are separate from Notes. Reply briefly; do not
                  repeat the body in chat. Returns metadata only. To revise,
                  pass {"id":"...","kind":"...","body":"<updated content>"};
                  an omitted title stays unchanged. Reuse --request-id on retries.
  soul.read      {}  Mono's own conversation only. Current SOUL.md text and hash.
  soul.update    {"text":"<complete Markdown>","expectedHash":"<hash from soul.read>"}
                  Update your standing instructions only when the user asks.
                  Preserve the other instructions. If the file changed since
                  soul.read, read it again and reapply the requested changes.
                  Habit runs and other sessions cannot change a Mono's soul.
  memory.read    {"topic":"releases"}  Mono only. Without topic:
                  MEMORY.md, how much of it loads, and the topic names.
  memory.search  {"query":"release tags","since":"7d"}
                  Entries across MEMORY.md, topic notes and the archive that
                  share words with query, best first. since is a date or a
                  span (24h, 7d, 2w) and keeps dated entries from then on.
  memory.add     {"fact":"...","topic":"releases","until":"2026-11-01"}
                  Add one dated entry to MEMORY.md, or to a topic file when
                  topic is set. until is optional, for facts that expire.
                  Oldest entries move to the archive when MEMORY.md is full.
  memory.replace {"find":"text of the old entry","fact":"...","topic":"..."}
                  Strike the one entry containing find through and add fact.
  memory.remove  {"find":"text of the entry","topic":"..."}
                  Delete the one entry containing find, for a wrong entry.
  habits.list    {}  Mono only. Your habits: what each does, when it runs
                  next, and how its last run went.
  habits.add     {"name":"Morning CI check","instructions":"...",
                  "schedule":{"kind":"weekdays","time":"09:00"}}
                  Add only after the user agreed to it in this chat. kind is
                  hourly (with "minute"), daily, weekdays or weekly (with
                  "dayOfWeek", 0 = Sunday); time is local 24-hour HH:MM.
                  Each run is a hidden session that posts to this chat only
                  when it has something worth saying.
  habits.update  {"id":"...","name":"...","instructions":"...",
                  "schedule":{...},"enabled":false}  Change or pause one.
  habits.run     {"id":"..."}  Run one within a minute, to try it out.
  habits.remove  {"id":"..."}
  board.list     {}  Mono only. Your board: each task's id, title, project,
                  column (todo, running, review, done) and session.
  board.add      {"title":"Fix the login redirect","prompt":"...",
                  "project":"..."}
                  Add a task to To do. prompt is the full brief its session
                  gets (the title when omitted). Pass "sessionId" instead to
                  put a session already started on the board.
                  Reuse --request-id on retries.
  board.start    {"id":"...","workspaceMode":"worktree"}
                  Start the task's session with its prompt and project and
                  link the two; it takes sessions.start's options (harness,
                  model, effort, runtimeMode, workspace, notifyOnComplete).
                  Its column then follows the session: Running while it
                  works, Review once it stops. Reuse --request-id on retries.
  board.update   {"id":"...","done":true}  Mark a reviewed task done, or
                  done:false to reopen it. title changes it; prompt only
                  before the task has started.
  board.remove   {"id":"..."}
  chat.card      Mono or habit only. Post a card to the Mono's chat:
                  {"type":"pr","repo":"owner/repo","number":123,"note":"..."}
                  {"type":"session","sessionId":"...","note":"..."}
                  {"type":"choices","options":["First choice","Second choice"]}
                  {"type":"habit","name":"...","instructions":"...",
                   "schedule":{"kind":"daily","time":"09:00"}}

The output is one JSON line: {"ok":true,"result":...} or {"ok":false,"error":"..."}.
Input must be a JSON object; unknown fields are rejected rather than ignored.
--json takes it inline, --input FILE reads a file, --input - reads stdin.
Keep the same --request-id when retrying a call after an uncertain result.
BenCode sets BENCODE_APP_SOCKET and BENCODE_APP_TOKEN for the turn only.
They are already in your environment; never print them.
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
    if !super::input::is_action(action) {
        return Err(format!(
            "Unknown action: {action}. Use one of: {}. Run app --help.",
            super::input::actions().join(", ")
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
        let value = args.get(index + 1).ok_or_else(|| {
            format!("Missing value for {flag}. Run app --help for the argument list.")
        })?;
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
        assert!(parse_args(&args(&["sessions.nope"])).is_err());
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
