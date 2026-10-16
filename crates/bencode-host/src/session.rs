//! One connection to the daemon: an app over SSH (through `connect`), or
//! `status` / `stop`. Lines in are `ClientFrame`s, lines out `ServerFrame`s;
//! each request runs on a thread of its own, so a slow `git push` or model
//! probe holds up nothing else, and replies go out as they are ready.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;

use bencode_core::remote::protocol::{
    ClientFrame, MAX_FRAME_BYTES, PROTOCOL_VERSION, Request, RunAttached, ServerFrame,
};

use crate::daemon::Host;

/// Frames waiting to be written to one connection, in order.
pub struct Outbox {
    tx: mpsc::Sender<String>,
    open: AtomicBool,
}

impl Outbox {
    /// False once the connection is gone, so a run stops sending to it.
    pub fn send(&self, frame: &ServerFrame) -> bool {
        if !self.open.load(Ordering::Relaxed) {
            return false;
        }
        match serde_json::to_string(frame) {
            Ok(line) => self.tx.send(line).is_ok(),
            Err(err) => {
                log::error!("could not encode a frame: {err}");
                true
            }
        }
    }
}

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

pub fn serve(stream: UnixStream, host: Arc<Host>) {
    let conn = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
    let Ok(writer) = stream.try_clone() else {
        log::error!("connection {conn}: could not share its socket");
        return;
    };
    let (tx, rx) = mpsc::channel::<String>();
    let outbox = Arc::new(Outbox {
        tx,
        open: AtomicBool::new(true),
    });
    let writing = std::thread::spawn(move || write_lines(writer, rx));
    read_frames(stream, conn, &host, &outbox);
    outbox.open.store(false, Ordering::Relaxed);
    host.runs.detach_all(conn);
    drop(outbox);
    let _ = writing.join();
    log::info!("connection {conn}: closed");
}

fn write_lines(mut stream: UnixStream, rx: mpsc::Receiver<String>) {
    for line in rx {
        let written = stream
            .write_all(line.as_bytes())
            .and_then(|()| stream.write_all(b"\n"));
        if written.is_err() {
            break;
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn read_frames(stream: UnixStream, conn: u64, host: &Arc<Host>, outbox: &Arc<Outbox>) {
    let mut reader = BufReader::new(stream);
    let mut said_hello = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_FRAME_BYTES as u64 + 1)
            .read_until(b'\n', &mut line);
        match read {
            Ok(0) => return,
            Ok(_) if line.len() > MAX_FRAME_BYTES => {
                log::warn!("connection {conn}: a frame over the limit; closing");
                return;
            }
            Ok(_) => {}
            Err(err) => {
                log::info!("connection {conn}: {err}");
                return;
            }
        }
        let frame: ClientFrame = match serde_json::from_slice(trim_newline(&line)) {
            Ok(frame) => frame,
            Err(err) => {
                // An id can still be read from a request this host does not
                // know (a newer app), so it hears why.
                let id = serde_json::from_slice::<serde_json::Value>(&line)
                    .ok()
                    .and_then(|value| value.get("id").and_then(|id| id.as_u64()));
                match id {
                    Some(id) => {
                        outbox.send(&ServerFrame::fail(id, format!("the host cannot read this request: {err}")));
                    }
                    None => log::warn!("connection {conn}: unreadable frame: {err}"),
                }
                continue;
            }
        };
        if let Request::Hello { protocol, client } = &frame.request {
            if *protocol != PROTOCOL_VERSION {
                outbox.send(&ServerFrame::fail(
                    frame.id,
                    format!(
                        "this host speaks protocol {PROTOCOL_VERSION} and the app {protocol}: update the host from Settings › Connections"
                    ),
                ));
                continue;
            }
            log::info!("connection {conn}: hello from {client}");
            said_hello = true;
            outbox.send(&ServerFrame::ok(frame.id, host.info()));
            continue;
        }
        if !said_hello {
            outbox.send(&ServerFrame::fail(frame.id, "say hello first"));
            continue;
        }
        let host = host.clone();
        let outbox = outbox.clone();
        std::thread::spawn(move || {
            let shutdown = matches!(frame.request, Request::Shutdown);
            let reply = handle(&host, conn, &outbox, frame);
            outbox.send(&reply);
            if shutdown {
                host.shutdown();
            }
        });
    }
}

fn trim_newline(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

fn handle(host: &Arc<Host>, conn: u64, outbox: &Arc<Outbox>, frame: ClientFrame) -> ServerFrame {
    let id = frame.id;
    let result: anyhow::Result<ServerFrame> = (|| {
        Ok(match frame.request {
            Request::Hello { .. } => ServerFrame::ok(id, host.info()),
            Request::Harnesses => ServerFrame::ok(id, crate::ops::harnesses()),
            Request::Models { harness } => ServerFrame::ok(id, crate::ops::models(harness)?),
            Request::Exec {
                program,
                args,
                cwd,
                stdin,
                env,
                timeout_ms,
            } => ServerFrame::ok(
                id,
                crate::ops::exec(program, &args, cwd.as_deref(), stdin, &env, timeout_ms)?,
            ),
            Request::ReadDir { path } => ServerFrame::ok(id, crate::ops::read_dir(&path)?),
            Request::ReadFile { path, max } => ServerFrame::ok(id, crate::ops::read_file(&path, max)?),
            Request::Stat { path } => ServerFrame::ok(id, crate::ops::stat(&path)),
            Request::WriteFile {
                path,
                bytes,
                create_new,
            } => {
                crate::ops::write_file(&path, &bytes.0, create_new)?;
                ServerFrame::ok(id, ())
            }
            Request::CreateDir { path } => {
                crate::ops::create_dir(&path)?;
                ServerFrame::ok(id, ())
            }
            Request::Rename { from, to } => {
                crate::ops::rename(&from, &to)?;
                ServerFrame::ok(id, ())
            }
            Request::Remove { path } => {
                crate::ops::remove(&path)?;
                ServerFrame::ok(id, ())
            }
            Request::Copy { from, to } => {
                crate::ops::copy(&from, &to)?;
                ServerFrame::ok(id, ())
            }
            Request::Tree { root, op } => crate::ops::tree(id, &root, op)?,
            Request::CountLines {
                root,
                files,
                max_bytes,
            } => {
                let root = std::path::PathBuf::from(crate::ops::absolute(&root)?);
                let counts: Vec<usize> = files
                    .iter()
                    .map(|file| bencode_core::files::count_text_lines(&root.join(file), max_bytes))
                    .collect();
                ServerFrame::ok(id, counts)
            }
            Request::ProjectFiles { root } => ServerFrame::ok(
                id,
                bencode_core::files::list_project_files(std::path::Path::new(&crate::ops::absolute(&root)?)),
            ),
            Request::Search { root, options } => {
                let never = std::sync::atomic::AtomicBool::new(false);
                let root = crate::ops::absolute(&root)?;
                let found = bencode_core::search::search(std::path::Path::new(&root), &options, &never)
                    .map_err(anyhow::Error::msg)?;
                ServerFrame::ok(id, found)
            }
            Request::Upload { name, bytes } => {
                ServerFrame::ok(id, crate::ops::upload(&host.dirs.attachments(), &name, &bytes.0)?)
            }
            Request::RunStart { run, spawn } => {
                host.runs.start(&run, *spawn, conn, outbox.clone())?;
                ServerFrame::ok(id, ())
            }
            Request::RunAttach { run, after } => {
                let known = host.runs.attach(&run, after, conn, outbox.clone());
                ServerFrame::ok(id, RunAttached { known })
            }
            Request::RunStdin { run, line } => ServerFrame::ok(id, host.runs.stdin(&run, &line)?),
            Request::RunPermission {
                run,
                request,
                allow,
            } => ServerFrame::ok(id, host.runs.permission(&run, &request, allow)?),
            Request::RunSteer {
                run,
                prompt,
                attachments,
            } => ServerFrame::ok(id, host.runs.steer(&run, &prompt, &attachments)?),
            Request::RunCancel { run } => {
                host.runs.cancel(&run)?;
                ServerFrame::ok(id, ())
            }
            Request::RunForget { run } => {
                host.runs.forget(&run);
                ServerFrame::ok(id, ())
            }
            Request::Runs => ServerFrame::ok(id, host.runs.summaries()),
            Request::Shutdown => ServerFrame::ok(id, ()),
        })
    })();
    result.unwrap_or_else(|err| ServerFrame::fail(id, format!("{err:#}")))
}
