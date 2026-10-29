//! MonoCode's git pill (`conversation/git_actions.rs`): under a thread whose
//! own edits are still uncommitted, one click asks the thread's agent to
//! stage, commit or push exactly what it changed, leaving other threads'
//! and the user's work in the checkout alone. The agent, which knows what
//! it did, picks the files and writes the message; the transcript shows the
//! short ask ("Commit your changes") and, once the turn ends, the commits it
//! made, read from git rather than from the agent's words, so every harness
//! gets the same summary. The pill then offers the next step: staged work
//! offers Commit, an unpushed commit offers Push. `/stage` and `/commit`
//! ask the same without the pill.
//!
//! The turn is the user's own click, shown in the thread like any message;
//! BenCode starts no turn of its own.

use std::sync::Arc;

use bencode_ui::primitives::IconName;
use gpui::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::{BenCodeApp, TurnInput};
use crate::db::Block;
use crate::git::session_git::{self, SessionChanges, SessionCommit};

/// The user block's field naming the action it asked for.
const ACTION_FIELD: &str = "gitAction";
/// HEAD when it was sent: the action's commits are counted from it.
const HEAD_FIELD: &str = "gitHead";
/// What the action's turn did, once it ended (`GitTurn`).
const TURN_FIELD: &str = "gitTurn";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitAction {
    Stage,
    #[default]
    Commit,
    CommitPush,
    /// The step after a commit, never the pill's habit.
    Push,
}

impl GitAction {
    /// What the pill's menu offers.
    pub const ALL: [Self; 3] = [Self::Stage, Self::Commit, Self::CommitPush];
    const EVERY: [Self; 4] = [Self::Stage, Self::Commit, Self::CommitPush, Self::Push];

    pub fn id(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Commit => "commit",
            Self::CommitPush => "commit-push",
            Self::Push => "push",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::EVERY.into_iter().find(|action| action.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Commit => "Commit",
            Self::CommitPush => "Commit & push",
            Self::Push => "Push",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Stage => "Stage only this thread's changes",
            Self::Commit => "Stage and commit with a written message",
            Self::CommitPush => "Commit, then push the branch",
            Self::Push => "Push this thread's commits",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Stage => IconName::Plus,
            Self::Commit => IconName::GitCommitHorizontal,
            Self::CommitPush | Self::Push => IconName::CloudUpload,
        }
    }

    /// What the thread shows for the turn.
    pub fn message(self) -> &'static str {
        match self {
            Self::Stage => "Stage your changes",
            Self::Commit => "Commit your changes",
            Self::CommitPush => "Commit and push your changes",
            Self::Push => "Push your commits",
        }
    }

    /// MonoCode's prompt: what the agent reads instead.
    pub fn prompt(self) -> String {
        if self == Self::Push {
            return "Push the current branch with git.\n\nDo not stage, commit, amend or change anything else. Use `git push`, or `git push -u origin <branch>` when the branch has no upstream. Never force-push. If the push is rejected, stop and report it; do not pull, rebase or merge.\n\nFinish with a short reply saying where it was pushed.".into();
        }
        let task = match self {
            Self::Stage => "Stage the changes you made in this session with git. Do not commit.",
            Self::Commit => "Commit the changes you made in this session with git.",
            Self::CommitPush | Self::Push => {
                "Commit the changes you made in this session with git, then push the branch."
            }
        };
        let mut rules = vec![
            "Work out which files you created, edited, renamed or deleted in this session from this conversation, then check them against `git status` and `git diff`.",
            "Name every path explicitly. Never use `git add .`, `git add -A`, `git add -u`, `git commit -a` or any other repository-wide form.",
            "If a file also holds changes that are not yours, take only your own hunks when you can separate them reliably (for example with `git apply --cached`). Otherwise leave that file out and say why.",
            "Leave everything else as it is: do not unstage, stash, reset, check out, or discard any other change.",
        ];
        if self != Self::Stage {
            rules.extend([
                "Commit only your paths, listing them explicitly (`git commit -m \"…\" -- <paths>`), so anything someone else staged stays out of the commit. If you staged partial hunks, check that `git diff --cached` holds nothing but your changes and commit the index instead.",
                "Write the message from what you did and why: a short summary line, then a body only if the change needs explaining. Follow the style of `git log`.",
                "Do not amend, and do not skip hooks. If a hook fails because of your change, fix it and retry; otherwise stop and report the failure.",
            ]);
        }
        rules.push(match self {
            Self::Stage | Self::Commit => "Do not push.",
            Self::CommitPush | Self::Push => {
                "Then push the current branch (`git push`, or `git push -u origin <branch>` when it has no upstream). Never force-push. If the push is rejected, stop and report it; do not pull, rebase or merge."
            }
        });
        let report = match self {
            Self::Stage => "the output of `git diff --cached --stat`",
            Self::Commit => "the commit's short hash and summary line, and the files it includes",
            Self::CommitPush | Self::Push => {
                "the commit's short hash and summary line, the files it includes, and where it was pushed"
            }
        };
        format!(
            "{task}\n\nOther agents and the user may be working in this same checkout, so the working tree can hold changes that are not yours. Include only what you changed:\n\n{}\n\nIf you changed nothing in this session, say so and stop. Otherwise finish with a short reply listing {report}, plus any changes you deliberately left out.",
            rules
                .iter()
                .map(|rule| format!("- {rule}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    /// The action a sent user block asked for.
    pub fn of_block(block: &Block) -> Option<Self> {
        Self::from_id(block.extra.get(ACTION_FIELD)?.as_str()?)
    }
}

/// What a git action's turn did, kept on its user block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitTurn {
    #[serde(default)]
    pub commits: Vec<SessionCommit>,
    /// The upstream the commits reached (`origin/main`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed: Option<String>,
    /// The repository's GitHub page, for the commits' links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
}

impl GitTurn {
    pub fn of_block(block: &Block) -> Option<Self> {
        serde_json::from_value(block.extra.get(TURN_FIELD)?.clone()).ok()
    }
}

/// A thread's git state, as the pill reads it.
#[derive(Clone, Debug, Default)]
pub struct SessionGit {
    pub changes: SessionChanges,
    /// The last commit a git action made is not on a remote yet.
    pub unpushed: bool,
    /// HEAD when last read: what a new action's commits are counted from.
    pub head: Option<String>,
    /// Read at least once since the app started.
    pub loaded: bool,
    generation: u64,
}

impl SessionGit {
    /// No read was started yet.
    pub fn unread(&self) -> bool {
        !self.loaded && self.generation == 0
    }
}

/// The files the thread's completed edit tools wrote (`paths`, which
/// BenCode records on an edit tool's block).
pub fn session_edit_paths(blocks: &[Arc<Block>]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for tool in blocks.iter().filter_map(|b| b.tool.as_ref()) {
        if tool.get("status").and_then(Value::as_str) != Some("completed") {
            continue;
        }
        let listed = tool
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for path in listed.filter_map(Value::as_str) {
            if !path.trim().is_empty() && !paths.iter().any(|p| p == path) {
                paths.push(path.to_string());
            }
        }
    }
    paths
}

/// The newest commit a git action's turn made.
fn latest_commit(blocks: &[Arc<Block>]) -> Option<String> {
    blocks
        .iter()
        .rev()
        .filter_map(|b| GitTurn::of_block(b))
        .find_map(|turn| turn.commits.last().map(|c| c.sha.clone()))
}

/// The last git action whose turn ended without its summary yet: its
/// block's id, the action, and the HEAD it was sent at.
fn pending_turn(blocks: &[Arc<Block>]) -> Option<(String, GitAction, String)> {
    let block = blocks.iter().rev().find(|b| b.role == "user")?;
    if block.extra.contains_key(TURN_FIELD) {
        return None;
    }
    let action = GitAction::of_block(block)?;
    let head = block.extra.get(HEAD_FIELD)?.as_str()?.to_string();
    Some((block.id.clone(), action, head))
}

/// What a read found.
struct GitRead {
    changes: SessionChanges,
    unpushed: bool,
    head: Option<String>,
    turn: Option<(String, GitTurn)>,
}

/// Reads `paths`' state, and the pending action's commits: blocking.
fn read_git(
    cwd: &str,
    paths: &[String],
    latest: Option<String>,
    pending: Option<(String, GitAction, String)>,
) -> GitRead {
    let changes = session_git::session_changes(cwd, paths);
    let turn = pending.map(|(block_id, action, base)| {
        let commits = session_git::commits_since(cwd, &base).unwrap_or_else(|err| {
            log::warn!("could not read the commits of a git action: {err:#}");
            Vec::new()
        });
        let pushes = matches!(action, GitAction::CommitPush | GitAction::Push);
        let reached = |sha: &str| !session_git::unpushed(cwd, sha);
        let pushed = commits
            .last()
            .map(|c| c.sha.clone())
            .or_else(|| latest.clone())
            .filter(|sha| pushes && reached(sha))
            .and_then(|_| session_git::upstream(cwd));
        let web = (!commits.is_empty())
            .then(|| session_git::github_web(cwd))
            .flatten();
        (
            block_id,
            GitTurn {
                commits,
                pushed,
                web,
            },
        )
    });
    let newest = turn
        .as_ref()
        .and_then(|(_, turn)| turn.commits.last().map(|c| c.sha.clone()))
        .or(latest);
    GitRead {
        changes,
        unpushed: newest.is_some_and(|sha| session_git::unpushed(cwd, &sha)),
        head: session_git::head(cwd),
        turn,
    }
}

impl BenCodeApp {
    /// The pill and the commands can ask the agent now: the thread is idle,
    /// nothing waits to be sent, and it runs here (a machine's or a Bot's
    /// thread has no pill).
    pub fn git_actions_ready(&self, session_id: &str) -> bool {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return false;
        };
        let thread = self.thread(session_id);
        !self.is_agent_running_in(session_id)
            && !self.is_bot_session(session_id)
            && !crate::remote::is_remote(session.work_dir())
            && !self.worktree_removed(session_id)
            && thread
                .is_none_or(|t| t.queue.is_empty() && !t.edit_rewinding && !t.preparing_worktree)
    }

    /// The pill shows: the thread's own work is uncommitted or unpushed.
    pub fn git_pill_shown(&self, session_id: &str) -> bool {
        self.thread(session_id)
            .is_some_and(|t| t.git.changes.uncommitted() || t.git.unpushed)
            && self.git_actions_ready(session_id)
    }

    /// The next steps, the pill's own first: unstaged work offers every
    /// action, staged work skips Stage, a committed but unpushed thread
    /// offers only Push.
    pub fn git_offers(&self, session_id: &str) -> Vec<GitAction> {
        let Some(git) = self.thread(session_id).map(|t| &t.git) else {
            return Vec::new();
        };
        let mut offers = if git.changes.unstaged {
            GitAction::ALL.to_vec()
        } else if git.changes.staged {
            vec![GitAction::Commit, GitAction::CommitPush]
        } else if git.unpushed {
            return vec![GitAction::Push];
        } else {
            return Vec::new();
        };
        let current = Some(self.last_git_action)
            .filter(|last| offers.contains(last))
            .unwrap_or(GitAction::Commit);
        offers.retain(|action| *action != current);
        offers.insert(0, current);
        offers
    }

    /// A pick from the pill or its menu.
    pub fn run_git_action(&mut self, session_id: &str, action: GitAction, cx: &mut Context<Self>) {
        if !self.git_pill_shown(session_id) || !self.git_offers(session_id).contains(&action) {
            return;
        }
        self.start_git_action(session_id, action, cx);
    }

    /// The command palette's and the menu's actions on the focused thread:
    /// as the pill, also while it is hidden (the agent says when it changed
    /// nothing).
    pub fn git_action_command(&mut self, action: GitAction, cx: &mut Context<Self>) {
        let Some(id) = self.selected_session_id.clone() else {
            return;
        };
        if self.git_actions_ready(&id) {
            self.start_git_action(&id, action, cx);
        }
    }

    /// Starts the turn at once, leaving the composer as it is: the thread
    /// shows the short ask, the agent reads the prompt.
    fn start_git_action(&mut self, session_id: &str, action: GitAction, cx: &mut Context<Self>) {
        if action != GitAction::Push {
            self.last_git_action = action;
        }
        let mut extra = serde_json::Map::new();
        extra.insert(ACTION_FIELD.into(), json!(action.id()));
        if let Some(head) = self.thread(session_id).and_then(|t| t.git.head.clone()) {
            extra.insert(HEAD_FIELD.into(), json!(head));
        }
        let input = TurnInput {
            text: action.message().to_string(),
            agent_prompt: Some(action.prompt()),
            extra,
            ..Default::default()
        };
        self.send_turn(session_id, input, cx);
        cx.notify();
    }

    /// Rereads where the thread's own changes stand, and once a git
    /// action's turn has ended, what it committed. A newer read wins.
    pub(crate) fn refresh_git_actions(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !self.git_actions_ready(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let cwd = session.work_dir().to_string();
        let mut paths = session_edit_paths(&session.blocks);
        // The card's files cover edits from before BenCode kept their paths.
        if let Some(review) = self.checkpoints.reviews.get(session_id) {
            for file in &review.files {
                if !paths.contains(&file.relative) {
                    paths.push(file.relative.clone());
                }
            }
        }
        let latest = latest_commit(&session.blocks);
        let pending = pending_turn(&session.blocks);
        let git = &mut self.thread_mut(session_id).git;
        git.generation += 1;
        let generation = git.generation;
        // The head is read even with nothing to check, for a `/commit`.
        let task = cx
            .background_executor()
            .spawn(async move { read_git(&cwd, &paths, latest, pending) });
        let id = session_id.to_string();
        cx.spawn(async move |this, cx| {
            let read = task.await;
            let landed = this.update(cx, |app, cx| app.land_git_read(&id, generation, read, cx));
            if let Err(err) = landed {
                log::debug!("git actions read after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn land_git_read(
        &mut self,
        session_id: &str,
        generation: u64,
        read: GitRead,
        cx: &mut Context<Self>,
    ) {
        let Some(git) = self.threads.get_mut(session_id).map(|t| &mut t.git) else {
            return;
        };
        if git.generation != generation {
            return;
        }
        let shown_before = (git.changes, git.unpushed);
        git.changes = read.changes;
        git.unpushed = read.unpushed;
        git.head = read.head;
        git.loaded = true;
        let changed = shown_before != (git.changes, git.unpushed);
        let stamped = read
            .turn
            .is_some_and(|(block_id, turn)| self.stamp_git_turn(session_id, &block_id, &turn));
        if stamped {
            self.persist_session(session_id);
        }
        if changed || stamped {
            self.redraw_thread(session_id, cx);
        }
    }

    /// Keeps what the action's turn did on its user block, unless a turn
    /// started since (it would be read against the wrong HEAD).
    fn stamp_git_turn(&mut self, session_id: &str, block_id: &str, turn: &GitTurn) -> bool {
        if self.is_agent_running_in(session_id) {
            return false;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return false;
        };
        let Some(block) = session
            .blocks
            .iter_mut()
            .rev()
            .find(|b| b.role == "user")
            .filter(|b| b.id == block_id && !b.extra.contains_key(TURN_FIELD))
        else {
            return false;
        };
        match serde_json::to_value(turn) {
            Ok(value) => {
                Arc::make_mut(block).extra.insert(TURN_FIELD.into(), value);
                true
            }
            Err(err) => {
                log::warn!("could not keep a git action's summary: {err}");
                false
            }
        }
    }

    /// Git moved in `cwd` (the watcher, a branch switch): the pills of the
    /// threads drawn there that edited or committed something follow.
    pub(crate) fn refresh_git_actions_in(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.work_dir() == cwd && self.transcripts.contains_key(&s.id))
            .filter(|s| {
                let reviewed = self
                    .checkpoints
                    .reviews
                    .get(&s.id)
                    .is_some_and(|review| !review.files.is_empty());
                reviewed
                    || !session_edit_paths(&s.blocks).is_empty()
                    || latest_commit(&s.blocks).is_some()
            })
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            self.refresh_git_actions(&id, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(path: &str, status: &str) -> Arc<Block> {
        let mut block = Block::new(format!("tool-{path}"), "tool", "");
        block.tool = Some(json!({ "kind": "edit", "status": status, "paths": [path] }));
        Arc::new(block)
    }

    fn user(id: &str, extra: Value) -> Arc<Block> {
        let mut block = Block::new(id, "user", "");
        if let Value::Object(map) = extra {
            block.extra = map;
        }
        Arc::new(block)
    }

    #[test]
    fn completed_edits_name_the_threads_files_once() {
        let blocks = vec![
            edit("/r/a.rs", "completed"),
            edit("/r/b.rs", "failed"),
            edit("/r/a.rs", "completed"),
            edit("/r/c.rs", "completed"),
        ];
        assert_eq!(session_edit_paths(&blocks), ["/r/a.rs", "/r/c.rs"]);
    }

    #[test]
    fn only_the_last_message_waits_for_its_summary() {
        let asked = user("u1", json!({ "gitAction": "commit", "gitHead": "abc" }));
        assert_eq!(
            pending_turn(std::slice::from_ref(&asked)),
            Some(("u1".into(), GitAction::Commit, "abc".into()))
        );
        let done = user(
            "u1",
            json!({ "gitAction": "commit", "gitHead": "abc", "gitTurn": { "commits": [] } }),
        );
        assert_eq!(pending_turn(&[done]), None);
        assert_eq!(pending_turn(&[asked, user("u2", json!({}))]), None);
    }

    #[test]
    fn the_newest_commit_comes_from_the_last_summary_with_one() {
        let commit = |sha: &str| json!({ "sha": sha, "subject": "s" });
        let blocks = vec![
            user(
                "u1",
                json!({ "gitTurn": { "commits": [commit("a"), commit("b")] } }),
            ),
            user("u2", json!({ "gitTurn": { "commits": [] } })),
        ];
        assert_eq!(latest_commit(&blocks).as_deref(), Some("b"));
    }

    #[test]
    fn actions_round_trip_and_scope_the_agent_to_its_own_changes() {
        for action in GitAction::EVERY {
            assert_eq!(GitAction::from_id(action.id()), Some(action));
        }
        for action in GitAction::ALL {
            let prompt = action.prompt();
            assert!(prompt.contains("Never use `git add .`"));
            assert!(prompt.contains("only what you changed"));
            assert_eq!(prompt.contains("-- <paths>"), action != GitAction::Stage);
            assert_eq!(
                prompt.contains("Never force-push"),
                action == GitAction::CommitPush
            );
        }
        assert!(GitAction::Stage.prompt().contains("Do not commit."));
        assert!(GitAction::Push.prompt().contains("Never force-push"));
    }
}
