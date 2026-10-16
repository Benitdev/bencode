//! The host end to end, as the app drives it: `bencode-host connect`
//! starts the daemon, a turn runs a fake Claude Code, the connection drops,
//! and a new one reads the run again. Everything lives in a temporary
//! folder; no real CLI is found (the fake one is first on PATH and the
//! daemon keeps that PATH).

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use bencode_core::harness::{AgentEvent, HarnessKind, PermissionPolicy};
use bencode_core::remote::protocol::{
    B64, ClientFrame, ExecOutput, HarnessWire, HostInfo, PROTOCOL_VERSION, Program, Request,
    RunAttached, RunSummary, ServerFrame, SpawnWire,
};

const FAKE_CLAUDE: &str = r#"#!/bin/sh
read line
echo '{"type":"system","subtype":"init","session_id":"fake-session","model":"fake"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello from the host"}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"hello from the host","session_id":"fake-session"}'
"#;

struct Client {
    child: Child,
    stdin: ChildStdin,
    frames: mpsc::Receiver<ServerFrame>,
    next_id: u64,
}

impl Client {
    fn connect(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bencode-host"))
            .arg("connect")
            .env("BENCODE_HOST_DIR", root.join("host"))
            .env("HOME", root.join("home"))
            .env("PATH", format!("{}:/usr/bin:/bin", root.join("bin").display()))
            .env("BENCODE_HOST_KEEP_PATH", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("bencode-host builds");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, frames) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let frame: ServerFrame = serde_json::from_str(&line).expect("frames parse");
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            frames,
            next_id: 0,
        };
        let info: HostInfo = client.call(Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "test".into(),
        });
        assert_eq!(info.protocol, PROTOCOL_VERSION);
        client
    }

    fn send(&mut self, request: Request) -> u64 {
        self.next_id += 1;
        let line = serde_json::to_string(&ClientFrame {
            id: self.next_id,
            request,
        })
        .unwrap();
        writeln!(self.stdin, "{line}").unwrap();
        self.next_id
    }

    fn next(&self) -> ServerFrame {
        self.frames
            .recv_timeout(Duration::from_secs(20))
            .expect("the host answers")
    }

    /// The reply to `request`; events that come first are dropped.
    fn call<T: serde::de::DeserializeOwned>(&mut self, request: Request) -> T {
        let id = self.send(request);
        loop {
            if let ServerFrame::Reply {
                id: reply,
                result,
                error,
            } = self.next()
                && reply == id
            {
                assert_eq!(error, None);
                return serde_json::from_value(result.unwrap_or_default()).unwrap();
            }
        }
    }

    /// `run`'s events until it ends.
    fn events_of(&self, run: &str) -> Vec<(u64, AgentEvent)> {
        let mut events = Vec::new();
        loop {
            match self.next() {
                ServerFrame::Event { run: r, seq, event } if r == run => events.push((seq, event)),
                ServerFrame::RunEnded { run: r, seq } if r == run => {
                    assert_eq!(seq, events.last().map_or(0, |(seq, _)| *seq));
                    return events;
                }
                _ => {}
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn turns_run_on_the_host_and_outlive_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for sub in ["home", "bin", "work"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let claude = root.join("bin/claude");
    std::fs::write(&claude, FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut client = Client::connect(root);
    let harnesses: Vec<HarnessWire> = client.call(Request::Harnesses);
    let found = harnesses
        .iter()
        .find(|h| h.harness == HarnessKind::Claude)
        .unwrap();
    assert!(found.available);
    assert_eq!(found.binary_path.as_deref(), Some(claude.to_str().unwrap()));

    let work = root.join("work").to_string_lossy().into_owned();
    let out: ExecOutput = client.call(Request::Exec {
        program: Program::Git,
        args: vec!["init".into(), "-q".into()],
        cwd: Some(work.clone()),
        stdin: None,
        env: vec![],
        timeout_ms: None,
    });
    assert_eq!(out.status, Some(0), "{}", String::from_utf8_lossy(&out.stderr.0));

    let run = "test-run-0001".to_string();
    let _: () = client.call(Request::RunStart {
        run: run.clone(),
        spawn: Box::new(SpawnWire {
            harness: HarnessKind::Claude,
            cwd: work.clone(),
            prompt: "hi".into(),
            model: None,
            permission: PermissionPolicy::AutoApprove,
            resume_id: None,
            disable_hooks: true,
            attachments: vec![],
            plan: false,
            compact: false,
            settings: Default::default(),
        }),
    });
    let events = client.events_of(&run);
    assert!(
        events.iter().any(|(_, e)| matches!(e, AgentEvent::SessionStarted { provider_session_id } if provider_session_id == "fake-session")),
        "{events:?}"
    );
    assert!(events.iter().any(|(_, e)| matches!(e, AgentEvent::Done(_))));
    let seqs: Vec<u64> = events.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());

    // The app goes away and comes back: the run is still there, whole.
    drop(client);
    let mut client = Client::connect(root);
    let runs: Vec<RunSummary> = client.call(Request::Runs);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].ended);
    let id = client.send(Request::RunAttach {
        run: run.clone(),
        after: 1,
    });
    let replay = client.events_of(&run);
    assert_eq!(replay, events[1..]);
    let _ = id;
    let attached: RunAttached = client.call(Request::RunAttach {
        run: "nope".into(),
        after: 0,
    });
    assert!(!attached.known);

    let _: () = client.call(Request::RunForget {
        run: run.clone(),
    });
    let runs: Vec<RunSummary> = client.call(Request::Runs);
    assert!(runs.is_empty());

    let uploaded: String = client.call(Request::Upload {
        name: "a.txt".into(),
        bytes: B64(b"attached".to_vec()),
    });
    assert!(uploaded.starts_with(root.join("host/attachments").to_str().unwrap()));

    let _: () = client.call(Request::Shutdown);
}
