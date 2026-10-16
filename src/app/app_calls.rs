//! `bencode app` on the app (MonoCode `App.tsx`'s agent-app host and
//! `handleAgentApp`): the socket the CLI reaches, the credential each
//! Mono's turn carries, and every call, checked for who sent it and then
//! handed to its domain: `artifacts.rs`, `app_cli/memory.rs` (soul and
//! memory), `app_cli/notes.rs`, `habits.rs`, `mono_board.rs`,
//! `mono_sessions.rs` (sessions, worktrees, folders) and `mono_cards.rs`.
//!
//! A token belongs to one conversation and works only while a turn of it
//! runs (MonoCode `APP_TURN_INACTIVE`): a Mono's own chat, or a run of one
//! of its habits.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{Context, Subscription};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::app::BenCodeApp;
use crate::app_cli::server::{self, AppCall};
use bencode_core::harness::{AppAccess, HarnessKind, catalog};

/// MonoCode `APP_TURN_INACTIVE`.
const TURN_INACTIVE: &str = "BenCode app access is inactive: it works only during a Mono's \
    running turn. Retrying this request now will not enable access.";

pub struct AppAccessState {
    /// Where `bencode app` reaches this app; None when the socket could not
    /// be opened.
    pub(crate) socket: Option<PathBuf>,
    /// Each conversation's credential, made at its first turn.
    tokens: HashMap<String, String>,
    _subscriptions: Vec<Subscription>,
}

impl AppAccessState {
    pub fn new(cx: &mut Context<BenCodeApp>) -> Self {
        let (calls_tx, mut calls_rx) = mpsc::unbounded_channel::<AppCall>();
        let socket = match server::listen(calls_tx) {
            Ok(path) => Some(path),
            Err(err) => {
                log::error!("app access is off: {err:#}");
                None
            }
        };
        cx.spawn(async move |this, cx| {
            while let Some(call) = calls_rx.recv().await {
                if this.update(cx, |this, cx| this.handle_app_call(call, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut subscriptions = Vec::new();
        if let Some(path) = socket.clone() {
            subscriptions.push(cx.on_app_quit(move |_, _| {
                server::remove(&path);
                async {}
            }));
        }
        Self {
            socket,
            tokens: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }
}

/// Who a call comes from: the Mono it works for, and whether it is that
/// Mono's own conversation (else a habit's run).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Caller {
    pub session: String,
    pub mono: String,
    pub own: bool,
}

impl BenCodeApp {
    /// What a turn of `session_id` carries for `bencode app`: a Mono's own
    /// conversation or a habit's run (MonoCode: "App access is always
    /// enabled in this thread"), on this computer.
    pub(crate) fn app_access_for(&mut self, session_id: &str, cwd: &str) -> Option<AppAccess> {
        if !self.has_app_access(session_id) || crate::remote::is_remote(cwd) {
            return None;
        }
        let socket = self.app_access.socket.as_ref()?.to_string_lossy().into_owned();
        let token = self
            .app_access
            .tokens
            .entry(session_id.to_string())
            .or_insert_with(|| crate::app_cli::random_hex(32))
            .clone();
        Some(AppAccess { socket, token })
    }

    /// Whether `session_id`'s turns get `bencode app`, while the socket is
    /// open.
    pub(crate) fn has_app_access(&self, session_id: &str) -> bool {
        self.app_access.socket.is_some() && self.app_caller(session_id).is_some()
    }

    /// MonoCode `monoOf`: the Mono a session works for.
    pub(crate) fn app_caller(&self, session_id: &str) -> Option<Caller> {
        if let Some(mono) = self.mono_for_session(session_id) {
            return Some(Caller {
                session: session_id.to_string(),
                mono: mono.id.clone(),
                own: true,
            });
        }
        let mono = self.habit_run_mono(session_id)?;
        Some(Caller {
            session: session_id.to_string(),
            mono,
            own: false,
        })
    }

    /// One `bencode app` call: whose turn sent it, then the action.
    fn handle_app_call(&mut self, call: AppCall, cx: &mut Context<Self>) {
        let session = self
            .app_access
            .tokens
            .iter()
            .find(|(_, token)| **token == call.request.token)
            .map(|(session, _)| session.clone());
        let Some(caller) = session.and_then(|session| self.app_caller(&session)) else {
            call.fail("Connection revoked or unauthorized", false);
            return;
        };
        if !self.is_agent_running_in(&caller.session) {
            call.fail(TURN_INACTIVE, false);
            return;
        }
        let action = call.request.action.clone();
        let input = match call.request.input.as_object() {
            Some(input) => input.clone(),
            None => {
                call.fail("Input must be a JSON object", true);
                return;
            }
        };
        if let Err(error) = crate::app_cli::input::check_fields(&action, &input) {
            call.fail(error, true);
            return;
        }
        let domain = action.split('.').next().unwrap_or("");
        match domain {
            "artifacts" => self.artifact_call(caller.session, call, cx),
            "soul" if !caller.own => {
                call.fail("Only a Mono's own conversation can manage its soul", true)
            }
            "soul" | "memory" => self.mono_file_call(caller, &action, input, call, cx),
            "notes" => self.notes_call(caller, &action, &input, call, cx),
            "models" => call.answer(models_listing(&self.harnesses)),
            "habits" if !caller.own => call.fail(
                "Only a Mono's own conversation can manage its habits",
                true,
            ),
            "habits" => self.habits_call(caller, &action, input, call, cx),
            "board" if !caller.own => call.fail(
                "Only a Mono's own conversation can manage its board",
                true,
            ),
            "board" => self.board_call(caller, &action, input, call, cx),
            "sessions" | "worktrees" | "folders" => {
                self.mono_sessions_call(caller, &action, input, call, cx)
            }
            "chat" => self.chat_card_call(caller, input, call, cx),
            _ => call.fail(format!("Unknown app action: {action}"), true),
        }
    }

    /// `soul.*` and `memory.*`: the Mono's files, off the UI thread; the
    /// views reload them after a change.
    fn mono_file_call(
        &mut self,
        caller: Caller,
        action: &str,
        input: Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = crate::app::mono_files::mono_files_dir(&caller.mono) else {
            call.fail("This Mono's folder is unavailable", true);
            return;
        };
        let action = action.to_string();
        let writes = !matches!(
            action.as_str(),
            "soul.read" | "memory.read" | "memory.search"
        );
        let task = cx.background_executor().spawn(async move {
            if action.starts_with("soul.") {
                crate::app_cli::memory::soul(&dir, &action, &input)
            } else {
                crate::app_cli::memory::memory(&dir, &action, &input, jiff::Zoned::now())
            }
        });
        let mono = caller.mono;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let ok = result.is_ok();
            match result {
                Ok(value) => call.answer(value),
                Err(error) => call.fail(error, true),
            }
            if !(ok && writes) {
                return;
            }
            let landed = this.update(cx, |this, cx| this.mono_files_changed(&mono, cx));
            if let Err(err) = landed {
                log::debug!("Mono files changed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// `notes.*`, on the database thread.
    fn notes_call(
        &mut self,
        caller: Caller,
        action: &str,
        input: &Map<String, Value>,
        call: AppCall,
        cx: &mut Context<Self>,
    ) {
        let parsed = match crate::app_cli::notes::parse(action, input) {
            Ok(parsed) => parsed,
            Err(error) => {
                call.fail(error, true);
                return;
            }
        };
        let writes = matches!(parsed, crate::app_cli::notes::Call::Write(_));
        // A Mono works from the home folder, no one project.
        let cwd = self
            .sessions
            .iter()
            .find(|s| s.id == caller.session)
            .map(|s| s.work_dir().to_string())
            .filter(|cwd| !self.is_mono_session(&caller.session) && crate::app::notes::is_project_folder(cwd));
        let request_id = call.request.request_id.clone();
        self.db_then(
            cx,
            move |db| crate::app_cli::notes::run(db, &parsed, &caller.session, cwd.as_deref(), &request_id),
            move |this, result, cx| {
                match result {
                    Ok(value) => call.answer(value),
                    Err(err) => call.fail(format!("{err:#}"), true),
                }
                if writes {
                    this.reload_notes(cx);
                }
            },
        );
    }
}

/// MonoCode `models.list`: the permission modes and every harness's
/// models with their settings.
fn models_listing(installed: &[bencode_core::harness::HarnessInfo]) -> Value {
    let modes: Vec<Value> = crate::ui::composer::permission_modes()
        .iter()
        .map(|(mode, label, hint, _)| json!({ "id": mode.id(), "label": label, "description": hint }))
        .collect();
    let harnesses: Vec<Value> = crate::harness::ALL_HARNESSES
        .iter()
        .map(|kind: &HarnessKind| {
            let available = installed
                .iter()
                .any(|info| info.id == kind.id() && info.available);
            let models: Vec<Value> = catalog::models_for(*kind)
                .iter()
                .map(|model| {
                    let settings: Vec<Value> = model
                        .settings
                        .iter()
                        .map(|setting| {
                            json!({
                                "id": setting.id,
                                "label": setting.label,
                                "default": setting.default,
                                "options": setting
                                    .options
                                    .iter()
                                    .map(|(value, label)| json!({ "value": value, "label": label }))
                                    .collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    json!({ "id": model.key, "name": model.label, "settings": settings })
                })
                .collect();
            json!({ "id": kind.id(), "available": available, "models": models })
        })
        .collect();
    json!({ "runtimeModes": modes, "harnesses": harnesses })
}

impl BenCodeApp {
    /// MonoCode `chat.card`: its app never hands the CLI a way to post one
    /// (`AgentAppHost.postCard` is not given), so the call is refused as
    /// MonoCode refuses it.
    pub(crate) fn chat_card_call(&mut self, _caller: Caller, _input: Map<String, Value>, call: AppCall, _cx: &mut Context<Self>) {
        call.fail("Only a Mono can put cards in its chat", true);
    }
}
