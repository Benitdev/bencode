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
