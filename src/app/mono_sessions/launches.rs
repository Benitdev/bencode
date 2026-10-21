//! A session a Mono starts (MonoCode `sessions.start`,
//! `monoSpawnedSessions.ts`): planned from what it asked for, launched in
//! the background, and recorded as a card on the turn that started it.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::Context;
use serde_json::{Map, Value, json};

use crate::app::agent::TurnInput;
use crate::app::app_calls::Caller;
use crate::app::{BenCodeApp, PermissionMode};
use crate::app_cli::input::{flag, is_id, optional, required};
use crate::app_cli::server::AppCall;
use crate::db::{Block, SessionRow};
use crate::harness::{HarnessKind, catalog};

/// MonoCode `MonoSpawnedSession`: what a launch card shows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnedSession {
    pub session_id: String,
    pub cwd: String,
    pub title: String,
    pub harness: String,
    pub model: String,
}

/// The launch cards on one block (MonoCode `sanitizeMonoSpawnedSessions`).
pub fn spawned_sessions(block: &Block) -> Vec<SpawnedSession> {
    let mut out: Vec<SpawnedSession> = Vec::new();
    let Some(list) = block
        .extra
        .get("monoSpawnedSessions")
        .and_then(Value::as_array)
    else {
        return out;
    };
    for item in list {
        let Ok(entry) = serde_json::from_value::<SpawnedSession>(item.clone()) else {
            continue;
        };
        let valid = !entry.session_id.is_empty()
            && entry.session_id.len() <= 512
            && entry
                .session_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            && !entry.cwd.trim().is_empty()
            && HarnessKind::from_id(&entry.harness).is_some()
            && !entry.model.trim().is_empty();
        if valid && !out.iter().any(|known| known.session_id == entry.session_id) {
            out.push(entry);
        }
    }
    out
}

/// Whether a block holds launch cards (cheap).
pub fn has_spawned(block: &Block) -> bool {
    block
        .extra
        .get("monoSpawnedSessions")
        .and_then(Value::as_array)
        .is_some_and(|list| !list.is_empty())
}

/// MonoCode `recordMonoSpawnedSession`: on the turn's message, once.
fn record_spawned(blocks: &mut [Arc<Block>], launched: &SpawnedSession) -> bool {
    let Some(anchor) = blocks
        .iter_mut()
        .rev()
        .find(|block| {
            block.role == "user"
                && block.extra.get("internal").and_then(Value::as_bool) != Some(true)
        })
        .or(None)
    else {
        return false;
    };
    let mut list = spawned_sessions(anchor);
    if list
        .iter()
        .any(|entry| entry.session_id == launched.session_id)
    {
        return true;
    }
    list.push(launched.clone());
    match serde_json::to_value(&list) {
        Ok(value) => {
            Arc::make_mut(anchor)
                .extra
                .insert("monoSpawnedSessions".into(), value);
            true
        }
        Err(err) => {
            log::error!("launch card not recorded: {err}");
            false
        }
    }
}

/// MonoCode `LaunchedSessionState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchedState {
    Working,
    Input,
    Draft,
    Failed,
    Stopped,
    Done,
}

impl LaunchedState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Input => "Needs input",
            Self::Draft => "Draft",
            Self::Failed => "Failed",
            Self::Stopped => "Stopped",
            Self::Done => "Done",
        }
    }
}

impl BenCodeApp {
    /// MonoCode `sessions.start`: a new session in one of its projects, in
    /// the background, with the Mono's model unless another is asked for.
    pub(crate) fn start_session(
        &mut self,
        caller: Caller,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let request_id = call.request.request_id.clone();
        let plan = match self.plan_start(&caller, &input, &request_id) {
            Ok(plan) => plan,
            Err(error) => return call.fail(error, true),
        };
        // The receipt names the session: a retry finds the one it made.
        if let Some(existing) = self.sessions.iter().find(|s| s.id == plan.id) {
            let value = json!({
                "id": existing.id,
                "cwd": existing.cwd,
                "harness": existing.harness,
                "model": existing.model,
                "submitted": !plan.draft,
                "draft": plan.draft,
            });
            if let Some(task) = &plan.task {
                self.link_board_task(&caller.mono, task, &plan.id, cx);
            }
            return call.answer(value);
        }
        if plan.new_worktree {
            let (cwd, base) = (
                plan.cwd.clone(),
                plan.worktree_base.clone().unwrap_or_else(|| "HEAD".into()),
            );
            let seed = {
                use std::hash::{BuildHasher, RandomState};
                RandomState::new().hash_one(&plan.id)
            };
            let branch = crate::ui::composer::new_worktree::temporary_branch_name(seed);
            let task = cx.background_executor().spawn(async move {
                crate::git::worktrees::create_worktree(&cwd, &branch, &base, false)
            });
            cx.spawn(async move |this, cx| {
                let created = task.await;
                let landed = this.update(cx, |this, cx| match created {
                    Ok(tree) => {
                        let mut plan = plan;
                        plan.worktree = Some((tree.path, tree.branch));
                        this.launch_session(caller, plan, call, cx);
                    }
                    Err(err) => call.fail(format!("Could not create the worktree: {err:#}"), true),
                });
                if let Err(err) = landed {
                    log::debug!("worktree for a Mono's session after app drop: {err:#}");
                }
            })
            .detach();
            return;
        }
        self.launch_session(caller, plan, call, cx);
    }

    /// MonoCode `startLaunch`: the launch checked against the catalog.
    fn plan_start(
        &self,
        caller: &Caller,
        input: &Map<String, Value>,
        request_id: &str,
    ) -> Result<StartPlan, String> {
        if !is_id(request_id) {
            return Err("request ID must use letters, digits, underscores or hyphens".into());
        }
        let cwd = self.require_project(caller, input)?;
        let prompt = required(input.get("prompt"), "prompt", 240_000)?;
        let draft = flag(input.get("draft"), "draft", false)?;
        let source = self
            .sessions
            .iter()
            .find(|s| s.id == caller.session)
            .ok_or_else(|| "The calling conversation is not loaded".to_string())?;
        let harness = match input.get("harness") {
            None => source.harness.clone(),
            Some(value) => {
                let id = value.as_str().unwrap_or("");
                HarnessKind::from_id(id)
                    .ok_or_else(|| {
                        "Unknown harness; run models.list for available providers".to_string()
                    })?
                    .id()
                    .to_string()
            }
        };
        let kind = HarnessKind::from_id(&harness).ok_or_else(|| "Unknown harness".to_string())?;
        if !self
            .harnesses
            .iter()
            .any(|info| info.id == kind.id() && info.available)
        {
            return Err(format!("{harness} is not available in BenCode"));
        }
        let model = match optional(input.get("model"), "model", 512)? {
            Some(model) => catalog::models_for(kind)
                .iter()
                .find(|m| m.key == model)
                .map(|m| m.key.clone())
                .ok_or_else(|| "Unknown model; run models.list for exact model IDs".to_string())?,
            None if harness == source.harness => source.model.clone(),
            None => catalog::models_for(kind)
                .first()
                .map(|m| m.key.clone())
                .ok_or_else(|| "Unknown model; run models.list for exact model IDs".to_string())?,
        };
        let mut settings: Map<String, Value> = if harness == source.harness && model == source.model
        {
            source.model_settings.clone().unwrap_or_default()
        } else {
            Map::new()
        };
        if let Some(raw) = input.get("modelSettings") {
            let raw = raw.as_object().ok_or_else(|| {
                "modelSettings must be an object of setting IDs and values".to_string()
            })?;
            settings.extend(raw.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        if let Some(effort) = input.get("effort") {
            let setting = catalog::effort_setting(&model)
                .ok_or_else(|| format!("{model} does not expose an effort setting"))?;
            settings.insert(setting.id, json!(required(Some(effort), "effort", 128)?));
        }
        let known = catalog::find(&model)
            .map(|m| m.settings)
            .unwrap_or_default();
        for (key, value) in &settings {
            let ok = known.iter().any(|setting| {
                &setting.id == key
                    && value
                        .as_str()
                        .is_some_and(|value| setting.options.iter().any(|(v, _)| v == value))
            });
            if !ok
                && input
                    .get("modelSettings")
                    .and_then(|m| m.get(key))
                    .is_some()
            {
                return Err(format!(
                    "Invalid model setting {key}; run models.list for allowed values"
                ));
            }
        }
        let settings = catalog::merge_settings(&model, Some(&settings));
        let runtime_mode = match input.get("runtimeMode") {
            None => source
                .runtime_mode
                .clone()
                .unwrap_or_else(|| PermissionMode::Supervised.id().into()),
            Some(value) => {
                let id = value.as_str().unwrap_or("");
                PermissionMode::from_id(id)
                    .ok_or_else(|| {
                        format!(
                            "runtimeMode must be one of: {}",
                            PermissionMode::ALL
                                .iter()
                                .map(|m| m.id())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?
                    .id()
                    .to_string()
            }
        };
        flag(input.get("reveal"), "reveal", false)?;
        let mode = input
            .get("workspaceMode")
            .and_then(Value::as_str)
            .unwrap_or("current");
        if input.get("workspaceMode").is_some_and(|v| !v.is_string())
            || !matches!(mode, "current" | "worktree")
        {
            return Err("workspaceMode must be current or worktree".into());
        }
        let worktree_base = optional(input.get("worktreeBase"), "worktreeBase", 512)?;
        if worktree_base.is_some() && mode != "worktree" {
            return Err("worktreeBase requires workspaceMode worktree".into());
        }
        let worktree_cwd = optional(input.get("worktreeCwd"), "worktreeCwd", 512)?;
        if worktree_cwd.is_some() && mode != "current" {
            return Err("worktreeCwd requires workspaceMode current".into());
        }
        let placement = input
            .get("placement")
            .and_then(Value::as_str)
            .unwrap_or("tab");
        if !matches!(placement, "tab" | "right" | "down") {
            return Err("placement must be tab, right or down".into());
        }
        if input.get("besideSessionId").is_some() && placement == "tab" {
            return Err("besideSessionId requires placement right or down".into());
        }
        let report = self.wants_report(caller, input, true)?;
        // Set by `board.start` only: its task is linked once launched.
        let task = optional(input.get("taskId"), "taskId", 256)?;
        let mono = self.mono(&caller.mono);
        Ok(StartPlan {
            id: format!("app-{}-{request_id}", caller.session),
            cwd,
            prompt,
            draft,
            harness,
            model,
            settings,
            runtime_mode,
            new_worktree: mode == "worktree",
            worktree_base,
            worktree: worktree_cwd.map(|path| (path, None)),
            report,
            task,
            shown: mono.is_some_and(|m| m.show_started_sessions_in_sidebar),
            folder: mono
                .filter(|m| m.show_started_sessions_in_sidebar && m.use_sidebar_folders)
                .map(|m| {
                    (
                        m.display_name(),
                        crate::app::session_folders::to_hex(self.mono_color(m)),
                    )
                }),
        })
    }

    fn launch_session(
        &mut self,
        caller: Caller,
        plan: StartPlan,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        // A checkout named by path must be one of the project's.
        if let Some((path, _)) = &plan.worktree
            && !plan.new_worktree
        {
            let known = self
                .workspace(cx)
                .worktrees
                .iter()
                .any(|tree| crate::app::same_project_path(&tree.path, path) && !tree.missing)
                || crate::app::same_project_path(path, &plan.cwd);
            if !known && crate::app::same_project_path(&self.workspace(cx).cwd, &plan.cwd) {
                return call.fail(
                    "Worktree is unavailable in this project; run worktrees.list",
                    true,
                );
            }
        }
        let now = crate::app::now_ms();
        let mut row = SessionRow {
            id: plan.id.clone(),
            title: crate::app::agent::NEW_SESSION_TITLE.to_string(),
            cwd: plan.cwd.clone(),
            harness: plan.harness.clone(),
            model: plan.model.clone(),
            created_at: now,
            updated_at: now,
            runtime_mode: Some(plan.runtime_mode.clone()),
            model_settings: Some(plan.settings.clone()),
            sidebar_hidden: !plan.shown,
            ..Default::default()
        };
        if let Some((path, branch)) = &plan.worktree
            && !crate::app::same_project_path(path, &plan.cwd)
        {
            row.worktree_cwd = Some(path.clone());
            row.branch = branch.clone();
        }
        let launched = SpawnedSession {
            session_id: row.id.clone(),
            cwd: row.cwd.clone(),
            title: crate::app::session_list::display_title(&row.title, &row.harness),
            harness: row.harness.clone(),
            model: row.model.clone(),
        };
        self.sessions.push(row);
        self.persist_session(&plan.id);
        let hidden = !plan.shown;
        let id = plan.id.clone();
        self.db_write("hide a Mono's session", move |db| {
            db.set_session_sidebar_hidden(&id, hidden)
        });
        if let Some((name, color)) = &plan.folder {
            self.file_in_mono_folder(&plan.cwd, &plan.id, name, color, cx);
        }
        if plan.draft {
            self.thread_mut(&plan.id).draft = Some(plan.prompt.clone());
        } else {
            let mut extra = Map::new();
            extra.insert("appRequestId".into(), json!(plan.id));
            self.send_turn(
                &plan.id,
                TurnInput {
                    text: plan.prompt.clone(),
                    extra,
                    ..Default::default()
                },
                cx,
            );
            if plan.report {
                self.watch_session(&caller, &plan.id, &plan.id, &plan.cwd, &plan.prompt);
            }
        }
        self.record_launch(&caller, launched);
        if let Some(task) = &plan.task {
            self.link_board_task(&caller.mono, task, &plan.id, cx);
        }
        let mut out = json!({
            "id": plan.id,
            "cwd": plan.cwd,
            "harness": plan.harness,
            "model": plan.model,
            "submitted": !plan.draft,
            "draft": plan.draft,
        });
        if plan.report {
            out["notifyOnComplete"] = json!(true);
        }
        call.answer(out);
        cx.notify();
    }

    /// MonoCode `fileInMonoFolder`: into the folder named after it, made in
    /// its colour the first time.
    fn file_in_mono_folder(
        &mut self,
        cwd: &str,
        session: &str,
        name: &str,
        color: &str,
        cx: &mut Context<Self>,
    ) {
        use crate::app::session_folders::{FolderTarget, place_session};
        let folders = self.session_folders.get(cwd).cloned().unwrap_or_default();
        let next = match folders.iter().find(|f| f.name == name) {
            Some(own) => place_session(&folders, session, &FolderTarget::Existing(own.id.clone())),
            None => {
                let mut next =
                    place_session(&folders, session, &FolderTarget::New(name.to_string()));
                if let Some(made) = next.iter_mut().find(|f| f.name == name) {
                    made.custom_color = Some(color.to_string());
                }
                next
            }
        };
        self.session_folders.insert(cwd.to_string(), next);
        self.save_settings(cx);
    }

    /// The card a launch leaves on the Mono's turn; a habit's run keeps it
    /// for its report (MonoCode `noteHabitRunLaunch`).
    fn record_launch(&mut self, caller: &Caller, launched: SpawnedSession) {
        if !caller.own {
            if let Some(run) = self.habits.runs.get_mut(&caller.session)
                && let Ok(value) = serde_json::to_value(&launched)
                && !run
                    .launches
                    .iter()
                    .any(|known| known.get("sessionId") == value.get("sessionId"))
            {
                run.launches.push(value);
            }
            return;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == caller.session) else {
            return;
        };
        if record_spawned(&mut session.blocks, &launched) {
            self.persist_session(&caller.session);
        }
    }

    /// A follow-up to a session already known in the chat puts its card on
    /// this turn too (MonoCode's `sessions.send` receipt).
    pub(super) fn note_launch(&mut self, caller: &Caller, session_id: &str) {
        let Some(target) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let launched = SpawnedSession {
            session_id: target.id.clone(),
            cwd: target.cwd.clone(),
            title: crate::app::session_list::display_title(&target.title, &target.harness),
            harness: target.harness.clone(),
            model: target.model.clone(),
        };
        self.record_launch(caller, launched);
    }
}

/// What `sessions.start` was asked for, checked.
struct StartPlan {
    id: String,
    cwd: String,
    prompt: String,
    draft: bool,
    harness: String,
    model: String,
    settings: Map<String, Value>,
    runtime_mode: String,
    new_worktree: bool,
    worktree_base: Option<String>,
    /// The checkout it runs in: (path, branch).
    worktree: Option<(String, Option<String>)>,
    report: bool,
    /// The board task it starts (`board.start`).
    task: Option<String>,
    shown: bool,
    folder: Option<(String, String)>,
}

/// The sessions a set of blocks started, in order (MonoCode
/// `monoSpawnedSessions` over the chat).
pub fn chat_launches(blocks: &[Arc<Block>]) -> Vec<SpawnedSession> {
    let mut seen = HashSet::new();
    blocks
        .iter()
        .filter(|b| has_spawned(b))
        .flat_map(|b| spawned_sessions(b))
        .filter(|entry| seen.insert(entry.session_id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launches_land_on_the_message_once() {
        let mut blocks: Vec<Arc<Block>> = vec![
            Block::new("u1", "user", "go").into(),
            Block::new("a1", "assistant", "ok").into(),
        ];
        let launched = SpawnedSession {
            session_id: "app-m-r1".into(),
            cwd: "/src/app".into(),
            title: "Claude".into(),
            harness: "claude".into(),
            model: "claude:opus".into(),
        };
        assert!(record_spawned(&mut blocks, &launched));
        assert!(record_spawned(&mut blocks, &launched));
        assert_eq!(spawned_sessions(&blocks[0]), vec![launched.clone()]);
        assert_eq!(chat_launches(&blocks), vec![launched]);
    }
}
