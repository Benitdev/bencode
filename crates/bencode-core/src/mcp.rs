//! How an agent CLI starts an MCP server BenCode offers it (the in-app
//! browser's, `browser/mcp.rs` in the app; Backlog's, `backlog/mcp.rs`):
//! the harnesses write each into the CLI's own MCP config.

/// The in-app browser's server, as the agents list its tools
/// (`mcp__bencode-browser__…`).
pub const BROWSER_SERVER: &str = "bencode-browser";
/// The connected Backlog space's server.
pub const BACKLOG_SERVER: &str = "bencode-backlog";

/// One server: its name in the CLI's MCP config, and the command and
/// arguments that start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}
