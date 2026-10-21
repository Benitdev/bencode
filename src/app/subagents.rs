//! MonoCode `apply.ts` `recordAgentStep` and `transcriptActivity.ts`
//! (`subagentName`, `subagentReport`, …): what a subagent did, kept on the
//! parent's Agent tool block as `agentRun` (MonoCode `AgentRunMeta`), so the
//! transcript opens the run's trail without a second session.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::db::{Block, SessionRow};
use crate::harness::{AgentStep, StepKind};

const META: &str = "agentRun";
/// MonoCode `MAX_AGENT_STEP_CHARS`.
const MAX_STEP_CHARS: usize = 2_000;
/// MonoCode `PERSISTED_AGENT_STEPS`: a run is reopened to read what it
/// concluded, not to replay every call it made.
const MAX_STEPS: usize = 100;
/// MonoCode `MAX_SUBAGENT_NAME`: past this a name is the brief again.
const MAX_NAME_CHARS: usize = 56;

fn agent_block_mut<'a>(session: &'a mut SessionRow, call_id: &str) -> Option<&'a mut Block> {
    session
        .blocks
        .iter_mut()
        .rev()
        .find(|b| {
            b.tool
                .as_ref()
                .and_then(|t| t.get("callId"))
                .and_then(Value::as_str)
                == Some(call_id)
        })
        .map(Arc::make_mut)
}

/// The block's `agentRun`, made when missing, named after the call.
fn run_mut(block: &mut Block) -> &mut Map<String, Value> {
    let title = block
        .tool
        .as_ref()
        .and_then(|t| t.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Subagent")
        .to_string();
    let run = block
        .extra
        .entry(META)
        .or_insert_with(|| json!({ "name": title, "steps": [] }));
    if !run.is_object() {
        *run = json!({ "name": title, "steps": [] });
    }
    run.as_object_mut().expect("an object")
}

fn cap(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_STEP_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

fn kind_name(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Tool => "tool",
        StepKind::Message => "message",
        StepKind::Reasoning => "reasoning",
    }
}

/// MonoCode `recordAgentStep`: steps merge by id, so a call that starts
/// pending and later completes stays one row; a completion keeps the
/// label the call announced itself with.
pub fn record_step(session: &mut SessionRow, call_id: &str, step: &AgentStep) {
    let text = cap(&step.text);
    // A tool step earns a row on its label alone; empty prose does not.
    if text.is_empty() && step.kind != StepKind::Tool {
        return;
    }
    let Some(block) = agent_block_mut(session, call_id) else {
        log::debug!("subagent step for unknown call {call_id}");
        return;
    };
    let run = run_mut(block);
    let steps = run
        .entry("steps")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !steps.is_array() {
        *steps = Value::Array(Vec::new());
    }
    let steps = steps.as_array_mut().expect("an array");
    let at = steps
        .iter()
        .position(|s| s.get("id").and_then(Value::as_str) == Some(&step.id));
    let mut row = match at {
        Some(at) => steps[at].as_object().cloned().unwrap_or_default(),
        None => Map::new(),
    };
    row.insert("id".into(), json!(step.id));
    row.insert("kind".into(), json!(kind_name(step.kind)));
    if !text.is_empty() || !row.contains_key("text") {
        row.insert("text".into(), json!(text));
    }
    if let Some(name) = &step.tool_name {
        row.insert("toolKind".into(), json!(super::agent::tool_kind(name)));
    }
    if let Some(status) = &step.status {
        row.insert("status".into(), json!(status));
    }
    if let Some(detail) = step.detail.as_deref().map(cap).filter(|d| !d.is_empty()) {
        row.insert("detail".into(), json!(detail));
    }
    match at {
        Some(at) => steps[at] = Value::Object(row),
        None => {
            steps.push(Value::Object(row));
            if steps.len() > MAX_STEPS {
                steps.drain(..steps.len() - MAX_STEPS);
            }
        }
    }
}

/// The model the run reports for itself, which may differ from its parent's.
pub fn record_model(session: &mut SessionRow, call_id: &str, model: &str) {
    if let Some(block) = agent_block_mut(session, call_id) {
        run_mut(block).insert("model".into(), json!(model));
    }
}

/// One mirrored step, as `record_step` keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    pub kind: StepKind,
    pub text: String,
    pub tool_kind: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
}

/// The steps an Agent block's run left, oldest first.
pub fn steps(block: &Block) -> Vec<Step> {
    let field = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).map(String::from);
    block
        .extra
        .get(META)
        .and_then(|run| run.get("steps"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let kind = match row.get("kind").and_then(Value::as_str)? {
                "tool" => StepKind::Tool,
                "message" => StepKind::Message,
                "reasoning" => StepKind::Reasoning,
                _ => return None,
            };
            Some(Step {
                id: field(row, "id").filter(|id| !id.is_empty())?,
                kind,
                text: field(row, "text").unwrap_or_default(),
                tool_kind: field(row, "toolKind"),
                status: field(row, "status"),
                detail: field(row, "detail"),
            })
        })
        .collect()
}

/// A step as the transcript block it stands for (MonoCode
/// `agentStepBlock`), so a run's trail goes through the same rows as the
/// main agent's work.
pub fn step_block(step: &Step) -> Block {
    match step.kind {
        StepKind::Tool => {
            let mut tool = Map::new();
            tool.insert("callId".into(), json!(step.id));
            tool.insert("title".into(), json!(step.text));
            tool.insert(
                "kind".into(),
                json!(step.tool_kind.as_deref().unwrap_or("other")),
            );
            if let Some(status) = &step.status {
                tool.insert("status".into(), json!(status));
            }
            if let Some(detail) = &step.detail {
                tool.insert("detail".into(), json!(detail));
            }
            let mut block = Block::new(step.id.clone(), "tool", step.text.clone());
            block.tool = Some(Value::Object(tool));
            block
        }
        StepKind::Message => Block::new(step.id.clone(), "assistant", step.text.clone()),
        StepKind::Reasoning => Block::new(step.id.clone(), "reasoning", step.text.clone()),
    }
}

/// MonoCode `subagentBrief`: the whole brief the run was spawned with.
pub fn brief(block: &Block) -> String {
    let named = block
        .extra
        .get(META)
        .and_then(|run| run.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let title = block
        .tool
        .as_ref()
        .and_then(|t| t.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = named.unwrap_or(title).trim();
    let lower = name.to_ascii_lowercase();
    let stripped = ["agent", "task", "subagent"]
        .iter()
        .find_map(|word| {
            let rest = lower.strip_prefix(word)?;
            let boundary = rest.chars().next().is_none_or(|c| !c.is_alphanumeric());
            boundary.then(|| name[word.len()..].trim_start_matches([' ', ':', '·', '-']))
        })
        .unwrap_or(name)
        .trim();
    if stripped.is_empty() {
        "Subagent".into()
    } else {
        stripped.to_string()
    }
}

/// MonoCode `subagentName`: the brief's first sentence, capped on a word.
pub fn name(block: &Block) -> String {
    let brief = brief(block);
    let sentence = brief
        .find(['.', '!', '?'])
        .map_or(brief.as_str(), |end| &brief[..=end])
        .trim();
    let name = if sentence.is_empty() {
        brief.as_str()
    } else {
        sentence
    };
    let Some((cut, _)) = name.char_indices().nth(MAX_NAME_CHARS) else {
        return name.to_string();
    };
    let head = &name[..cut];
    let head = match head.rfind(' ') {
        Some(space) if space > head.len() / 2 => &head[..space],
        _ => head,
    };
    format!("{}…", head.trim_end_matches([' ', ',', ';', ':']))
}

/// The model id the run reported, unless it is a placeholder.
pub fn model(block: &Block) -> Option<&str> {
    let model = block.extra.get(META)?.get("model")?.as_str()?.trim();
    let placeholder = ["auto", "default", "inherit", "unspecified"]
        .iter()
        .any(|p| model.eq_ignore_ascii_case(p));
    (!model.is_empty() && !placeholder).then_some(model)
}

/// MonoCode `subagentStatusLine`: a run is counted, never narrated.
pub fn status_line(failed: bool, steps: &[Step]) -> String {
    if failed {
        return "failed".into();
    }
    let tools: Vec<&Step> = steps.iter().filter(|s| s.kind == StepKind::Tool).collect();
    let count = match tools.len() {
        0 => return String::new(),
        1 => "1 step".to_string(),
        n => format!("{n} steps"),
    };
    let failed = tools
        .iter()
        .filter(|s| matches!(s.status.as_deref(), Some("failed" | "error")))
        .count();
    match failed {
        0 => count,
        1 => format!("{count}, 1 failed"),
        n => format!("{count}, {n} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SessionRow;

    fn agent_session() -> SessionRow {
        let mut block = Block::new("tool-a1", "tool", "");
        block.tool = Some(json!({
            "callId": "a1",
            "kind": "agent",
            "title": "Correctness review. Look at the reducer",
            "status": "in_progress",
        }));
        SessionRow {
            blocks: vec![block.into()],
            ..Default::default()
        }
    }

    fn step(id: &str, kind: StepKind, text: &str, status: Option<&str>) -> AgentStep {
        AgentStep {
            id: id.into(),
            kind,
            text: text.into(),
            tool_name: (kind == StepKind::Tool).then(|| "Read".into()),
            status: status.map(String::from),
            detail: None,
        }
    }

    #[test]
    fn steps_merge_by_id_and_keep_their_label() {
        let mut s = agent_session();
        record_step(
            &mut s,
            "a1",
            &step("m1:text", StepKind::Message, "I will grep", None),
        );
        record_step(
            &mut s,
            "a1",
            &step("r1", StepKind::Tool, "/repo/a.rs", Some("in_progress")),
        );
        record_step(
            &mut s,
            "a1",
            &step("r1", StepKind::Tool, "", Some("completed")),
        );
        // Empty prose earns no row.
        record_step(
            &mut s,
            "a1",
            &step("m2:text", StepKind::Message, "  ", None),
        );
        let steps = steps(&s.blocks[0]);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].text, "/repo/a.rs");
        assert_eq!(steps[1].status.as_deref(), Some("completed"));
        assert_eq!(steps[1].tool_kind.as_deref(), Some("read"));
        assert_eq!(status_line(false, &steps), "1 step");
        let block = step_block(&steps[1]);
        assert_eq!(block.role, "tool");
        assert_eq!(block.tool.unwrap()["kind"], "read");
    }

    #[test]
    fn a_run_keeps_only_its_latest_steps() {
        let mut s = agent_session();
        for n in 0..(MAX_STEPS + 5) {
            record_step(
                &mut s,
                "a1",
                &step(&format!("t{n}"), StepKind::Tool, "x", None),
            );
        }
        let steps = steps(&s.blocks[0]);
        assert_eq!(steps.len(), MAX_STEPS);
        assert_eq!(steps[0].id, "t5");
    }

    #[test]
    fn a_run_is_named_by_its_first_sentence() {
        let mut s = agent_session();
        assert_eq!(name(&s.blocks[0]), "Correctness review.");
        record_model(&mut s, "a1", "claude-haiku-4-5");
        assert_eq!(model(&s.blocks[0]), Some("claude-haiku-4-5"));
        let mut long = Block::new("b", "tool", "");
        long.tool = Some(
            json!({ "title": "Agent: trace the legacy stock sync from the shell script through every action it loads" }),
        );
        let named = name(&long);
        assert!(named.starts_with("trace the legacy"), "{named}");
        assert!(named.ends_with('…') && named.chars().count() <= MAX_NAME_CHARS + 1);
    }

    #[test]
    fn the_status_line_counts_failures() {
        let tool = |status: &str| Step {
            id: status.into(),
            kind: StepKind::Tool,
            text: String::new(),
            tool_kind: None,
            status: Some(status.into()),
            detail: None,
        };
        let steps = [tool("completed"), tool("failed")];
        assert_eq!(status_line(false, &steps), "2 steps, 1 failed");
        assert_eq!(status_line(true, &steps), "failed");
        assert_eq!(status_line(false, &[]), "");
    }
}
