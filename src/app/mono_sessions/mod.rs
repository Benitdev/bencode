//! Sessions a Mono drives (MonoCode `agentApp.ts`'s `sessions.*`,
//! `worktrees.*` and `folders.*`): it lists, reads, starts, continues,
//! stops, archives and deletes sessions in its projects, saves drafts,
//! picks and makes worktrees and files sessions in folders. Starting one
//! and its launch card are `launches.rs`; the results that come back to the
//! Mono when the sessions it watches stop are `completion.rs`.

mod completion;
mod launches;

use gpui::Context;
use serde_json::{Map, Value, json};

use crate::app::BenCodeApp;
use crate::app::agent::TurnInput;
use crate::app::app_calls::Caller;
use crate::app_cli::input::{flag, is_id, optional, required};
use crate::app_cli::server::AppCall;
use crate::db::{Block, SessionRow};

pub use completion::MonoSessionsState;
use completion::clip;
pub use launches::{LaunchedState, SpawnedSession, chat_launches, spawned_sessions};

/// MonoCode `sessionConversationPage`: prose only, newest page first, with
/// a cursor for older exchanges.
pub fn conversation_page(
    session: &SessionRow,
    busy: bool,
    before: Option<&str>,
    limit: usize,
    max_chars: usize,
) -> Result<Value, String> {
    let flag =
        |block: &Block, key: &str| block.extra.get(key).and_then(Value::as_bool) == Some(true);
    let mut exchanges: Vec<(&Block, Vec<&Block>)> = Vec::new();
    for block in &session.blocks {
        let text = block.text.as_deref().unwrap_or("");
        if block.role == "user"
            && (!flag(block, "internal") || block.extra.contains_key("monoSessionCompletion"))
            && !flag(block, "draft")
        {
            exchanges.push((block, Vec::new()));
        } else if block.role == "assistant"
            && !flag(block, "internal")
            && block.tool.is_none()
            && !text.trim().is_empty()
            && let Some(last) = exchanges.last_mut()
        {
            last.1.push(block);
        }
    }
    let end = match before {
        None => exchanges.len(),
        Some(id) => exchanges
            .iter()
            .position(|(user, _)| user.id == id)
            .ok_or_else(|| "before is not a turn ID in this session".to_string())?,
    };
    let start = end.saturating_sub(limit);
    let capped = |text: &str| {
        let trimmed = text.trim();
        let (text, truncated) = clip(trimmed, max_chars);
        json!({ "text": text, "truncated": truncated })
    };
    let turns: Vec<Value> = exchanges[start..end]
        .iter()
        .map(|(user, replies)| {
            let said = match user.extra.get("monoSessionCompletion") {
                Some(meta) => match meta.get("sessionCount").and_then(Value::as_u64) {
                    Some(n) => format!("BenCode: results from {n} sessions"),
                    None => format!(
                        "BenCode: session {}: {}",
                        meta.get("status").and_then(Value::as_str).unwrap_or(""),
                        meta.get("title").and_then(Value::as_str).unwrap_or("")
                    ),
                },
                None => user.text.clone().unwrap_or_default(),
            };
            json!({
                "turnId": user.id,
                "user": capped(&said),
                "assistant": replies.last().map(|reply| capped(reply.text.as_deref().unwrap_or(""))),
                "earlierAssistantMessages": replies.len().saturating_sub(1),
            })
        })
        .collect();
    Ok(json!({
        "sessionId": session.id,
        "title": session.title,
        "busy": busy,
        "hasDraft": session.blocks.iter().any(|b| b.role == "user" && flag(b, "draft")),
        "turns": turns,
        "nextBefore": if start > 0 { exchanges.get(start).map(|(user, _)| json!(user.id)) } else { None },
    }))
}

fn project_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

impl BenCodeApp {
    /// MonoCode `requireProject`: the project an action works in. A Mono
    /// works on several, so it names one with "project" (its path or
    /// name), and may leave it out when it has only one.
    pub(crate) fn require_project(
        &self,
        caller: &Caller,
        input: &Map<String, Value>,
    ) -> Result<String, String> {
        let projects = self
            .mono(&caller.mono)
            .map(|m| m.projects.clone())
            .unwrap_or_default();
        let choices = || {
            if projects.is_empty() {
                "none yet; the user adds them from your settings".to_string()
            } else {
                projects
                    .iter()
                    .map(|path| format!("{} ({path})", project_name(path)))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        if let Some(named) = optional(input.get("project"), "project", 4096)? {
            let by_path = projects
                .iter()
                .find(|path| crate::app::same_project_path(path, &named));
            let by_name: Vec<&String> = projects
                .iter()
                .filter(|path| project_name(path) == named)
                .collect();
            return by_path
                .or(if by_name.len() == 1 {
                    Some(by_name[0])
                } else {
                    None
                })
                .cloned()
                .ok_or_else(|| format!("Not one of your projects. Yours: {}", choices()));
        }
        let own = self
            .sessions
            .iter()
            .find(|s| s.id == caller.session)
            .and_then(|s| {
                projects
                    .iter()
                    .find(|p| crate::app::same_project_path(p, &s.cwd))
            });
        if let Some(own) = own {
            return Ok(own.clone());
        }
        match projects.as_slice() {
            [only] => Ok(only.clone()),
            _ => Err(format!(
                "Pass \"project\" to choose one. Yours: {}",
                choices()
            )),
        }
    }

    /// The project's sessions a Mono may see: not its own chat, not a run.
    pub(crate) fn project_sessions(&self, cwd: &str) -> Vec<&SessionRow> {
        self.sessions
            .iter()
            .filter(|s| crate::app::same_project_path(&s.cwd, cwd))
            .filter(|s| !self.is_mono_session(&s.id) && !self.is_habit_run(&s.id))
            .collect()
    }

    /// MonoCode `projectSession`: `id` in the chosen project.
    fn project_session(
        &self,
        caller: &Caller,
        input: &Map<String, Value>,
    ) -> Result<(String, String), String> {
        let cwd = self.require_project(caller, input)?;
        let id = required(input.get("sessionId"), "sessionId", 256)?;
        if !self.project_sessions(&cwd).iter().any(|s| s.id == id) {
            return Err("Session was not found in this project".into());
        }
        Ok((cwd, id))
    }

    /// MonoCode `launchedSessionState`.
    pub fn launched_state(&self, session_id: &str) -> Option<LaunchedState> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        if self.pending_permission_for(session_id).is_some() {
            return Some(LaunchedState::Input);
        }
        if self.is_agent_running_in(session_id) {
            return Some(LaunchedState::Working);
        }
        if self
            .thread(session_id)
            .and_then(|t| t.draft.as_deref())
            .is_some_and(|d| !d.trim().is_empty())
            && !session.has_user_message()
        {
            return Some(LaunchedState::Draft);
        }
        let last = session.blocks.last();
        let notice = last
            .and_then(|b| b.extra.get("notice"))
            .and_then(Value::as_str);
        Some(match notice {
            Some("error") => LaunchedState::Failed,
            Some("interrupt") => LaunchedState::Stopped,
            _ if last
                .is_some_and(|b| b.text.as_deref() == Some(crate::app::agent::STOPPED_NOTICE)) =>
            {
                LaunchedState::Stopped
            }
            _ => LaunchedState::Done,
        })
    }

    /// `sessions.*`, `worktrees.*` and `folders.*`.
    pub(crate) fn mono_sessions_call(
        &mut self,
        caller: Caller,
        action: &str,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let request_id = call.request.request_id.clone();
        let result = match action {
            "sessions.list" => self.require_project(&caller, &input).map(|cwd| {
                let sessions: Vec<Value> = self
                    .project_sessions(&cwd)
                    .iter()
                    .map(|s| {
                        json!({
                            "id": s.id,
                            "title": crate::app::session_list::display_title(&s.title, &s.harness),
                            "harness": s.harness,
                            "model": s.model,
                            "busy": self.is_agent_running_in(&s.id),
                            "hasDraft": self.thread(&s.id).and_then(|t| t.draft.as_deref()).is_some_and(|d| !d.trim().is_empty()),
                            "archived": s.archived,
                        })
                    })
                    .collect();
                json!({ "cwd": cwd, "sessions": sessions })
            }),
            "sessions.read" => self.read_session(&caller, &input),
            "sessions.send" => self.send_to_session(&caller, &input, &request_id, cx),
            "sessions.draft" => self.draft_in_session(&caller, &input, &request_id, cx),
            "sessions.stop" | "sessions.archive" | "sessions.delete" => {
                self.manage_session(&caller, action, &input, &request_id, cx)
            }
            "sessions.start" => {
                return self.start_session(caller, input, call, cx);
            }
            "worktrees.list" => {
                let cwd = match self.require_project(&caller, &input) {
                    Ok(cwd) => cwd,
                    Err(error) => return call.fail(error, true),
                };
                let task = cx.background_executor().spawn(async move {
                    crate::git::worktrees::list_worktrees(&cwd).map(|trees| json!({ "cwd": cwd, "worktrees": trees }))
                });
                cx.spawn(async move |_, _| match task.await {
                    Ok(value) => call.answer(value),
                    Err(err) => call.fail(format!("{err:#}"), true),
                })
                .detach();
                return;
            }
            "worktrees.create" => {
                let parsed = (|| {
                    let cwd = self.require_project(&caller, &input)?;
                    let branch = required(input.get("branch"), "branch", 400)?;
                    let existing = flag(input.get("existing"), "existing", false)?;
                    let base = optional(input.get("base"), "base", 400)?;
                    if existing && base.is_some() {
                        return Err("base cannot be set for an existing branch".to_string());
                    }
                    Ok((cwd, branch, base.unwrap_or_else(|| "HEAD".into()), existing))
                })();
                let (cwd, branch, base, existing) = match parsed {
                    Ok(parsed) => parsed,
                    Err(error) => return call.fail(error, true),
                };
                let task = cx.background_executor().spawn(async move {
                    crate::git::worktrees::create_worktree(&cwd, &branch, &base, existing)
                });
                cx.spawn(async move |this, cx| {
                    match task.await {
                        Ok(tree) => call.answer(serde_json::to_value(tree).unwrap_or(json!({}))),
                        Err(err) => call.fail(format!("{err:#}"), true),
                    }
                    let refreshed = this.update(cx, |this, cx| this.refresh_workspace(cx));
                    if let Err(err) = refreshed {
                        log::debug!("worktree made after app drop: {err:#}");
                    }
                })
                .detach();
                return;
            }
            "folders.list" => self.require_project(&caller, &input).map(|cwd| {
                let folders: Vec<Value> = self
                    .session_folders
                    .get(&cwd)
                    .map(|list| {
                        list.iter()
                            .map(|f| json!({ "id": f.id, "name": f.name, "sessionIds": f.session_ids }))
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "cwd": cwd, "folders": folders })
            }),
            "folders.move" => self.move_to_folder(&caller, &input, cx),
            _ => Err(format!("Unknown app action: {action}")),
        };
        match result {
            Ok(value) => call.answer(value),
            Err(error) => call.fail(error, true),
        }
        cx.notify();
    }

    fn read_session(&self, caller: &Caller, input: &Map<String, Value>) -> Result<Value, String> {
        let id = required(input.get("sessionId"), "sessionId", 256)?;
        let target = if id == caller.session && caller.own {
            id
        } else {
            self.project_session(caller, input)?.1
        };
        let session = self
            .sessions
            .iter()
            .find(|s| s.id == target)
            .ok_or_else(|| "Session was not found in this project".to_string())?;
        let limit = match input.get("limit") {
            None => 3,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=3).contains(n))
                .ok_or_else(|| "limit must be an integer from 1 to 3".to_string())?
                as usize,
        };
        let max_chars = match input.get("maxChars") {
            None => 1200,
            Some(value) => value
                .as_u64()
                .filter(|n| (200..=6000).contains(n))
                .ok_or_else(|| "maxChars must be an integer from 200 to 6000".to_string())?
                as usize,
        };
        let before = optional(input.get("before"), "before", 256)?;
        conversation_page(
            session,
            self.is_agent_running_in(&target),
            before.as_deref(),
            limit,
            max_chars,
        )
    }

    /// MonoCode `completionRecipient`: whether this call asks for a report.
    fn wants_report(
        &self,
        caller: &Caller,
        input: &Map<String, Value>,
        default: bool,
    ) -> Result<bool, String> {
        let draft = flag(input.get("draft"), "draft", false)?;
        let requested = match input.get("notifyOnComplete") {
            None => None,
            Some(Value::Bool(on)) => Some(*on),
            Some(_) => return Err("notifyOnComplete must be a boolean".into()),
        };
        if !requested.unwrap_or(default && caller.own && !draft) {
            return Ok(false);
        }
        if !caller.own {
            return Err("Completion notifications are only available in a Mono's chat".into());
        }
        if draft {
            return Err("An unsent draft cannot send a completion notification".into());
        }
        Ok(true)
    }

    fn send_to_session(
        &mut self,
        caller: &Caller,
        input: &Map<String, Value>,
        request_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let id = required(input.get("sessionId"), "sessionId", 256)?;
        let prompt = required(input.get("prompt"), "prompt", 240_000)?;
        let report = self.wants_report(caller, input, false)?;
        if id == caller.session {
            return Err("Use the current conversation to continue this session".into());
        }
        if !is_id(request_id) {
            return Err("Invalid request ID".into());
        }
        let (cwd, id) = self.project_session(caller, input)?;
        let receipt = format!("app-{}-{request_id}", caller.session);
        let already = self.sessions.iter().find(|s| s.id == id).is_some_and(|s| {
            s.blocks.iter().any(|b| {
                b.extra.get("appRequestId").and_then(Value::as_str) == Some(receipt.as_str())
            })
        });
        if !already {
            if self.is_agent_running_in(&id) {
                return Err("That session is busy; wait for it to finish or stop it first".into());
            }
            if let Some(why) = self.turn_blocked(&id) {
                return Err(format!("That session {why}"));
            }
            let mut extra = Map::new();
            extra.insert("appRequestId".into(), json!(receipt));
            self.send_turn(
                &id,
                TurnInput {
                    text: prompt.clone(),
                    extra,
                    ..Default::default()
                },
                cx,
            );
            if report {
                self.watch_session(caller, &receipt, &id, &cwd, &prompt);
            }
            self.note_launch(caller, &id);
        }
        let mut out = json!({ "sessionId": id, "submitted": true, "alreadySubmitted": already });
        if report {
            out["notifyOnComplete"] = json!(true);
        }
        Ok(out)
    }

    fn draft_in_session(
        &mut self,
        caller: &Caller,
        input: &Map<String, Value>,
        request_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let id = required(input.get("sessionId"), "sessionId", 256)?;
        let prompt = required(input.get("prompt"), "prompt", 240_000)?;
        if id == caller.session {
            return Err("Use the composer to save a draft in this session".into());
        }
        if !is_id(request_id) {
            return Err("Invalid request ID".into());
        }
        let (_, id) = self.project_session(caller, input)?;
        if self.is_agent_running_in(&id) {
            return Err("That session is busy; save the draft once it is idle".into());
        }
        let current = self
            .thread(&id)
            .and_then(|t| t.draft.clone())
            .unwrap_or_default();
        if current.trim() == prompt.trim() {
            return Ok(
                json!({ "sessionId": id, "saved": true, "alreadySaved": true, "draft": true }),
            );
        }
        if !current.trim().is_empty() {
            return Err(
                "That session already has a draft; send or remove it in BenCode first".into(),
            );
        }
        self.thread_mut(&id).draft = Some(prompt);
        cx.notify();
        Ok(json!({ "sessionId": id, "saved": true, "alreadySaved": false, "draft": true }))
    }

    fn manage_session(
        &mut self,
        caller: &Caller,
        action: &str,
        input: &Map<String, Value>,
        request_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let id = required(input.get("sessionId"), "sessionId", 256)?;
        if id == caller.session {
            return Err("Cannot stop, archive or delete the calling session".into());
        }
        if !is_id(request_id) {
            return Err("Invalid request ID".into());
        }
        let (_, id) = self.project_session(caller, input)?;
        let orchestrated = self
            .sessions
            .iter()
            .find(|s| s.id == id)
            .is_some_and(|s| s.orchestration.is_some());
        if orchestrated || self.is_mono_session(&id) || self.is_habit_run(&id) {
            return Err("Only regular project sessions can be managed here".into());
        }
        // The Mono knows how its own command went: no report for it.
        self.dismiss_watched(&caller.session, &id);
        match action {
            "sessions.stop" => {
                self.stop_agent(&id, cx);
                Ok(json!({ "sessionId": id, "stopped": true }))
            }
            "sessions.archive" => {
                self.stop_agent(&id, cx);
                self.set_session_archived(&id, true, cx);
                Ok(json!({ "sessionId": id, "archived": true }))
            }
            _ => {
                self.stop_agent(&id, cx);
                self.delete_session(&id, cx);
                Ok(json!({ "sessionId": id, "deleted": true }))
            }
        }
    }

    fn move_to_folder(
        &mut self,
        caller: &Caller,
        input: &Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let cwd = self.require_project(caller, input)?;
        let session = required(input.get("sessionId"), "sessionId", 256)?;
        let folder = optional(input.get("folderId"), "folderId", 256)?;
        let name = optional(input.get("newFolderName"), "newFolderName", 100)?;
        if folder.is_some() == name.is_some() {
            return Err("Supply exactly one of folderId or newFolderName".into());
        }
        if !self.project_sessions(&cwd).iter().any(|s| s.id == session) {
            return Err("Session was not found in this project".into());
        }
        let folders = self.session_folders.get(&cwd).cloned().unwrap_or_default();
        if let Some(folder) = &folder
            && !folders.iter().any(|f| &f.id == folder)
        {
            return Err("Folder was not found in this project".into());
        }
        let target = match (folder, name) {
            (Some(id), _) => crate::app::session_folders::FolderTarget::Existing(id),
            (_, Some(name)) => crate::app::session_folders::FolderTarget::New(name),
            _ => unreachable!("checked above"),
        };
        let next = crate::app::session_folders::place_session(&folders, &session, &target);
        let placed = next
            .iter()
            .find(|f| f.session_ids.contains(&session))
            .cloned();
        self.session_folders.insert(cwd, next);
        self.save_settings(cx);
        Ok(json!({
            "sessionId": session,
            "folderId": placed.as_ref().map(|f| f.id.clone()),
            "folderName": placed.map(|f| f.name),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn session(blocks: Vec<Block>) -> SessionRow {
        SessionRow {
            id: "s1".into(),
            title: "Fix the build".into(),
            harness: "claude".into(),
            blocks: blocks.into_iter().map(Arc::new).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_conversation_reads_as_exchanges() {
        let mut internal = Block::new("x", "user", "app note");
        internal.extra.insert("internal".into(), json!(true));
        let blocks = vec![
            Block::new("u1", "user", "one"),
            Block::new("a1", "assistant", "reply one"),
            internal,
            Block::new("u2", "user", "two"),
            Block::new("a2a", "assistant", "first"),
            Block::new("a2b", "assistant", "second"),
            Block::new("u3", "user", "three"),
        ];
        let page = conversation_page(&session(blocks.clone()), false, None, 2, 1200).unwrap();
        assert_eq!(page["turns"].as_array().unwrap().len(), 2);
        assert_eq!(page["turns"][0]["assistant"]["text"], "second");
        assert_eq!(page["turns"][0]["earlierAssistantMessages"], 1);
        assert_eq!(page["nextBefore"], "u2");
        let older =
            conversation_page(&session(blocks.clone()), false, Some("u2"), 3, 1200).unwrap();
        assert_eq!(older["turns"][0]["turnId"], "u1");
        assert_eq!(older["nextBefore"], Value::Null);
        assert!(conversation_page(&session(blocks), false, Some("nope"), 3, 1200).is_err());
    }
}
