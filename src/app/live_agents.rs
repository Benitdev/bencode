//! MonoCode `features/sessions/model/liveAgents.ts`: the threads the
//! Working agents card lists, across every project. A thread is listed
//! while its agent runs or waits on the user, and stays as "Done" after it
//! finishes until it is looked at.

use gpui::Context;

use crate::app::session_list::display_title;
use crate::app::{BenCodeApp, Surface};
use crate::db::{Block, SessionRow};
use crate::ui::transcript::activity::tool_label;
use crate::ui::transcript::turns::is_tool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAgent {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub harness: String,
    /// What the agent is doing: its last tool call, else "Working".
    pub activity: String,
    /// When the current turn started, and how long it took once done.
    pub started_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub needs_approval: bool,
    pub done: bool,
}

/// What the app knows about a thread's run.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunFlags {
    pub busy: bool,
    pub needs_approval: bool,
    pub unseen_finished: bool,
}

/// MonoCode `toLiveAgent`, for a thread in flight or finished unseen.
pub fn live_agent(session: &SessionRow, flags: RunFlags) -> Option<LiveAgent> {
    // MonoCode `isInFlightSession`.
    let in_flight = flags.busy && !session.worktree_removed;
    if !in_flight && !flags.unseen_finished {
        return None;
    }
    let done = !in_flight;
    let turn = session.blocks.iter().rev().find(|block| block.role == "user");
    Some(LiveAgent {
        id: session.id.clone(),
        cwd: session.cwd.clone(),
        title: display_title(&session.title, &session.harness),
        harness: session.harness.clone(),
        activity: if done {
            "Done".to_string()
        } else {
            activity_label(&session.blocks, &session.cwd)
        },
        started_at: turn.and_then(|block| block.started_at),
        duration_ms: turn.filter(|_| done).and_then(|block| block.duration_ms),
        needs_approval: in_flight && flags.needs_approval,
        done,
    })
}

/// MonoCode `compareLiveAgents`: those waiting on the user first, the
/// finished last, else the longest-running first.
pub fn sort_live_agents(agents: &mut [LiveAgent]) {
    agents.sort_by_key(|agent| {
        (
            !agent.needs_approval,
            agent.done,
            agent.started_at.unwrap_or(i64::MAX),
        )
    });
}

/// MonoCode `formatLiveElapsed`: "12s", "3m 6s", "1h 4m".
pub fn format_live_elapsed(elapsed_ms: i64) -> String {
    let seconds = ((elapsed_ms as f64) / 1000.0).round().max(1.0) as i64;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, rest) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return if rest == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {rest}s")
        };
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if rest == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {rest}m")
    }
}

/// MonoCode `activityLabel` over `lastActivityBlock`: the latest tool call
/// (or a handoff being prepared) as the transcript titles it.
fn activity_label(blocks: &[Block], cwd: &str) -> String {
    let preparing = |block: &Block| {
        block.role == "handoff"
            && block
                .extra
                .get("handoff")
                .and_then(|handoff| handoff.get("status"))
                .and_then(|status| status.as_str())
                == Some("preparing")
    };
    let Some(block) = blocks.iter().rev().find(|block| is_tool(block) || preparing(block)) else {
        return "Working".to_string();
    };
    if preparing(block) {
        return "Preparing a handoff".to_string();
    }
    let (verb, target, _) = tool_label(block, cwd);
    match verb {
        Some(verb) if !target.is_empty() => format!("{verb} {target}"),
        Some(verb) => verb.to_string(),
        None => target,
    }
}

impl BenCodeApp {
    /// MonoCode `liveAgentsFromSessions`; empty while the card is off.
    pub fn live_agents(&self) -> Vec<LiveAgent> {
        if self.live_agents_off {
            return Vec::new();
        }
        let unseen = &self.title_strip.unseen_finished;
        if self.runs.is_empty() && unseen.is_empty() {
            return Vec::new();
        }
        let mut agents: Vec<LiveAgent> = self
            .sessions
            .iter()
            .filter_map(|session| {
                let run = self.runs.get(&session.id);
                live_agent(
                    session,
                    RunFlags {
                        busy: run.is_some(),
                        needs_approval: run.is_some_and(|run| run.pending_permission.is_some()),
                        unseen_finished: unseen.contains(&session.id),
                    },
                )
            })
            .collect();
        sort_live_agents(&mut agents);
        agents
    }

    /// MonoCode `onSelectLiveAgent`: leaves the open surface and shows the
    /// thread, in whichever project it belongs to.
    pub fn select_live_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.surface.is_some_and(|surface| surface != Surface::Settings) {
            self.close_surface(cx);
        }
        self.open_session(id.to_string(), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str, blocks: Vec<Block>) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: "claude · Fix the rail".into(),
            cwd: "/repo".into(),
            harness: "claude".into(),
            blocks,
            ..Default::default()
        }
    }

    fn user(started_at: i64, duration_ms: Option<i64>) -> Block {
        Block {
            started_at: Some(started_at),
            duration_ms,
            ..Block::new("u", "user", "go")
        }
    }

    fn tool(kind: &str, title: &str) -> Block {
        Block {
            tool: Some(json!({ "kind": kind, "title": title })),
            ..Block::new("t", "tool", "")
        }
    }

    const BUSY: RunFlags = RunFlags {
        busy: true,
        needs_approval: false,
        unseen_finished: false,
    };

    #[test]
    fn idle_threads_are_not_listed() {
        assert_eq!(live_agent(&session("a", vec![]), RunFlags::default()), None);
    }

    #[test]
    fn a_running_thread_shows_its_last_tool_call() {
        let row = session("a", vec![user(100, None), tool("read", "/repo/src/main.rs")]);
        let agent = live_agent(&row, BUSY).unwrap();
        assert_eq!(agent.title, "Fix the rail");
        assert_eq!(agent.activity, "Read src/main.rs");
        assert_eq!(agent.started_at, Some(100));
        assert_eq!(agent.duration_ms, None);
        assert!(!agent.done && !agent.needs_approval);

        let fresh = live_agent(&session("b", vec![user(100, None)]), BUSY).unwrap();
        assert_eq!(fresh.activity, "Working");
    }

    #[test]
    fn a_finished_unseen_thread_is_done_with_its_duration() {
        let row = session("a", vec![user(100, Some(24_000)), tool("search", "fn main")]);
        let flags = RunFlags {
            unseen_finished: true,
            ..Default::default()
        };
        let agent = live_agent(&row, flags).unwrap();
        assert!(agent.done);
        assert_eq!(agent.activity, "Done");
        assert_eq!(agent.duration_ms, Some(24_000));
    }

    #[test]
    fn a_thread_whose_worktree_is_gone_is_not_in_flight() {
        let mut row = session("a", vec![user(100, None)]);
        row.worktree_removed = true;
        assert_eq!(live_agent(&row, BUSY), None);
    }

    #[test]
    fn approvals_come_first_and_the_finished_last() {
        let agent = |id: &str, needs_approval: bool, done: bool, started_at: i64| LiveAgent {
            id: id.into(),
            cwd: String::new(),
            title: String::new(),
            harness: String::new(),
            activity: String::new(),
            started_at: Some(started_at),
            duration_ms: None,
            needs_approval,
            done,
        };
        let mut agents = vec![
            agent("done", false, true, 1),
            agent("late", false, false, 30),
            agent("early", false, false, 10),
            agent("ask", true, false, 50),
        ];
        sort_live_agents(&mut agents);
        let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["ask", "early", "late", "done"]);
    }

    #[test]
    fn elapsed_times_grow_from_seconds_to_hours() {
        assert_eq!(format_live_elapsed(0), "1s");
        assert_eq!(format_live_elapsed(24_000), "24s");
        assert_eq!(format_live_elapsed(186_000), "3m 6s");
        assert_eq!(format_live_elapsed(120_000), "2m");
        assert_eq!(format_live_elapsed(3_840_000), "1h 4m");
        assert_eq!(format_live_elapsed(7_200_000), "2h");
    }
}
