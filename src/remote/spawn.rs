//! An agent turn on a machine (MonoCode `RemoteSession`'s commands, which
//! the host's engine runs). `spawn` gives the app what `harness::spawn`
//! does, a handle and the turn's events, so the transcript, approvals,
//! questions and follow-ups work as they do here. The host runs the CLI and
//! keeps the events; this end follows them over the machine's connection.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bencode_core::harness::handle::RelayMsg;
use bencode_core::harness::{
    self, AgentEvent, Attachment, DoneStatus, EventRx, HarnessProcessHandle, SpawnRequest,
};
use bencode_core::remote::protocol::{B64, MAX_UPLOAD_BYTES, Request, SpawnWire};
use tokio::sync::{mpsc, oneshot};

use super::Connection;
use super::connection::DEFAULT_TIMEOUT;

/// A turn started on a machine: the app keeps `machine` and `run` so it
/// can follow the turn again after a restart (`attach`).
pub struct RemoteTurn {
    pub handle: HarnessProcessHandle,
    pub events: EventRx,
    pub machine: String,
    pub run: String,
}

/// Starts `req` (its `cwd` a `remote://` path) on that machine. Returns at
/// once; the upload and the start happen on the harness runtime, and a
/// failure arrives as the turn's error.
pub fn spawn(req: &SpawnRequest) -> Result<RemoteTurn> {
    let route = super::route(&req.cwd)
        .context("not a folder on a machine")?
        .map_err(anyhow::Error::msg)?;
    if req
        .attachments
        .iter()
        .any(|file| file.mime_type == "inode/directory")
    {
        bail!("Folders cannot be attached in a project on a machine; attach the files instead.");
    }
    let run = super::askpass::random_hex(8).context("no random source")?;
    let (event_tx, events) = mpsc::unbounded_channel();
    route.conn.follow(&run, event_tx.clone());
    let (relay_tx, relay_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let handle = HarnessProcessHandle::relay(
        relay_tx,
        cancel_tx,
        harness::answers_permissions(req.harness),
        harness::can_steer(req.harness, req.compact),
    );
    let spawn = SpawnWire::new(req, route.host_path.clone(), Vec::new());
    let attachments = req.attachments.clone();
    harness::runtime::runtime().spawn(drive(
        route.conn.clone(),
        run.clone(),
        spawn,
        attachments,
        event_tx,
        relay_rx,
        cancel_rx,
    ));
    Ok(RemoteTurn {
        handle,
        events,
        machine: route.machine,
        run,
    })
}

/// Follows a turn the app started before it last quit: its events from
/// the first, then the ones to come. The handle can still cancel it and
/// answer its prompts.
pub fn attach(machine: &str, run: &str, harness: harness::HarnessKind) -> Result<RemoteTurn> {
    let conn = super::connection(machine)
        .with_context(|| format!("BenCode is not connected to the machine {machine}"))?;
    let (event_tx, events) = mpsc::unbounded_channel();
    conn.follow(run, event_tx);
    let (relay_tx, relay_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let handle = HarnessProcessHandle::relay(
        relay_tx,
        cancel_tx,
        harness::answers_permissions(harness),
        harness::can_steer(harness, false),
    );
    let owned = run.to_string();
    harness::runtime::runtime().spawn(async move {
        let (attach_conn, attach_run) = (conn.clone(), owned.clone());
        let attached = tokio::task::spawn_blocking(move || attach_conn.attach(&attach_run)).await;
        if let Err(err) = attached {
            log::error!("remote: attaching a run panicked: {err}");
        }
        relay(conn, owned, relay_rx, cancel_rx).await;
    });
    Ok(RemoteTurn {
        handle,
        events,
        machine: machine.to_string(),
        run: run.to_string(),
    })
}

async fn drive(
    conn: Arc<Connection>,
    run: String,
    mut spawn: SpawnWire,
    attachments: Vec<Attachment>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    relay_rx: mpsc::UnboundedReceiver<RelayMsg>,
    cancel_rx: oneshot::Receiver<()>,
) {
    let start_conn = conn.clone();
    let start_run = run.clone();
    let started = tokio::task::spawn_blocking(move || -> Result<(), String> {
        spawn.attachments = upload_all(&start_conn, &attachments)?;
        start_conn.call::<()>(
            Request::RunStart {
                run: start_run,
                spawn: Box::new(spawn),
            },
            DEFAULT_TIMEOUT,
        )
    })
    .await
    .unwrap_or_else(|err| Err(format!("starting the turn panicked: {err}")));
    if let Err(err) = started {
        conn.unfollow(&run);
        let _ = event_tx.send(AgentEvent::Error(format!(
            "Could not start the turn on the machine: {err}"
        )));
        let _ = event_tx.send(AgentEvent::Done(DoneStatus::Failed));
        return;
    }
    // The follower holds the other sender; the stream ends with the run.
    drop(event_tx);
    relay(conn, run, relay_rx, cancel_rx).await;
}

/// Sends what the app asks of the turn to the host, until the handle is
/// gone or the turn is cancelled.
async fn relay(
    conn: Arc<Connection>,
    run: String,
    mut relay_rx: mpsc::UnboundedReceiver<RelayMsg>,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            msg = relay_rx.recv() => {
                let Some(msg) = msg else { return };
                let request = match msg {
                    RelayMsg::Line(line) => Request::RunStdin { run: run.clone(), line },
                    RelayMsg::Permission(request, allow) => Request::RunPermission {
                        run: run.clone(),
                        request,
                        allow,
                    },
                    RelayMsg::Steer(steer) => {
                        let conn = conn.clone();
                        let files = tokio::task::spawn_blocking(move || upload_all(&conn, &steer.attachments).map(|files| (steer.prompt, files)))
                            .await
                            .unwrap_or_else(|err| Err(err.to_string()));
                        match files {
                            Ok((prompt, attachments)) => Request::RunSteer { run: run.clone(), prompt, attachments },
                            Err(err) => {
                                log::warn!("remote: a follow-up's attachments did not upload: {err}");
                                continue;
                            }
                        }
                    }
                };
                send(&conn, request).await;
            }
            cancelled = &mut cancel_rx => {
                if cancelled.is_ok() {
                    send(&conn, Request::RunCancel { run: run.clone() }).await;
                }
                return;
            }
        }
    }
}

async fn send(conn: &Arc<Connection>, request: Request) {
    let conn = conn.clone();
    let sent = tokio::task::spawn_blocking(move || conn.call_value(request, DEFAULT_TIMEOUT)).await;
    match sent {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            log::warn!("remote: the machine did not take a request for the turn: {err}")
        }
        Err(err) => log::error!("remote: a request for the turn panicked: {err}"),
    }
}

/// Copies each file to the host; the turn reads them there.
fn upload_all(conn: &Arc<Connection>, files: &[Attachment]) -> Result<Vec<Attachment>, String> {
    files
        .iter()
        .map(|file| {
            let bytes = std::fs::read(Path::new(&file.path))
                .map_err(|err| format!("{}: {err}", file.name))?;
            if bytes.len() > MAX_UPLOAD_BYTES {
                return Err(format!("{} is larger than 20 MB", file.name));
            }
            let path: String = conn.call(
                Request::Upload {
                    name: file.name.clone(),
                    bytes: B64(bytes),
                },
                DEFAULT_TIMEOUT,
            )?;
            Ok(Attachment {
                path,
                ..file.clone()
            })
        })
        .collect()
}
