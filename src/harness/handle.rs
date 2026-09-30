use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::harness::events::PermissionRequest;

/// Messages for the child's stdin writer task.
#[derive(Debug)]
pub enum StdinMsg {
    Line(String),
    Close,
}

/// Builds the harness-specific stdin reply to a permission prompt.
pub type PermissionResponder = fn(&PermissionRequest, bool) -> String;

/// Cheap, cloneable control handle for one running harness process. Every
/// method is synchronous and safe to call from the GPUI thread.
#[derive(Clone)]
pub struct HarnessProcessHandle {
    stdin_tx: mpsc::UnboundedSender<StdinMsg>,
    cancel_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    permission_responder: Option<PermissionResponder>,
}

impl HarnessProcessHandle {
    pub fn new(
        stdin_tx: mpsc::UnboundedSender<StdinMsg>,
        cancel_tx: oneshot::Sender<()>,
        permission_responder: Option<PermissionResponder>,
    ) -> Self {
        Self {
            stdin_tx,
            cancel_tx: Arc::new(Mutex::new(Some(cancel_tx))),
            permission_responder,
        }
    }

    /// Writes one line to the child's stdin; a trailing newline is added.
    pub fn send_line(&self, line: &str) -> bool {
        self.stdin_tx.send(StdinMsg::Line(format!("{line}\n"))).is_ok()
    }

    /// Answers a permission prompt using the harness' own wire format.
    /// Returns false when the harness has no interactive permission protocol.
    pub fn respond_permission(&self, request: &PermissionRequest, allow: bool) -> bool {
        match self.permission_responder {
            Some(responder) => self.send_line(&responder(request, allow)),
            None => false,
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
