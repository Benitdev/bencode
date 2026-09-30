use serde::{Deserialize, Serialize};

/// A tool-permission prompt raised by a running harness. Carries everything the
/// harness needs to build its reply, so the UI never has to know the protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool: String,
    pub description: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoneStatus {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Provider-side conversation id; persisted so the next turn can resume.
    SessionStarted {
        provider_session_id: String,
    },
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolCallFinish {
        id: String,
        output: String,
        success: bool,
    },
    PermissionRequest(PermissionRequest),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
    },
    Done(DoneStatus),
    Error(String),
}
