//! What BenCode's app and its remote host (`bencode-host`) share: the agent
//! CLIs (`harness`), the helpers they lean on, and the wire protocol between
//! the two (`remote`). Nothing here draws or links GPUI, so the host builds
//! for Linux as well as macOS.

pub mod files;
pub mod definitions;
pub mod git_cli;
pub mod harness;
pub mod http;
pub mod keychain;
pub mod mcp;
pub mod paths;
pub mod remote;
pub mod search;
pub mod tree;
