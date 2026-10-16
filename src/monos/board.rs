//! A Mono's board: the work it splits a request into, one task per
//! session, kept in its folder's `board.json`. BenCode's own (MonoCode has
//! none). A task is a title and the prompt its session will get; once
//! started it is linked to that session, and where it sits (To do,
//! Running, Review, Done) follows from the session's state, except Done,
//! which someone decides: the Mono after reviewing its result, or the user.
//!
//! Every change is made over the version read (`update_board`); fields
//! this version does not model round-trip.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::files::{self, BOARD_FILE, SaveError};
use crate::app::mono_sessions::LaunchedState;

/// Open tasks a board holds at most.
pub const OPEN_MAX: usize = 50;
/// Finished tasks kept, newest first; older ones drop off.
pub const DONE_KEPT: usize = 30;
pub const TITLE_MAX: usize = 200;
/// A task's prompt is a brief; the whole file stays under 256 KB.
pub const PROMPT_MAX: usize = 8_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    /// What its session is told; the title when empty.
    #[serde(default)]
    pub prompt: String,
    /// The project it works in, by path.
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// When it was marked done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Task {
    /// The prompt its session starts with.
    pub fn brief(&self) -> &str {
        if self.prompt.trim().is_empty() {
            &self.title
        } else {
            &self.prompt
        }
    }
}

/// The board's columns, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    Todo,
    Running,
    Review,
    Done,
}

impl Column {
    pub const ALL: [Column; 4] = [Column::Todo, Column::Running, Column::Review, Column::Done];

    pub fn id(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Running => "running",
            Self::Review => "review",
            Self::Done => "done",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "To do",
            Self::Running => "Running",
            Self::Review => "Review",
            Self::Done => "Done",
        }
    }
}

/// Where a task sits, from its session's state (`None` when the session
/// is not loaded or was deleted: its work stopped, so it waits for review).
pub fn column(task: &Task, state: Option<LaunchedState>) -> Column {
    if task.done_at.is_some() {
        return Column::Done;
    }
    if task.session_id.is_none() {
        return Column::Todo;
    }
    match state {
        Some(LaunchedState::Working | LaunchedState::Input) => Column::Running,
        Some(LaunchedState::Draft) => Column::Todo,
        Some(LaunchedState::Failed | LaunchedState::Stopped | LaunchedState::Done) | None => {
            Column::Review
        }
    }
}

/// `board.json` as written; a malformed task is left out.
pub fn parse_board(text: &str) -> Vec<Task> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|raw| serde_json::from_value::<Task>(raw).ok())
        .filter(|task| !task.id.is_empty() && !task.title.trim().is_empty())
        .collect()
}

pub fn serialize_board(tasks: &[Task]) -> String {
    match serde_json::to_string_pretty(tasks) {
        Ok(text) => format!("{text}\n"),
        Err(err) => {
            log::error!("board did not serialize: {err}");
            "[]\n".to_string()
        }
    }
}

pub fn load_board(dir: &Path) -> Result<Vec<Task>, String> {
    let (text, _) = files::read(dir, BOARD_FILE).map_err(|err| format!("{err:#}"))?;
    Ok(parse_board(text.as_deref().unwrap_or("")))
}

/// `board.json` changed over the version read, again from the top when
/// another change landed in between.
pub fn update_board<T>(
    dir: &Path,
    change: impl Fn(Vec<Task>) -> Result<(Vec<Task>, T), String>,
) -> Result<T, String> {
    for attempt in 0.. {
        let (text, hash) = files::read(dir, BOARD_FILE).map_err(|err| format!("{err:#}"))?;
        let (tasks, result) = change(parse_board(text.as_deref().unwrap_or("")))?;
        match files::save(dir, BOARD_FILE, &serialize_board(&tasks), Some(&hash)) {
            Ok(_) => return Ok(result),
            Err(SaveError::Conflict) if attempt < 2 => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    unreachable!("the loop returns")
}

/// A change to the board, checked against the app before it reaches the
/// file.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Add {
        id: String,
        title: String,
        prompt: String,
        project: String,
        session_id: Option<String>,
    },
    Update {
        id: String,
        title: Option<String>,
        prompt: Option<String>,
        done: Option<bool>,
    },
    /// The session started for it.
    Link { id: String, session_id: String },
    Remove { id: String },
}

/// `edit` applied to `tasks`: the board after it, and the task it touched
/// (`None` once removed).
pub fn apply(mut tasks: Vec<Task>, edit: &Edit, now: i64) -> Result<(Vec<Task>, Option<Task>), String> {
    let find = |tasks: &[Task], id: &str| {
        tasks
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| "No task with that id; run board.list".to_string())
    };
    let touched = match edit {
        Edit::Add { id, title, prompt, project, session_id } => {
            if let Some(known) = tasks.iter().find(|t| &t.id == id) {
                // A retry of the same call.
                return Ok((tasks.clone(), Some(known.clone())));
            }
            if let Some(session) = session_id
                && tasks.iter().any(|t| t.session_id.as_ref() == Some(session) && t.done_at.is_none())
            {
                return Err("That session is already on the board".into());
            }
            if tasks.iter().filter(|t| t.done_at.is_none()).count() >= OPEN_MAX {
                return Err(format!(
                    "The board holds at most {OPEN_MAX} open tasks; finish or remove some first"
                ));
            }
            tasks.push(Task {
                id: id.clone(),
                title: title.clone(),
                prompt: prompt.clone(),
                project: project.clone(),
                session_id: session_id.clone(),
                done_at: None,
                created_at: now,
                extra: Map::new(),
            });
            tasks.len() - 1
        }
        Edit::Update { id, title, prompt, done } => {
            let index = find(&tasks, id)?;
            let task = &mut tasks[index];
            if let Some(title) = title {
                task.title = title.clone();
            }
            if let Some(prompt) = prompt {
                if task.session_id.is_some() {
                    return Err("This task's session has started; follow up with sessions.send".into());
                }
                task.prompt = prompt.clone();
            }
            match done {
                Some(true) if task.done_at.is_none() => task.done_at = Some(now),
                Some(false) => task.done_at = None,
                _ => {}
            }
            index
        }
        Edit::Link { id, session_id } => {
            let index = find(&tasks, id)?;
            tasks[index].session_id = Some(session_id.clone());
            index
        }
        Edit::Remove { id } => {
            let index = find(&tasks, id)?;
            tasks.remove(index);
            return Ok((tasks, None));
        }
    };
    let task = tasks[touched].clone();
    Ok((prune_done(tasks), Some(task)))
}

/// The open tasks in their order, then the newest [`DONE_KEPT`] done ones.
fn prune_done(tasks: Vec<Task>) -> Vec<Task> {
    let (open, mut done): (Vec<Task>, Vec<Task>) = tasks.into_iter().partition(|t| t.done_at.is_none());
    done.sort_by_key(|t| std::cmp::Reverse(t.done_at));
    done.truncate(DONE_KEPT);
    open.into_iter().chain(done).collect()
}

/// The tasks in `column`, in the order the board shows them: open ones as
/// they were added, done ones newest first.
pub fn in_column(
    tasks: &[Task],
    want: Column,
    state: impl Fn(&str) -> Option<LaunchedState>,
) -> Vec<&Task> {
    let mut out: Vec<&Task> = tasks
        .iter()
        .filter(|t| column(t, t.session_id.as_deref().and_then(&state)) == want)
        .collect();
    if want == Column::Done {
        out.sort_by_key(|t| std::cmp::Reverse(t.done_at));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("Task {id}"),
            prompt: String::new(),
            project: "/src/app".into(),
            session_id: None,
            done_at: None,
            created_at: 1,
            extra: Map::new(),
        }
    }

    #[test]
    fn a_task_sits_where_its_session_is() {
        let mut t = task("a");
        assert_eq!(column(&t, None), Column::Todo);
        t.session_id = Some("s".into());
        assert_eq!(column(&t, Some(LaunchedState::Working)), Column::Running);
        assert_eq!(column(&t, Some(LaunchedState::Input)), Column::Running);
        assert_eq!(column(&t, Some(LaunchedState::Draft)), Column::Todo);
        assert_eq!(column(&t, Some(LaunchedState::Failed)), Column::Review);
        assert_eq!(column(&t, Some(LaunchedState::Done)), Column::Review);
        assert_eq!(column(&t, None), Column::Review);
        t.done_at = Some(5);
        assert_eq!(column(&t, Some(LaunchedState::Working)), Column::Done);
    }

    #[test]
    fn the_file_round_trips_what_it_does_not_model() {
        let text = r#"[{"id":"a","title":"Fix","project":"/p","color":"red"},{"id":"","title":"x","project":"/p"},{"nope":1}]"#;
        let tasks = parse_board(text);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].extra.get("color"), Some(&Value::from("red")));
        assert!(serialize_board(&tasks).contains("\"color\": \"red\""));
        assert!(parse_board("not json").is_empty());
    }

    #[test]
    fn edits() {
        let add = Edit::Add {
            id: "a".into(),
            title: "Fix".into(),
            prompt: "Fix the build".into(),
            project: "/p".into(),
            session_id: None,
        };
        let (tasks, added) = apply(Vec::new(), &add, 10).unwrap();
        assert_eq!(added.unwrap().created_at, 10);
        // A retry adds nothing.
        let (tasks, _) = apply(tasks, &add, 11).unwrap();
        assert_eq!(tasks.len(), 1);

        let link = Edit::Link { id: "a".into(), session_id: "s1".into() };
        let (tasks, _) = apply(tasks, &link, 12).unwrap();
        let reprompt = Edit::Update { id: "a".into(), title: None, prompt: Some("x".into()), done: None };
        assert!(apply(tasks.clone(), &reprompt, 13).is_err());

        let done = Edit::Update { id: "a".into(), title: None, prompt: None, done: Some(true) };
        let (tasks, touched) = apply(tasks, &done, 14).unwrap();
        assert_eq!(touched.unwrap().done_at, Some(14));
        // Marked done again keeps when it was.
        let (tasks, touched) = apply(tasks, &done, 15).unwrap();
        assert_eq!(touched.unwrap().done_at, Some(14));
        let reopen = Edit::Update { id: "a".into(), title: None, prompt: None, done: Some(false) };
        let (tasks, touched) = apply(tasks, &reopen, 16).unwrap();
        assert_eq!(touched.unwrap().done_at, None);

        let (tasks, gone) = apply(tasks, &Edit::Remove { id: "a".into() }, 17).unwrap();
        assert!(tasks.is_empty() && gone.is_none());
        assert!(apply(tasks, &Edit::Remove { id: "a".into() }, 18).is_err());
    }

    #[test]
    fn a_session_goes_on_the_board_once() {
        let add = |id: &str| Edit::Add {
            id: id.into(),
            title: "T".into(),
            prompt: String::new(),
            project: "/p".into(),
            session_id: Some("s1".into()),
        };
        let (tasks, _) = apply(Vec::new(), &add("a"), 1).unwrap();
        assert!(apply(tasks, &add("b"), 2).is_err());
    }

    #[test]
    fn the_board_has_room_for_so_much() {
        let mut tasks: Vec<Task> = (0..OPEN_MAX).map(|i| task(&i.to_string())).collect();
        let add = Edit::Add {
            id: "x".into(),
            title: "T".into(),
            prompt: String::new(),
            project: "/p".into(),
            session_id: None,
        };
        assert!(apply(tasks.clone(), &add, 1).is_err());
        // Done ones do not count, and only the newest are kept.
        for (i, t) in tasks.iter_mut().enumerate() {
            t.done_at = Some(i as i64);
        }
        let (tasks, _) = apply(tasks, &add, 100).unwrap();
        assert_eq!(tasks.len(), 1 + DONE_KEPT);
        assert_eq!(tasks[1].done_at, Some((OPEN_MAX - 1) as i64));
    }

    #[test]
    fn columns_list_their_tasks() {
        let mut a = task("a");
        a.session_id = Some("s1".into());
        let mut b = task("b");
        b.done_at = Some(1);
        let mut c = task("c");
        c.done_at = Some(2);
        let tasks = vec![a, task("d"), b, c];
        let state = |id: &str| (id == "s1").then_some(LaunchedState::Working);
        let ids = |col| in_column(&tasks, col, state).iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(Column::Running), ["a"]);
        assert_eq!(ids(Column::Todo), ["d"]);
        assert_eq!(ids(Column::Done), ["c", "b"]);
        assert!(ids(Column::Review).is_empty());
    }
}
