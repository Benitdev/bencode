//! BenCode's own: brings the sessions Claude Code or Codex ran outside
//! BenCode in as threads (Settings › Providers › Import sessions), so a
//! conversation started in a terminal carries on here with the same
//! provider session. Reading is `harness/session_import.rs`; this folds a
//! session into transcript blocks the way a live run does and saves the
//! threads BenCode does not have yet.
//!
//! Left out: sessions a thread already resumes (BenCode's own runs leave
//! their files there too), and those whose folder is gone or is the home
//! folder (no project to show or run them in).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use gpui::Context;

use crate::app::agent::{apply_event, cancel_open_tools, start_turn};
use crate::app::{BenCodeApp, ids, is_home_path, normalize_project_path, same_project_path};
use crate::db::SessionRow;
use crate::harness::session_import::{self, ImportedSession};
use crate::harness::{AgentEvent, DoneStatus, HarnessKind};

/// Where one provider's import is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportState {
    Running,
    Done(ImportReport),
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Threads added.
    pub added: usize,
    /// Sessions a thread already resumes.
    pub known: usize,
    /// Sessions whose folder is gone or is the home folder.
    pub homeless: usize,
}

/// Each provider's last import, for its row in Settings.
pub type CliImports = HashMap<HarnessKind, ImportState>;

/// The `harness:model` key for a CLI's own model id; the CLI's default
/// when it named none.
fn model_key(kind: HarnessKind, model: Option<&str>) -> String {
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    match (kind, model) {
        (HarnessKind::Claude, Some(id)) => {
            format!("claude:{}", id.strip_prefix("claude-").unwrap_or(id))
        }
        (_, Some(id)) => format!("{}:{id}", kind.id()),
        (_, None) => format!("{}:default", kind.id()),
    }
}

/// `imported` as a thread, its transcript folded from the same events a
/// live run sends.
pub fn session_row(kind: HarnessKind, imported: ImportedSession) -> SessionRow {
    let mut row = SessionRow {
        id: ids::unique_id("bencode"),
        title: String::new(),
        cwd: normalize_project_path(&imported.cwd),
        harness: kind.id().to_string(),
        model: model_key(kind, imported.model.as_deref()),
        created_at: imported.created_at,
        updated_at: imported.updated_at,
        branch: imported.branch,
        archived: imported.archived,
        provider_account_id: imported.account_id,
        ..Default::default()
    };
    for turn in imported.turns {
        start_turn(&mut row, &turn.prompt, turn.at, &[]);
        for (at, event) in turn.events {
            apply_event(&mut row, event, at);
        }
        apply_event(
            &mut row,
            AgentEvent::Done(DoneStatus::Completed),
            turn.ended_at,
        );
    }
    // A call the CLI never answered (a quit, an interrupt) is over.
    cancel_open_tools(&mut row);
    if let Some(title) = imported.title {
        row.title = title;
    }
    // Set last: folding the events moved it.
    row.provider_session_id = Some(imported.provider_session_id);
    row.updated_at = imported.updated_at;
    row
}

/// Whether a thread could show and run in `cwd` on this machine.
fn has_project(cwd: &str) -> bool {
    !cwd.is_empty() && !is_home_path(cwd) && Path::new(cwd).is_dir()
}

impl BenCodeApp {
    /// Imports every `kind` session on this machine BenCode does not have.
    /// Files are read off the UI thread, rows written on the database's.
    pub fn import_cli_sessions(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        if !session_import::supported(kind)
            || self.cli_imports.get(&kind) == Some(&ImportState::Running)
        {
            return;
        }
        self.cli_imports.insert(kind, ImportState::Running);
        cx.notify();
        // Threads in memory may not be saved yet.
        let in_memory: HashSet<String> = self
            .sessions
            .iter()
            .filter_map(|s| s.provider_session_id.clone())
            .collect();
        let read = cx.background_executor().spawn(async move {
            let (mut known, mut homeless) = (0, 0);
            let rows: Vec<SessionRow> = session_import::read_all(kind, |imported| {
                if in_memory.contains(&imported.provider_session_id) {
                    known += 1;
                    return None;
                }
                if !has_project(&imported.cwd) {
                    homeless += 1;
                    return None;
                }
                Some(session_row(kind, imported))
            })
            .into_iter()
            .flatten()
            .collect();
            let report = ImportReport {
                added: 0,
                known,
                homeless,
            };
            (rows, report)
        });
        cx.spawn(async move |this, cx| {
            let (rows, report) = read.await;
            let landed = this.update(cx, |app, cx| app.save_imported(kind, rows, report, cx));
            if let Err(err) = landed {
                log::debug!("imported sessions after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn save_imported(
        &mut self,
        kind: HarnessKind,
        rows: Vec<SessionRow>,
        report: ImportReport,
        cx: &mut Context<Self>,
    ) {
        self.db_then(
            cx,
            move |db| {
                let resumed = db.provider_session_ids()?;
                let (mut added, mut known) = (Vec::new(), 0);
                for row in rows {
                    if row
                        .provider_session_id
                        .as_ref()
                        .is_some_and(|id| resumed.contains(id))
                    {
                        known += 1;
                        continue;
                    }
                    db.upsert_session(&row)?;
                    added.push(row.cwd);
                }
                Ok((added, known))
            },
            move |app, result, cx| {
                let state = match result {
                    Ok((added, known)) => {
                        let report = ImportReport {
                            added: added.len(),
                            known: report.known + known,
                            ..report
                        };
                        log::info!("imported {} {} sessions", added.len(), kind.id());
                        app.show_imported_projects(added, cx);
                        ImportState::Done(report)
                    }
                    Err(err) => {
                        log::error!("importing {} sessions failed: {err:#}", kind.id());
                        ImportState::Failed(format!("{err:#}"))
                    }
                };
                app.cli_imports.insert(kind, state);
                cx.notify();
            },
        );
    }

    /// The rail gains the projects new threads landed in, and the current
    /// project's list reads them in.
    fn show_imported_projects(&mut self, cwds: Vec<String>, cx: &mut Context<Self>) {
        for cwd in cwds {
            if !self
                .recent_projects
                .iter()
                .any(|p| same_project_path(p, &cwd))
            {
                self.recent_projects.push(cwd);
            }
        }
        let current = self.current_cwd.clone();
        self.load_sessions_in_background(Some(current), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::session_import::ImportedTurn;
    use serde_json::json;

    fn imported() -> ImportedSession {
        ImportedSession {
            provider_session_id: "c0ffee-1".into(),
            cwd: "/Users/me/app/".into(),
            model: Some("claude-opus-5-5".into()),
            title: Some("Fix the login bug".into()),
            branch: Some("main".into()),
            account_id: Some("work".into()),
            archived: false,
            created_at: 1_000,
            updated_at: 9_000,
            turns: vec![ImportedTurn {
                prompt: "Fix it".into(),
                at: 1_000,
                events: vec![
                    (2_000, AgentEvent::TextDelta("Looking.".into())),
                    (
                        3_000,
                        AgentEvent::ToolCallStart {
                            id: "t1".into(),
                            name: "Bash".into(),
                            input: json!({"command": "ls"}),
                        },
                    ),
                    (
                        3_500,
                        AgentEvent::ToolCallStart {
                            id: "t2".into(),
                            name: "Bash".into(),
                            input: json!({"command": "sleep 99"}),
                        },
                    ),
                    (
                        4_000,
                        AgentEvent::ToolCallFinish {
                            id: "t1".into(),
                            output: "a".into(),
                            success: true,
                            patch: None,
                        },
                    ),
                ],
                ended_at: 9_000,
            }],
        }
    }

    #[test]
    fn a_session_folds_into_a_thread_that_resumes_it() {
        let row = session_row(HarnessKind::Claude, imported());
        assert_eq!(row.provider_session_id.as_deref(), Some("c0ffee-1"));
        assert_eq!(row.provider_account_id.as_deref(), Some("work"));
        assert_eq!(row.cwd, "/Users/me/app");
        assert_eq!(row.model, "claude:opus-5-5");
        assert_eq!(row.title, "Fix the login bug");
        assert_eq!((row.created_at, row.updated_at), (1_000, 9_000));
        let roles: Vec<_> = row.blocks.iter().map(|b| b.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "tool"]);
        assert_eq!(row.blocks[0].duration_ms, Some(8_000));
        let status = |ix: usize| row.blocks[ix].tool.as_ref().unwrap()["status"].clone();
        assert_eq!(status(2), "completed");
        // The call the CLI never answered does not spin forever.
        assert_eq!(status(3), "cancelled");
    }

    #[test]
    fn an_untitled_session_is_named_after_its_first_prompt() {
        let row = session_row(
            HarnessKind::Codex,
            ImportedSession {
                title: None,
                model: None,
                ..imported()
            },
        );
        assert_eq!(row.title, "Fix it");
        assert_eq!(row.model, "codex:default");
    }

    #[test]
    fn model_keys_use_bencode_s_spelling() {
        assert_eq!(
            model_key(HarnessKind::Claude, Some("claude-sonnet-5-5")),
            "claude:sonnet-5-5"
        );
        assert_eq!(
            model_key(HarnessKind::Codex, Some("gpt-5.5")),
            "codex:gpt-5.5"
        );
    }
}

