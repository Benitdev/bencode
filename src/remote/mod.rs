//! Agents on another machine (MonoCode `features/connections/`,
//! `src-tauri/src/remote.rs` and `remote_ssh.rs`; MonoCode Host is
//! `bencode-host` here). A folder on a machine is a project whose path is
//! `remote://<machine>/<path>`; everything that takes a path routes it
//! here: git and gh (`exec`), the Explorer and the editor (`files`), the
//! agent's turn (`spawn`), Go to File and search.
//!
//! The machines are `remoteMachines` in `settings.json`; their connections
//! live in a registry every module can reach, since the git and file code
//! runs on background executors with nothing but a path.

pub mod askpass;
pub mod connection;
pub mod exec;
pub mod files;
pub mod setup;
pub mod spawn;
pub mod ssh;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, OnceLock, PoisonError, RwLock};
use std::time::Duration;

pub use bencode_core::remote::path::{is_remote, remote_path};
pub use bencode_core::remote::protocol;
use serde::{Deserialize, Serialize};

pub use connection::{Connection, Status};

/// A machine this BenCode connects to (MonoCode `RemoteMachine`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    /// The host's environment id: its `remote://` name.
    pub id: String,
    pub name: String,
    /// What `ssh` connects to: `user@host` or an alias from the SSH config.
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl Machine {
    /// MonoCode's machine row: `SSH · user@host · port 2222`.
    pub fn address(&self) -> String {
        match self.port {
            Some(port) => format!("SSH · {} · port {port}", self.target),
            None => format!("SSH · {}", self.target),
        }
    }
}

static REGISTRY: LazyLock<RwLock<HashMap<String, Arc<Connection>>>> =
    LazyLock::new(Default::default);
static ON_CHANGE: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

/// What a connection calls when its status changes; the app redraws.
pub fn set_on_change(on_change: Arc<dyn Fn() + Send + Sync>) {
    if ON_CHANGE.set(on_change).is_err() {
        log::warn!("remote: the change hook was set twice");
    }
}

fn on_change() -> Arc<dyn Fn() + Send + Sync> {
    ON_CHANGE.get().cloned().unwrap_or_else(|| Arc::new(|| {}))
}

/// Makes the registry hold `machines`: new ones get a connection (opened
/// on first use), removed ones are closed.
pub fn sync_machines(machines: &[Machine]) {
    let mut registry = REGISTRY.write().unwrap_or_else(PoisonError::into_inner);
    registry.retain(|id, conn| {
        let keep = machines.iter().any(|m| &m.id == id);
        if !keep {
            conn.close();
        }
        keep
    });
    for machine in machines {
        match registry.get(&machine.id) {
            Some(conn) => conn.set_machine(machine.clone()),
            None => {
                registry.insert(machine.id.clone(), Connection::new(machine.clone(), on_change()));
            }
        }
    }
}

/// Adds a connection a setup has already opened (Settings' Connect).
pub fn adopt(conn: Arc<Connection>) {
    let id = conn.machine().id;
    let old = REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, conn);
    if let Some(old) = old {
        old.close();
    }
}

pub fn connection(machine: &str) -> Option<Arc<Connection>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(machine)
        .cloned()
}

/// A new connection for `machine`, not yet in the registry.
pub fn new_connection(machine: Machine) -> Arc<Connection> {
    Connection::new(machine, on_change())
}

/// The machine a `remote://` path is on, and the path there.
pub struct Route {
    pub conn: Arc<Connection>,
    pub machine: String,
    pub host_path: String,
}

impl Route {
    pub fn call<T: serde::de::DeserializeOwned>(&self, request: protocol::Request) -> Result<T, String> {
        self.conn.call(request, connection::DEFAULT_TIMEOUT)
    }

    pub fn call_for<T: serde::de::DeserializeOwned>(
        &self,
        request: protocol::Request,
        timeout: Duration,
    ) -> Result<T, String> {
        self.conn.call(request, timeout)
    }

    /// `path` on this route's machine as a host path; a local path or one
    /// on another machine is refused.
    pub fn host_path_of(&self, path: &str) -> Result<String, String> {
        bencode_core::remote::path::host_path_on(path, &self.machine).ok_or_else(|| {
            format!("{path} is not on {}.", self.conn.machine().name)
        })
    }
}

/// `None` for a local path. A `remote://` path on a machine BenCode does
/// not know is an error the caller shows.
pub fn route(path: impl AsRef<Path>) -> Option<Result<Route, String>> {
    let text = path.as_ref().to_str()?;
    if !is_remote(text) {
        return None;
    }
    let Some(parsed) = bencode_core::remote::path::parse(text) else {
        return Some(Err(format!("{text} is not a path on a machine.")));
    };
    let machine = parsed.machine.to_string();
    Some(match connection(&machine) {
        Some(conn) => Ok(Route {
            conn,
            machine,
            host_path: parsed.host_path,
        }),
        None => Err(format!(
            "{text} is on a machine BenCode is not connected to. Add it in Settings › Connections."
        )),
    })
}

/// Whether `path` is a folder here that is gone. A path on a machine is
/// never asked about here: that costs a round trip, and the command that
/// follows fails on its own.
pub fn missing_here(path: &str) -> bool {
    !is_remote(path) && !Path::new(path).exists()
}
