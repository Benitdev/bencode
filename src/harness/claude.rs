use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::harness::events::AgentEvent;

pub struct ClaudeHarness;

impl ClaudeHarness {
    pub async fn spawn(
        cwd: &str,
        prompt: &str,
        model: Option<&str>,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<()> {
        let mut cmd = Command::new("claude");
        cmd.current_dir(cwd)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("-p")
            .arg(prompt)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }

        let mut child = cmd
            .spawn()
            .context("Failed to spawn `claude` CLI. Ensure `claude` is installed in PATH.")?;

        let stdout = child.stdout.take().context("Failed to open child stdout")?;
        let _stdin = child.stdin.take().context("Failed to open child stdin")?;

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match event_type {
                        "text" | "content_block_delta" => {
                            if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
                                let _ = event_tx.send(AgentEvent::TextDelta(text.to_string()));
                            } else if let Some(delta) = val.get("delta").and_then(|d| d.get("text")).and_then(|v| v.as_str()) {
                                let _ = event_tx.send(AgentEvent::TextDelta(delta.to_string()));
                            }
                        }
                        "thinking" => {
                            if let Some(thought) = val.get("thinking").and_then(|v| v.as_str()) {
                                let _ = event_tx.send(AgentEvent::ThinkingDelta(thought.to_string()));
                            }
                        }
                        "tool_use" => {
                            let id = val.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let input = val.get("input").cloned().unwrap_or(serde_json::Value::Null);
                            let _ = event_tx.send(AgentEvent::ToolCallStart { id, name, input });
                        }
                        "result" => {
                            let _ = event_tx.send(AgentEvent::Done {
                                status: "completed".to_string(),
                            });
                        }
                        _ => {}
                    }
                }
            }

            let _ = child.wait().await;
        });

        Ok(())
    }
}
