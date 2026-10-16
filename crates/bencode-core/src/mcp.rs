//! How an agent CLI starts an MCP server BenCode offers it (the in-app
//! browser's, `browser/mcp.rs` in the app): the harnesses write it into
//! each CLI's own MCP config.

/// The server's name in the CLI's MCP config.
pub const MCP_SERVER_NAME: &str = "bencode-browser";

/// The command and arguments that start the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
}
