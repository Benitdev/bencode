//! MonoCode `taskList.ts` and `apply.ts` `upsertTaskList`: the agent's task
//! list, kept in the transcript as one `tasks` block per turn that each new
//! snapshot replaces.

use serde_json::{Value, json};

use crate::db::{Block, SessionRow};
use crate::harness::{TaskItem, TaskStatus};

const ROLE: &str = "tasks";
const META: &str = "taskList";

/// MonoCode `taskListText`: the block's text, which search and copy read.
pub fn text(items: &[TaskItem]) -> String {
    items
        .iter()
        .map(|item| {
            let mark = match item.status {
                TaskStatus::Completed => "[x]",
                TaskStatus::InProgress => "[~]",
                TaskStatus::Cancelled => "[-]",
                TaskStatus::Pending => "[ ]",
            };
            format!("{mark} {}", item.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// MonoCode `taskListProgressLabel`: "2 of 5", or "Complete".
pub fn progress_label(items: &[TaskItem]) -> String {
    let count = |status| items.iter().filter(|i| i.status == status).count();
    let completed = count(TaskStatus::Completed);
    let actionable = items.len() - count(TaskStatus::Cancelled);
    if actionable > 0 && completed == actionable {
        return "Complete".into();
    }
    let total = if actionable > 0 {
        actionable
    } else {
        items.len()
    };
    format!("{completed} of {total}")
}

/// The list a `tasks` block holds; empty for any other block.
pub fn items(block: &Block) -> Vec<TaskItem> {
    if block.role != ROLE {
        return Vec::new();
    }
    block
        .extra
        .get(META)
        .and_then(|meta| meta.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let text = item.get("text")?.as_str()?.trim();
            (!text.is_empty()).then(|| TaskItem {
                id: item.get("id").and_then(Value::as_str).map(String::from),
                text: text.to_string(),
                status: TaskStatus::parse(
                    item.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
            })
        })
        .collect()
}

/// MonoCode `upsertTaskList` for a whole snapshot: this turn's list is
/// replaced, an empty snapshot takes it away, and an earlier turn's list
/// stays as history.
pub fn upsert(session: &mut SessionRow, items: &[TaskItem], now: i64) {
    let turn_start = session
        .blocks
        .iter()
        .rposition(|block| block.role == "user")
        .map_or(0, |ix| ix + 1);
    let existing = session.blocks[turn_start..]
        .iter()
        .rposition(|block| block.role == ROLE)
        .map(|ix| turn_start + ix);
    if items.is_empty() {
        if let Some(ix) = existing {
            session.blocks.remove(ix);
        }
        return;
    }
    let block = match existing {
        Some(ix) => &mut session.blocks[ix],
        None => {
            let mut block = Block::new(format!("tasks-{now}"), ROLE, "");
            block.started_at = Some(now);
            session.blocks.push(block);
            session.blocks.last_mut().expect("just pushed")
        }
    };
    block.text = Some(text(items));
    // Whatever else the stored list carries (MonoCode's `key`,
    // `explanation`) stays.
    match block.extra.get_mut(META).and_then(Value::as_object_mut) {
        Some(meta) => {
            meta.insert("items".into(), json!(items));
        }
        None => {
            block.extra.insert(META.into(), json!({ "items": items }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, status: TaskStatus) -> TaskItem {
        TaskItem {
            id: None,
            text: text.into(),
            status,
        }
    }

    fn session() -> SessionRow {
        SessionRow {
            blocks: vec![Block::new("u1", "user", "do it")],
            ..Default::default()
        }
    }

    #[test]
    fn a_snapshot_replaces_this_turns_list() {
        let mut s = session();
        upsert(&mut s, &[item("Inspect", TaskStatus::InProgress)], 1);
        upsert(
            &mut s,
            &[
                item("Inspect", TaskStatus::Completed),
                item("Add test", TaskStatus::Pending),
            ],
            2,
        );
        assert_eq!(s.blocks.len(), 2);
        assert_eq!(s.blocks[1].role, "tasks");
        assert_eq!(
            s.blocks[1].text.as_deref(),
            Some("[x] Inspect\n[ ] Add test")
        );
        assert_eq!(
            s.blocks[1].extra["taskList"]["items"][0],
            json!({ "text": "Inspect", "status": "completed" })
        );
        assert_eq!(items(&s.blocks[1]).len(), 2);
    }

    #[test]
    fn an_earlier_turns_list_stays_and_an_empty_snapshot_removes() {
        let mut s = session();
        upsert(&mut s, &[item("Old", TaskStatus::Completed)], 1);
        s.blocks.push(Block::new("u2", "user", "next"));
        upsert(&mut s, &[], 2);
        assert_eq!(s.blocks.len(), 3, "the first turn's list is history");
        upsert(&mut s, &[item("New", TaskStatus::Pending)], 3);
        assert_eq!(s.blocks.len(), 4);
        upsert(&mut s, &[], 4);
        assert_eq!(s.blocks.len(), 3);
        assert_eq!(items(&s.blocks[1])[0].text, "Old");
    }

    #[test]
    fn progress_counts_what_can_still_be_done() {
        let list = [
            item("a", TaskStatus::Completed),
            item("b", TaskStatus::Cancelled),
            item("c", TaskStatus::Pending),
        ];
        assert_eq!(progress_label(&list), "1 of 2");
        assert_eq!(progress_label(&list[..2]), "Complete");
        assert_eq!(progress_label(&list[1..2]), "0 of 1");
    }

    #[test]
    fn statuses_read_every_spelling() {
        assert_eq!(TaskStatus::parse("in-progress"), TaskStatus::InProgress);
        assert_eq!(TaskStatus::parse("Done"), TaskStatus::Completed);
        assert_eq!(TaskStatus::parse("canceled"), TaskStatus::Cancelled);
        assert_eq!(TaskStatus::parse(""), TaskStatus::Pending);
    }
}
