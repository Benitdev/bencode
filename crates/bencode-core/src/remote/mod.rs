//! Agents on another machine (MonoCode `features/connections/`): the
//! `remote://` paths the app addresses them by and the protocol it speaks
//! to `bencode-host` there.

pub mod path;
pub mod protocol;

pub use path::{REMOTE_PREFIX, is_remote};
