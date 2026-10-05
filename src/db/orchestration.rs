//! What MonoCode records about an orchestrator thread
//! (`session_store.rs` `index_orchestration`, `orchestrationSummary.ts`).
//! BenCode never runs orchestrations; it shows the saved summary.

use serde::Deserialize;

/// MonoCode's three orchestration tables, created when missing so the
/// session query can read them in BenCode's own database too.
pub(super) const ORCHESTRATION_SQL: &str = "
    CREATE TABLE IF NOT EXISTS orchestration_runs (lead_id TEXT PRIMARY KEY, state TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS orchestration_sidebar (lead_id TEXT PRIMARY KEY, summary TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS orchestration_workers (session_id TEXT PRIMARY KEY, lead_id TEXT NOT NULL);
";

/// MonoCode `OrchestrationSidebarSummary` as saved (never `live`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrchestrationSummary {
    /// `active`, `paused`, `stopped` or `finished`.
    pub status: String,
    pub tasks: Vec<OrchestrationTask>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrchestrationTask {
    pub session_id: Option<String>,
    pub title: String,
    pub harness: String,
    pub model: String,
    /// MonoCode `TaskStatus`.
    pub status: String,
}

/// How a task's label is tinted (MonoCode's row colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTone {
    Attention,
    Done,
    Quiet,
}

impl OrchestrationSummary {
    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn done(&self) -> usize {
        self.tasks.iter().filter(|t| t.status == "completed").count()
    }
}

impl OrchestrationTask {
    /// MonoCode `orchestrationTaskLabel` for a saved summary: running work
    /// reads "Saved" (it is not live here); `needs_input` comes from the
    /// worker thread's own pending approval or question.
    pub fn label(&self, run_status: &str, needs_input: bool) -> &'static str {
        if needs_input {
            return "Needs input";
        }
        match self.status.as_str() {
            "running" | "cancelling" | "queued" if run_status == "paused" && self.status == "queued" => "Paused",
            "running" | "cancelling" | "queued" => "Saved",
            "completed" => "Done",
            "failed" => "Failed",
            "blocked" => "Needs review",
            "interrupted" => "Interrupted",
            "cancelled" => "Cancelled",
            _ => "Queued",
        }
    }

    pub fn tone(&self, needs_input: bool) -> TaskTone {
        match self.status.as_str() {
            _ if needs_input => TaskTone::Attention,
            "failed" | "blocked" | "interrupted" => TaskTone::Attention,
            "completed" => TaskTone::Done,
            _ => TaskTone::Quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_summaries_parse_and_label_like_monocode() {
        let json = r#"{"status":"active","tasks":[
            {"sessionId":"w1","title":"Tests","harness":"claude","model":"claude:opus","status":"running"},
            {"sessionId":"w2","title":"Docs","harness":"codex","model":"codex:gpt-5","status":"completed"},
            {"title":"Lint","harness":"claude","model":"","status":"blocked"}]}"#;
        let summary = OrchestrationSummary::from_json(json).unwrap();
        assert_eq!(summary.tasks.len(), 3);
        assert_eq!(summary.done(), 1);
        assert_eq!(summary.tasks[0].label("active", false), "Saved");
        assert_eq!(summary.tasks[0].label("active", true), "Needs input");
        assert_eq!(summary.tasks[1].label("active", false), "Done");
        assert_eq!(summary.tasks[2].label("active", false), "Needs review");
        assert_eq!(summary.tasks[2].tone(false), TaskTone::Attention);
        let queued = OrchestrationTask { status: "queued".into(), ..Default::default() };
        assert_eq!(queued.label("paused", false), "Paused");
        assert!(OrchestrationSummary::from_json("not json").is_none());
    }
}
