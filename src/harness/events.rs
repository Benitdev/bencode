use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
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
    PermissionRequest {
        id: String,
        tool: String,
        description: String,
    },
    Done {
        status: String,
    },
    Error(String),
}
