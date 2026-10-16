//! What the app and `bencode-host` say to each other: one JSON object per
//! line, over the stdio of `ssh <machine> bencode-host connect` (MonoCode's
//! `connections/model/protocol.ts` travels as HTTP to the host's `/rpc`;
//! BenCode rides SSH, which authenticates, so there are no device tokens).
//!
//! The app sends `ClientFrame`s, each with an id the reply repeats. The host
//! answers with `ServerFrame::Reply` and pushes a run's events, in order and
//! numbered, to every connection attached to it (`ServerFrame::Event`).
//!
//! Bytes travel as base64 (`B64`); paths are host paths, never `remote://`.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::harness::{AgentEvent, Attachment, HarnessKind, PermissionPolicy, SpawnRequest};

/// Bumped when a frame changes meaning; a host on another one is refused
/// (MonoCode `HOST_PROTOCOL_VERSION`).
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest line either side accepts. A file read is capped below it
/// (`MAX_READ_BYTES`), as base64 and JSON grow it.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// The most `read_file` returns (MonoCode reads and edits up to 1 MiB of
/// text remotely; the editor's own cap decides, this bounds a frame).
pub const MAX_READ_BYTES: u64 = 32 * 1024 * 1024;

/// The most an uploaded attachment may hold (MonoCode: 20 MiB a file).
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

/// What the host runs for `Request::Exec`: the git and GitHub CLIs the
/// app's views already drive, nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program {
    Git,
    Gh,
}

impl Program {
    pub fn binary(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Gh => "gh",
        }
    }
}

/// Bytes as base64 in JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct B64(pub Vec<u8>);

impl Serialize for B64 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for B64 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map(B64)
            .map_err(serde::de::Error::custom)
    }
}

/// `SpawnRequest` as it travels: no account profile (the host's CLIs run
/// under the host's own sign-in) and no browser tools (the browser is on
/// this computer). Attachments carry host paths (`Request::Upload`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnWire {
    pub harness: HarnessKind,
    pub cwd: String,
    pub prompt: String,
    pub model: Option<String>,
    pub permission: PermissionPolicy,
    pub resume_id: Option<String>,
    pub disable_hooks: bool,
    pub attachments: Vec<Attachment>,
    pub plan: bool,
    pub compact: bool,
    pub settings: std::collections::BTreeMap<String, String>,
}

impl SpawnWire {
    /// `req` with `cwd` (a host path) and the attachments uploaded to the host.
    pub fn new(req: &SpawnRequest, cwd: String, attachments: Vec<Attachment>) -> Self {
        Self {
            harness: req.harness,
            cwd,
            prompt: req.prompt.clone(),
            model: req.model.clone(),
            permission: req.permission,
            resume_id: req.resume_id.clone(),
            disable_hooks: req.disable_hooks,
            attachments,
            plan: req.plan,
            compact: req.compact,
            settings: req.settings.clone(),
        }
    }

    /// The request the host's `harness::spawn` runs.
    pub fn into_request(self) -> SpawnRequest {
        SpawnRequest {
            harness: self.harness,
            cwd: self.cwd,
            prompt: self.prompt,
            model: self.model,
            permission: self.permission,
            resume_id: self.resume_id,
            disable_hooks: self.disable_hooks,
            attachments: self.attachments,
            plan: self.plan,
            compact: self.compact,
            settings: self.settings,
            account: None,
            browser_mcp: None,
            app: None,
        }
    }
}

/// Everything the app asks of the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum Request {
    /// First on every connection: the host answers with `HostInfo`, and
    /// refuses anything else until it has been said.
    #[serde(rename_all = "camelCase")]
    Hello { protocol: u32, client: String },
    /// The agent CLIs installed on the host (`Vec<HarnessWire>`).
    Harnesses,
    /// A CLI's live model list (`Vec<ModelOption>`).
    Models { harness: HarnessKind },
    /// Runs `git` or `gh` (`ExecOutput`).
    #[serde(rename_all = "camelCase")]
    Exec {
        program: Program,
        args: Vec<String>,
        cwd: Option<String>,
        stdin: Option<B64>,
        /// Only `GIT_*` names are honoured.
        env: Vec<(String, String)>,
        timeout_ms: Option<u64>,
    },
    /// A directory's entries (`Vec<EntryWire>`), unsorted.
    ReadDir { path: String },
    /// Up to `max` bytes of a file (`FileWire`).
    ReadFile { path: String, max: u64 },
    /// What is at `path` (`Option<MetaWire>`), without following a link.
    Stat { path: String },
    /// Writes a file whole, through a temporary file and a rename. With
    /// `create_new` it fails when the file exists.
    #[serde(rename_all = "camelCase")]
    WriteFile {
        path: String,
        bytes: B64,
        create_new: bool,
    },
    CreateDir { path: String },
    Rename { from: String, to: String },
    /// Removes a file, or a directory and everything in it.
    Remove { path: String },
    /// Copies a file, or a directory and everything in it.
    Copy { from: String, to: String },
    /// An Explorer change or listing, `tree::*` on the host (see `TreeOp`).
    Tree { root: String, op: TreeOp },
    /// `files::count_text_lines` of each of `files`, relative to `root`
    /// (`Vec<usize>`, in order).
    #[serde(rename_all = "camelCase")]
    CountLines {
        root: String,
        files: Vec<String>,
        max_bytes: u64,
    },
    /// `files::list_project_files` on the host (`Vec<String>`).
    ProjectFiles { root: String },
    /// `search::search` on the host (`SearchResult`).
    Search {
        root: String,
        options: crate::search::SearchOptions,
    },
    /// Keeps an attachment on the host; answers its host path (`String`).
    Upload { name: String, bytes: B64 },
    /// Starts an agent turn under the id the app made for it (`valid_run_id`),
    /// so it follows the run before the first event arrives. Its events are
    /// pushed to this connection from the first.
    RunStart { run: String, spawn: Box<SpawnWire> },
    /// Follows a run again, from the event after `after` (0: all of them):
    /// on a new connection, or after the app restarted (`RunAttached`).
    RunAttach { run: String, after: u64 },
    /// A line for the CLI's stdin (`bool`: whether it went).
    RunStdin { run: String, line: String },
    /// The user's answer to a permission prompt, which the host's handle
    /// puts in the harness' own words (`bool`: whether it could).
    RunPermission {
        run: String,
        request: crate::harness::PermissionRequest,
        allow: bool,
    },
    /// A follow-up for the running turn (`bool`: whether it was taken).
    RunSteer {
        run: String,
        prompt: String,
        attachments: Vec<Attachment>,
    },
    /// Stops the CLI.
    RunCancel { run: String },
    /// The app has everything; the host may forget the run once it ended.
    RunForget { run: String },
    /// The runs the host still keeps (`Vec<RunSummary>`).
    Runs,
    /// Stops the host once this reply is out: Update Host restarts it.
    Shutdown,
}

/// What `Request::Tree` does under its `root`, with workspace-relative
/// paths; the answer is what the `tree` function returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum TreeOp {
    /// `Vec<FsEntry>`.
    List { rel: String },
    /// The new entry's relative path.
    #[serde(rename_all = "camelCase")]
    Create {
        parent: String,
        name: String,
        is_dir: bool,
    },
    Rename { rel: String, name: String },
    Delete { rel: String },
    #[serde(rename_all = "camelCase")]
    MoveInto { rel: String, dest_parent: String },
    /// `from` is an absolute path on the host.
    #[serde(rename_all = "camelCase")]
    CopyInto { from: String, dest_parent: String },
}

/// The host's answer to `Hello` (MonoCode `HostDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub protocol: u32,
    /// `bencode-host --version`.
    pub version: String,
    /// The host's id: the machine's part of a `remote://` path.
    pub environment_id: String,
    /// The host's computer name.
    pub name: String,
    /// `uname -s` / `uname -m`, lowercase (`linux`, `x86_64`).
    pub os: String,
    pub arch: String,
    /// The SSH account's home folder on the host.
    pub home: String,
    /// Runs going right now.
    pub running: usize,
}

/// One agent CLI on the host (`harness::HarnessInfo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessWire {
    pub harness: HarnessKind,
    pub available: bool,
    pub binary_path: Option<String>,
}

/// What `Exec` ran into. `status` is `None` when it was killed (timeout).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutput {
    pub status: Option<i32>,
    pub stdout: B64,
    pub stderr: B64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// A file or directory's metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWire {
    pub kind: EntryKind,
    /// For a link: whether it leads to a directory.
    pub target_is_dir: bool,
    pub size: u64,
    /// Milliseconds since the epoch; 0 when unknown.
    pub modified_ms: i64,
    pub readonly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryWire {
    pub name: String,
    pub meta: MetaWire,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWire {
    pub bytes: B64,
    pub meta: MetaWire,
    /// The file is longer than what came back.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAttached {
    /// The host still has this run; its events follow.
    pub known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub run: String,
    pub cwd: String,
    pub harness: HarnessKind,
    pub events: u64,
    pub ended: bool,
    pub started_ms: i64,
}

/// A request and the id its reply comes back with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientFrame {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

/// What the host writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerFrame {
    /// The answer to request `id`: `result` on success, `error` otherwise.
    Reply {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Event `seq` (from 1) of `run`.
    Event {
        run: String,
        seq: u64,
        event: AgentEvent,
    },
    /// `run` has no more events; `seq` is its last.
    RunEnded { run: String, seq: u64 },
}

impl ServerFrame {
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        match serde_json::to_value(result) {
            Ok(result) => Self::Reply {
                id,
                result: Some(result),
                error: None,
            },
            Err(err) => Self::fail(id, format!("could not encode the reply: {err}")),
        }
    }

    pub fn fail(id: u64, error: impl Into<String>) -> Self {
        Self::Reply {
            id,
            result: None,
            error: Some(error.into()),
        }
    }
}

/// Whether a run id could be one the app made: it names the host's files.
pub fn valid_run_id(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

/// Whether an `Exec` env name may reach git (`GIT_OPTIONAL_LOCKS`,
/// `GIT_INDEX_FILE`, …): nothing that changes which program runs.
pub fn exec_env_allowed(name: &str) -> bool {
    name.starts_with("GIT_")
        && !matches!(
            name,
            "GIT_EXEC_PATH" | "GIT_SSH" | "GIT_SSH_COMMAND" | "GIT_ASKPASS" | "GIT_EDITOR"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = ClientFrame {
            id: 7,
            request: Request::ReadFile {
                path: "/home/me/a.rs".into(),
                max: 10,
            },
        };
        let text = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            text,
            r#"{"id":7,"method":"readFile","params":{"path":"/home/me/a.rs","max":10}}"#
        );
        assert_eq!(serde_json::from_str::<ClientFrame>(&text).unwrap(), frame);
        let unit = serde_json::to_string(&ClientFrame {
            id: 1,
            request: Request::Harnesses,
        })
        .unwrap();
        assert_eq!(unit, r#"{"id":1,"method":"harnesses"}"#);
        assert_eq!(
            serde_json::from_str::<ClientFrame>(&unit).unwrap().request,
            Request::Harnesses
        );
    }

    #[test]
    fn bytes_travel_as_base64() {
        let out = ExecOutput {
            status: Some(0),
            stdout: B64(b"hi\0".to_vec()),
            stderr: B64::default(),
        };
        let text = serde_json::to_string(&out).unwrap();
        assert!(text.contains(r#""stdout":"aGkA""#), "{text}");
        assert_eq!(serde_json::from_str::<ExecOutput>(&text).unwrap(), out);
    }

    #[test]
    fn events_and_replies_round_trip() {
        for frame in [
            ServerFrame::Event {
                run: "r".into(),
                seq: 3,
                event: AgentEvent::TextDelta("x".into()),
            },
            ServerFrame::RunEnded {
                run: "r".into(),
                seq: 3,
            },
            ServerFrame::ok(2, vec!["a"]),
            ServerFrame::fail(2, "nope"),
        ] {
            let text = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<ServerFrame>(&text).unwrap(), frame);
        }
    }

    #[test]
    fn only_plain_git_variables_pass() {
        assert!(exec_env_allowed("GIT_OPTIONAL_LOCKS"));
        assert!(exec_env_allowed("GIT_INDEX_FILE"));
        assert!(!exec_env_allowed("GIT_SSH_COMMAND"));
        assert!(!exec_env_allowed("PATH"));
        assert!(!exec_env_allowed("LD_PRELOAD"));
    }
}
