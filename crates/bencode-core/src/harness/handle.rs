use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::harness::attachments::Attachment;
use crate::harness::events::PermissionRequest;

/// Messages for the child's stdin writer task.
#[derive(Debug)]
pub enum StdinMsg {
    Line(String),
    Close,
}

/// A follow-up for the running turn, handed to the harness' parser
/// (`LineParser::steer`), which knows how its protocol takes one.
#[derive(Debug)]
pub struct Steer {
    pub prompt: String,
    pub attachments: Vec<Attachment>,
}

/// Builds the harness-specific stdin reply to a permission prompt.
pub type PermissionResponder = fn(&PermissionRequest, bool) -> String;

/// What a turn running on another machine is asked to do; its relay
/// (`remote/` in the app) sends each to the host, where the turn's own
/// handle does it.
#[derive(Debug)]
pub enum RelayMsg {
    Line(String),
    Permission(PermissionRequest, bool),
    Steer(Steer),
}

#[derive(Clone)]
enum Control {
    /// A child process of this one.
    Local {
        stdin_tx: mpsc::UnboundedSender<StdinMsg>,
        permission_responder: Option<PermissionResponder>,
        /// None for a harness that takes nothing in the middle of a turn.
        steer_tx: Option<mpsc::UnboundedSender<Steer>>,
    },
    /// A turn the remote host runs.
    Relay {
        tx: mpsc::UnboundedSender<RelayMsg>,
        answers_permissions: bool,
        can_steer: bool,
    },
}

/// Cheap, cloneable control handle for one running harness process. Every
/// method is synchronous and safe to call from the GPUI thread.
#[derive(Clone)]
pub struct HarnessProcessHandle {
    control: Control,
    cancel_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl HarnessProcessHandle {
    pub fn new(
        stdin_tx: mpsc::UnboundedSender<StdinMsg>,
        cancel_tx: oneshot::Sender<()>,
        permission_responder: Option<PermissionResponder>,
        steer_tx: Option<mpsc::UnboundedSender<Steer>>,
    ) -> Self {
        Self {
            control: Control::Local {
                stdin_tx,
                permission_responder,
                steer_tx,
            },
            cancel_tx: Arc::new(Mutex::new(Some(cancel_tx))),
        }
    }

    /// A handle on a turn another machine runs: what it is asked goes to
    /// `tx`, a cancel to `cancel_tx`. The flags say what the harness takes
    /// there (`harness::answers_permissions`, `harness::can_steer`).
    pub fn relay(
        tx: mpsc::UnboundedSender<RelayMsg>,
        cancel_tx: oneshot::Sender<()>,
        answers_permissions: bool,
        can_steer: bool,
    ) -> Self {
        Self {
            control: Control::Relay {
                tx,
                answers_permissions,
                can_steer,
            },
            cancel_tx: Arc::new(Mutex::new(Some(cancel_tx))),
        }
    }

    /// Whether this harness takes a follow-up in the middle of a turn.
    pub fn can_steer(&self) -> bool {
        match &self.control {
            Control::Local { steer_tx, .. } => steer_tx.is_some(),
            Control::Relay { can_steer, .. } => *can_steer,
        }
    }

    /// Hands a follow-up to the running turn. False when the harness cannot
    /// take one, or its process is gone.
    pub fn steer(&self, prompt: &str, attachments: &[Attachment]) -> bool {
        let steer = Steer {
            prompt: prompt.to_string(),
            attachments: attachments.to_vec(),
        };
        match &self.control {
            Control::Local { steer_tx, .. } => {
                steer_tx.as_ref().is_some_and(|tx| tx.send(steer).is_ok())
            }
            Control::Relay { tx, can_steer, .. } => {
                *can_steer && tx.send(RelayMsg::Steer(steer)).is_ok()
            }
        }
    }

    /// Writes one line to the child's stdin; a trailing newline is added.
    pub fn send_line(&self, line: &str) -> bool {
        match &self.control {
            Control::Local { stdin_tx, .. } => {
                stdin_tx.send(StdinMsg::Line(format!("{line}\n"))).is_ok()
            }
            Control::Relay { tx, .. } => tx.send(RelayMsg::Line(line.to_string())).is_ok(),
        }
    }

    /// Answers a permission prompt using the harness' own wire format.
    /// Returns false when the harness has no interactive permission protocol.
    pub fn respond_permission(&self, request: &PermissionRequest, allow: bool) -> bool {
        match &self.control {
            Control::Local {
                permission_responder: Some(responder),
                ..
            } => self.send_line(&responder(request, allow)),
            Control::Local { .. } => false,
            Control::Relay {
                tx,
                answers_permissions,
                ..
            } => {
                *answers_permissions
                    && tx
                        .send(RelayMsg::Permission(request.clone(), allow))
                        .is_ok()
            }
        }
    }

    /// Kills the child process. Idempotent.
    pub fn cancel(&self) {
        let sender = self
            .cancel_tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(tx) = sender {
            let _ = tx.send(());
        }
    }
}
