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

/// MonoCode `TurnMetrics`: what the provider reports a user turn cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Share of input served from cache, as a percentage.
    pub cache_hit_percent: Option<f64>,
}

impl TurnMetrics {
    /// Zero counts are dropped, as MonoCode's `turnMetricsFrom*` do; `None`
    /// when nothing was spent. `cacheable` is the input the hit rate is
    /// over, given only when the provider reported cache fields.
    pub fn from_counts(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cacheable: Option<u64>,
    ) -> Option<Self> {
        let nonzero = |n: u64| (n > 0).then_some(n);
        let metrics = Self {
            input_tokens: nonzero(input),
            output_tokens: nonzero(output),
            cache_read_tokens: nonzero(cache_read),
            cache_write_tokens: nonzero(cache_write),
            cache_hit_percent: cacheable
                .filter(|c| *c > 0)
                .map(|c| cache_read as f64 / c as f64 * 100.0),
        };
        (metrics != Self::default()).then_some(metrics)
    }
}

/// MonoCode `TaskListItemStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl TaskStatus {
    /// MonoCode `normalizeTaskListStatus`: every provider's spelling.
    pub fn parse(value: &str) -> Self {
        let status: String = value
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .collect();
        match status.as_str() {
            "completed" | "complete" | "done" => Self::Completed,
            "inprogress" | "active" | "running" => Self::InProgress,
            "cancelled" | "canceled" | "skipped" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// MonoCode `TaskListItem`: one row of the agent's task list, in the shape
/// a `tasks` block stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub text: String,
    pub status: TaskStatus,
}

/// MonoCode `AgentStepKind`: what a subagent step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Tool,
    Message,
    Reasoning,
}

/// MonoCode `AgentStep`: one thing a subagent did, mirrored onto the
/// parent's Agent call. Steps merge by `id`, so a call that starts and
/// later completes stays one row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStep {
    pub id: String,
    pub kind: StepKind,
    /// The tool's label, or the prose the subagent wrote; empty on an
    /// update that only settles a call.
    pub text: String,
    /// The tool a tool step called (`Read`, `Bash`, …).
    pub tool_name: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
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
    /// A subagent's step, on the Agent call `call_id` that spawned it
    /// (MonoCode `agent.step`).
    AgentStep {
        call_id: String,
        step: AgentStep,
    },
    /// The model a subagent runs on, as its own messages report it.
    AgentModel {
        call_id: String,
        model: String,
    },
    /// What the turn still waits on after the agent yielded: tasks it
    /// left running in the background (MonoCode `background.updated`).
    /// Empty once the agent picks the thread back up.
    Background(Vec<String>),
    /// A status row in the work trail (MonoCode `status`), such as what a
    /// finished background subagent reported.
    Status(String),
    /// The agent's task list, whole (MonoCode `tasks.updated`); empty
    /// takes the list away.
    Tasks(Vec<TaskItem>),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
        /// The model's context window, when the CLI reports it.
        context_window: Option<u64>,
    },
    /// Token accounting for the running user turn (MonoCode `turn.metrics`).
    TurnMetrics(TurnMetrics),
    /// The turn stopped on the provider's usage limit; it resets then
    /// (epoch ms), when known.
    UsageLimited {
        resets_at: Option<i64>,
    },
    /// The conversation was compacted; the context now holds this many tokens.
    Compacted {
        tokens_after: Option<u64>,
    },
    Done(DoneStatus),
    Error(String),
}
