//! A Bot's board on the app (`bots/board.rs`): `board.*` from its chat,
//! `board.start` launching a task's session through `sessions.start` and
//! linking the two, and the Overview's Board tab acting for the user (mark
//! done, reopen, remove).

use gpui::Context;
use serde_json::{Map, Value, json};

use crate::app::BenCodeApp;
use crate::app::app_calls::Caller;
use crate::app::bot_files::bot_files_dir;
use crate::app_cli::input::{flag, is_id, optional, required};
use crate::app_cli::server::AppCall;
use crate::bots::board::{self, Column, Edit, PROMPT_MAX, TITLE_MAX, Task};

/// What `board.list` shows of a prompt.
const PROMPT_PREVIEW: usize = 300;

/// `board.start`'s options, passed on to `sessions.start`.
const START_OPTIONS: [&str; 9] = [
    "harness",
    "model",
    "modelSettings",
    "effort",
    "runtimeMode",
    "workspaceMode",
    "worktreeBase",
    "worktreeCwd",
    "notifyOnComplete",
];

impl BenCodeApp {
    pub fn board_column(&self, task: &Task) -> Column {
        board::column(
            task,
            task.session_id
                .as_deref()
                .and_then(|id| self.launched_state(id)),
        )
    }

    fn task_view(&self, task: &Task) -> Value {
        let state = task
            .session_id
            .as_deref()
            .and_then(|id| self.launched_state(id));
        let brief = task.brief();
        let mut view = json!({
            "id": task.id,
            "title": task.title,
            "project": task.project,
            "column": self.board_column(task).id(),
            "prompt": brief.chars().take(PROMPT_PREVIEW).collect::<String>(),
        });
        if brief.chars().count() > PROMPT_PREVIEW {
            view["promptTruncated"] = json!(true);
        }
        if let Some(session) = &task.session_id {
            view["sessionId"] = json!(session);
            view["sessionStatus"] = json!(state.map_or("Not loaded", |s| s.label()));
        }
        view
    }

    fn board_view(&self, tasks: &[Task]) -> Value {
        let state = |id: &str| self.launched_state(id);
        let ordered: Vec<Value> = Column::ALL
            .iter()
            .flat_map(|&col| board::in_column(tasks, col, state))
            .map(|task| self.task_view(task))
            .collect();
        json!({ "tasks": ordered })
    }

    /// `board.*` from a Bot's own conversation.
    pub(crate) fn board_call(
        &mut self,
        caller: Caller,
        action: &str,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = bot_files_dir(&caller.bot) else {
            return call.fail("This bot's folder is unavailable", true);
        };
        let request_id = call.request.request_id.clone();
        let edit = match action {
            "board.list" => None,
            "board.start" => return self.start_board_task(caller, input, call, cx),
            _ => match self.board_edit(&caller, action, &input, &request_id) {
                Ok(edit) => Some(edit),
                Err(error) => return call.fail(error, true),
            },
        };
        let task = cx.background_executor().spawn(async move {
            match edit {
                None => board::load_board(&dir).map(|tasks| (tasks, None, false)),
                Some(edit) => board::update_board(&dir, |tasks| {
                    let (next, touched) = board::apply(tasks, &edit, crate::app::now_ms())?;
                    Ok((next.clone(), (next, touched, true)))
                }),
            }
        });
        let bot = caller.bot;
        let action = action.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| match result {
                Ok((tasks, touched, changed)) => {
                    let value = match (action.as_str(), touched) {
                        ("board.list", _) => this.board_view(&tasks),
                        (_, Some(task)) => json!({ "task": this.task_view(&task) }),
                        (_, None) => json!({ "removed": true }),
                    };
                    call.answer(value);
                    if changed {
                        this.refresh_bot_overview(&bot, cx);
                    }
                }
                Err(error) => call.fail(error, true),
            });
            if let Err(err) = landed {
                log::debug!("board call after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// `board.add`, `board.update` and `board.remove`, checked.
    fn board_edit(
        &self,
        caller: &Caller,
        action: &str,
        input: &Map<String, Value>,
        request_id: &str,
    ) -> Result<Edit, String> {
        match action {
            "board.add" => {
                if !is_id(request_id) {
                    return Err("Invalid request ID".into());
                }
                let title = required(input.get("title"), "title", TITLE_MAX)?;
                let prompt =
                    optional(input.get("prompt"), "prompt", PROMPT_MAX)?.unwrap_or_default();
                let project = self.require_project(caller, input)?;
                let session_id = optional(input.get("sessionId"), "sessionId", 256)?;
                if let Some(session) = &session_id
                    && !self
                        .project_sessions(&project)
                        .iter()
                        .any(|s| &s.id == session)
                {
                    return Err("Session was not found in this project".into());
                }
                Ok(Edit::Add {
                    // The request names the task: a retry finds the one it made.
                    id: format!("task-{request_id}"),
                    title,
                    prompt,
                    project,
                    session_id,
                })
            }
            "board.update" => {
                let done = match input.get("done") {
                    None => None,
                    some => Some(flag(some, "done", false)?),
                };
                Ok(Edit::Update {
                    id: required(input.get("id"), "id", 256)?,
                    title: optional(input.get("title"), "title", TITLE_MAX)?,
                    prompt: optional(input.get("prompt"), "prompt", PROMPT_MAX)?,
                    done,
                })
            }
            "board.remove" => Ok(Edit::Remove {
                id: required(input.get("id"), "id", 256)?,
            }),
            _ => Err(format!("Unknown app action: {action}")),
        }
    }

    /// `board.start`: the task's session, launched as `sessions.start`
    /// would with the task's prompt and project, then linked to it.
    fn start_board_task(
        &mut self,
        caller: Caller,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let id = match required(input.get("id"), "id", 256) {
            Ok(id) => id,
            Err(error) => return call.fail(error, true),
        };
        let Some(dir) = bot_files_dir(&caller.bot) else {
            return call.fail("This bot's folder is unavailable", true);
        };
        let read = cx
            .background_executor()
            .spawn(async move { board::load_board(&dir) });
        cx.spawn(async move |this, cx| {
            let tasks = read.await;
            let landed = this.update(cx, |this, cx| {
                let task = match tasks {
                    Ok(tasks) => tasks.into_iter().find(|t| t.id == id),
                    Err(error) => return call.fail(error, true),
                };
                let Some(task) = task else {
                    return call.fail("No task with that id; run board.list", true);
                };
                if task.done_at.is_some() {
                    return call.fail("That task is done; reopen it with board.update first", true);
                }
                // A retry of this call finds the session it made.
                let retry = format!("app-{}-{}", caller.session, call.request.request_id);
                if let Some(session) = task.session_id.as_ref().filter(|s| **s != retry) {
                    return call.fail(
                        format!(
                            "That task already has session {session}; follow up with sessions.send"
                        ),
                        true,
                    );
                }
                let mut start = Map::new();
                start.insert("prompt".into(), json!(task.brief()));
                start.insert("project".into(), json!(task.project));
                start.insert("taskId".into(), json!(task.id));
                for key in START_OPTIONS {
                    if let Some(value) = input.get(key) {
                        start.insert(key.into(), value.clone());
                    }
                }
                this.start_session(caller, start, call, cx);
            });
            if let Err(err) = landed {
                log::debug!("board start after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The task `board.start` launched now names its session.
    pub(crate) fn link_board_task(
        &mut self,
        bot: &str,
        task: &str,
        session: &str,
        cx: &mut Context<Self>,
    ) {
        let edit = Edit::Link {
            id: task.to_string(),
            session_id: session.to_string(),
        };
        self.edit_bot_board(bot, edit, cx);
    }

    /// The Board tab's actions on the user's side.
    pub fn set_board_task_done(&mut self, task: &str, done: bool, cx: &mut Context<Self>) {
        let Some(bot) = self.bots.view.clone() else {
            return;
        };
        let edit = Edit::Update {
            id: task.to_string(),
            title: None,
            prompt: None,
            done: Some(done),
        };
        self.edit_bot_board(&bot, edit, cx);
    }

    pub fn remove_board_task(&mut self, task: &str, cx: &mut Context<Self>) {
        let Some(bot) = self.bots.view.clone() else {
            return;
        };
        self.edit_bot_board(
            &bot,
            Edit::Remove {
                id: task.to_string(),
            },
            cx,
        );
    }

    pub fn toggle_board_done(&mut self, cx: &mut Context<Self>) {
        self.bots.overview.done_open = !self.bots.overview.done_open;
        cx.notify();
    }

    fn edit_bot_board(&mut self, bot: &str, edit: Edit, cx: &mut Context<Self>) {
        let Some(dir) = bot_files_dir(bot) else {
            return;
        };
        let task = cx.background_executor().spawn(async move {
            board::update_board(&dir, |tasks| {
                let (next, _) = board::apply(tasks, &edit, crate::app::now_ms())?;
                Ok((next, ()))
            })
        });
        let bot = bot.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                this.bots.overview.board_error = result.err().inspect(|err| {
                    log::warn!("could not change the bot's board: {err}");
                });
                this.refresh_bot_overview(&bot, cx);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("board changed after app drop: {err:#}");
            }
        })
        .detach();
    }
}
