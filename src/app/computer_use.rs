//! Desktop access is granted to one local turn. Closing the prompt, ending
//! the turn, or stopping Computer Use revokes its outstanding calls.

use std::sync::Arc;
use std::time::Duration;

use bencode_core::mcp::McpLaunch;
use gpui::{Context, Window};

use super::BenCodeApp;
use crate::computer_use::{self, Action, Desktop, Grant};
use crate::hotkey::{self, Shortcut};
use crate::mcp::bridge::{ToolCall, ToolReply, token_matches};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
pub(crate) struct ComputerUse {
    request: Option<Approval>,
    active: Option<Access>,
    desktop: Arc<Desktop>,
    stop_listening: bool,
    stop_registered: bool,
}

/// A turn's request for the desktop: every call it made while the prompt
/// was up waits on the one answer.
struct Approval {
    turn: u64,
    label: String,
    waiting: Vec<(Action, ToolCall)>,
}

impl Approval {
    fn answer_all(self, text: &str) {
        for (_, call) in self.waiting {
            call.answer(ToolReply::error(text));
        }
    }
}

struct Access {
    turn: u64,
    label: String,
    grant: Arc<Grant>,
}

impl Drop for ComputerUse {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl ComputerUse {
    pub(crate) fn prompt(&self) -> Option<(u64, &str)> {
        self.request
            .as_ref()
            .map(|request| (request.turn, request.label.as_str()))
    }

    pub(crate) fn active_label(&self) -> Option<&str> {
        self.active.as_ref().map(|access| access.label.as_str())
    }

    pub(super) fn revoke(&mut self) {
        if std::mem::take(&mut self.stop_registered) {
            hotkey::unregister(Shortcut::StopComputerUse);
        }
        if let Some(access) = self.active.take() {
            access.grant.revoke();
        }
        if let Some(request) = self.request.take() {
            request.answer_all("Desktop access was stopped.");
        }
    }
}

impl BenCodeApp {
    pub(super) fn computer_use_launch(&self) -> Option<(McpLaunch, String)> {
        if !self.computer_use_enabled {
            return None;
        }
        let socket = self.tool_bridge_socket()?;
        match computer_use::launch(socket) {
            Ok(launch) => Some((launch, crate::app_cli::random_hex(32))),
            Err(err) => {
                log::warn!("Computer Use could not start: {err:#}");
                None
            }
        }
    }

    pub(crate) fn set_computer_use(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.computer_use_enabled = enabled;
        if !enabled {
            self.computer_use.revoke();
            for run in self.runs.values_mut() {
                run.computer_turn = None;
            }
        }
        self.save_settings(cx);
        cx.notify();
    }

    pub(crate) fn stop_computer_use(&mut self, cx: &mut Context<Self>) {
        let turn = self
            .computer_use
            .active
            .as_ref()
            .map(|access| access.turn)
            .or_else(|| {
                self.computer_use
                    .request
                    .as_ref()
                    .map(|request| request.turn)
            });
        if let Some(turn) = turn {
            self.reject_computer_turn(turn);
            self.computer_use.revoke();
            cx.notify();
        }
    }

    pub(super) fn end_computer_use(&mut self, turn: u64, cx: &mut Context<Self>) {
        if self
            .computer_use
            .active
            .as_ref()
            .is_some_and(|access| access.turn == turn)
            || self
                .computer_use
                .request
                .as_ref()
                .is_some_and(|request| request.turn == turn)
        {
            self.computer_use.revoke();
            cx.notify();
        }
    }

    pub(super) fn handle_computer_call(
        &mut self,
        mut call: ToolCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let credential = call.request.computer_turn.take().unwrap_or_default();
        let run = self.runs.iter().find(|(_, run)| {
            run.computer_turn
                .as_ref()
                .is_some_and(|token| token_matches(token, &credential))
                && run.outcome.is_none()
        });
        let Some((session, run)) = run.filter(|_| self.computer_use_enabled) else {
            return call.answer(ToolReply::error("This turn has no desktop access. Start a new local turn with Computer Use enabled in Settings → Integrations."));
        };
        let turn = run.id;
        let action = match Action::parse(&call.request.tool, std::mem::take(&mut call.request.args))
        {
            Ok(action) => action,
            Err(err) => return call.answer(ToolReply::error(format!("{err:#}"))),
        };
        if let Some(access) = &self.computer_use.active {
            if access.turn != turn {
                return call.answer(ToolReply::error(
                    "Another turn is using the desktop. Try again after it finishes.",
                ));
            }
            self.run_computer_action(turn, access.grant.clone(), action, call, cx);
            return;
        }
        if let Some(request) = &mut self.computer_use.request {
            // Calls made side by side wait on the same prompt.
            if request.turn == turn {
                request.waiting.push((action, call));
                return;
            }
            return call.answer(ToolReply::error(
                "BenCode is already waiting for a desktop approval.",
            ));
        }
        let label = self
            .sessions
            .iter()
            .find(|row| &row.id == session)
            .map(super::session_list::session_title)
            .unwrap_or_else(|| "Agent".into());
        self.computer_use.request = Some(Approval {
            turn,
            label,
            waiting: vec![(action, call)],
        });
        self.announce_computer_use(session, cx);
        // With notifications off, the Dock still says someone must answer.
        if !super::frame_bench::enabled() && !super::snapshots_enabled() {
            window.request_attention();
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(APPROVAL_TIMEOUT).await;
            if let Err(err) = this.update(cx, |this, cx| this.deny_computer_use(turn, cx)) {
                log::debug!("Computer Use approval expired after app drop: {err:#}");
            }
        })
        .detach();
    }

    pub(crate) fn deny_computer_use(&mut self, turn: u64, cx: &mut Context<Self>) {
        if let Some(request) = self.take_computer_request(turn) {
            self.reject_computer_turn(turn);
            request.answer_all("Desktop access was declined or the approval expired.");
            cx.notify();
        }
    }

    pub(crate) fn allow_computer_use(&mut self, turn: u64, cx: &mut Context<Self>) {
        let Some(request) = self.take_computer_request(turn) else {
            return;
        };
        let grant = Arc::new(Grant::default());
        self.computer_use.active = Some(Access {
            turn,
            label: request.label,
            grant: grant.clone(),
        });
        self.register_desktop_stop(cx);
        cx.notify();

        // Clicking Allow left BenCode in front: keys, text and clicks
        // would land in BenCode, not where the agent meant them.
        let (replay, refused): (Vec<_>, Vec<_>) = request
            .waiting
            .into_iter()
            .partition(|(action, _)| !action.follows_focus());
        for (_, call) in refused {
            call.answer(ToolReply::error(
                "Desktop access was approved for this turn, but this action was not run because \
                 BenCode is in front after the approval. Bring the intended app forward \
                 (computer_open_app or computer_window focus), take a new screenshot, then retry.",
            ));
        }
        if replay.is_empty() {
            return;
        }
        // Let the permission dialog leave the window before the agent takes
        // its first screenshot.
        cx.spawn(async move |entity, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            if let Err(err) = entity.update(cx, |this, cx| {
                for (action, call) in replay {
                    this.run_computer_action(turn, grant.clone(), action, call, cx);
                }
            }) {
                log::debug!("Computer Use approved after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn register_desktop_stop(&mut self, cx: &mut Context<Self>) {
        if super::frame_bench::enabled() || super::snapshots_enabled() {
            return;
        }
        if !self.computer_use.stop_listening {
            let Some(mut presses) = hotkey::listen(Shortcut::StopComputerUse) else {
                return;
            };
            self.computer_use.stop_listening = true;
            cx.spawn(async move |entity, cx| {
                while presses.recv().await.is_some() {
                    if entity
                        .update(cx, |app, cx| app.stop_computer_use(cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        match hotkey::register(
            Shortcut::StopComputerUse,
            hotkey::PERIOD,
            hotkey::COMMAND | hotkey::OPTION,
        ) {
            Ok(()) => self.computer_use.stop_registered = true,
            Err(status) => {
                log::warn!("Computer Use global stop shortcut is unavailable: OSStatus {status}")
            }
        }
    }

    fn reject_computer_turn(&mut self, turn: u64) {
        if let Some(run) = self.runs.values_mut().find(|run| run.id == turn) {
            run.computer_turn = None;
        }
    }

    fn take_computer_request(&mut self, turn: u64) -> Option<Approval> {
        if self.computer_use.request.as_ref()?.turn != turn {
            return None;
        }
        self.computer_use.request.take()
    }

    fn run_computer_action(
        &self,
        turn: u64,
        grant: Arc<Grant>,
        action: Action,
        call: ToolCall,
        cx: &mut Context<Self>,
    ) {
        let desktop = self.computer_use.desktop.clone();
        cx.background_executor()
            .spawn(async move {
                call.answer(desktop.perform(turn, &grant, action));
            })
            .detach();
    }
}
