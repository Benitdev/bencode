//! Session reminders, ported from MonoCode's `src-tauri/src/reminders.rs`:
//! one reminder per thread in `session_reminders`, claimed (`fired_at`)
//! before its alert so MonoCode and BenCode never both announce it.

use anyhow::{Result, bail};
use rusqlite::{Transaction, TransactionBehavior, params};

use super::AppDb;

/// MonoCode `ensure_table`.
pub(super) const REMINDERS_SQL: &str = "
    CREATE TABLE IF NOT EXISTS session_reminders (
        session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
        due_at INTEGER NOT NULL CHECK (due_at > 0),
        fired_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS session_reminders_pending
        ON session_reminders (due_at) WHERE fired_at IS NULL;
";

/// JavaScript's largest date, MonoCode's upper bound for `due_at`.
const MAX_DUE_AT: i64 = 8_640_000_000_000_000;

/// MonoCode `Reminder`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub session_id: String,
    pub due_at: i64,
    pub fired_at: Option<i64>,
    pub title: String,
    pub harness: String,
    pub cwd: String,
}

impl AppDb {
    /// MonoCode `reminder_list`: every project's reminders, soonest first.
    pub fn list_reminders(&self) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.session_id, r.due_at, r.fired_at, s.title, s.harness, s.cwd
             FROM session_reminders r JOIN sessions s ON s.id = r.session_id
             ORDER BY r.due_at, r.session_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Reminder {
                session_id: row.get(0)?,
                due_at: row.get(1)?,
                fired_at: row.get(2)?,
                title: row.get(3)?,
                harness: row.get(4)?,
                cwd: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// MonoCode `reminder_set`: all or nothing; rescheduling re-arms a
    /// reminder that already fired.
    pub fn set_reminders(&self, session_ids: &[String], due_at: i64, now: i64) -> Result<()> {
        if due_at <= now || due_at > MAX_DUE_AT {
            bail!("Choose a reminder time in the future.");
        }
        // Take the write lock up front: MonoCode shares the file.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for id in session_ids {
            let saved: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions
                 WHERE id = ?1 AND has_user_message = 1 AND inbox_ask IS NULL)",
                [id],
                |row| row.get(0),
            )?;
            if !saved {
                bail!("This conversation must be saved before adding a reminder.");
            }
            tx.execute(
                "INSERT INTO session_reminders (session_id, due_at) VALUES (?1, ?2)
                 ON CONFLICT(session_id) DO UPDATE SET due_at = excluded.due_at, fired_at = NULL",
                params![id, due_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// MonoCode `reminder_clear`; with `expected_due_at`, only that
    /// occurrence goes (an old alert cannot clear a newer reminder).
    pub fn clear_reminders(&self, session_ids: &[String], expected_due_at: Option<i64>) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for id in session_ids {
            tx.execute(
                "DELETE FROM session_reminders WHERE session_id = ?1 AND (?2 IS NULL OR due_at = ?2)",
                params![id, expected_due_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// MonoCode `take_due`: claims the reminders due at `now` that were not
    /// announced yet, and returns them.
    pub fn take_due_reminders(&self, now: i64) -> Result<Vec<Reminder>> {
        let due: Vec<Reminder> = self
            .list_reminders()?
            .into_iter()
            .filter(|r| r.due_at <= now && r.fired_at.is_none())
            .collect();
        if due.is_empty() {
            return Ok(due);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut claimed = Vec::new();
        for reminder in due {
            let changed = tx.execute(
                "UPDATE session_reminders SET fired_at = ?1
                 WHERE session_id = ?2 AND due_at = ?3 AND fired_at IS NULL",
                params![now, reminder.session_id, reminder.due_at],
            )?;
            if changed == 1 {
                claimed.push(Reminder {
                    fired_at: Some(now),
                    ..reminder
                });
            }
        }
        tx.commit()?;
        Ok(claimed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Block, SessionRow};

    fn db_with(ids: &[&str]) -> AppDb {
        let db = AppDb::open_in_memory().unwrap();
        for id in ids {
            let session = SessionRow {
                id: id.to_string(),
                title: format!("T {id}"),
                cwd: "/p".into(),
                harness: "claude".into(),
                blocks: vec![Block::new("b", "user", "hi")],
                ..Default::default()
            };
            db.upsert_session(&session).unwrap();
        }
        db
    }

    #[test]
    fn reminders_set_reschedule_claim_and_clear() {
        let db = db_with(&["a", "b"]);
        assert!(db.set_reminders(&["a".into()], 100, 200).is_err(), "past");
        db.set_reminders(&["a".into(), "b".into()], 500, 200).unwrap();
        assert_eq!(db.list_reminders().unwrap().len(), 2);
        let claimed = db.take_due_reminders(600).unwrap();
        assert_eq!(claimed.len(), 2);
        assert!(db.take_due_reminders(700).unwrap().is_empty(), "claimed once");
        db.set_reminders(&["a".into()], 900, 700).unwrap();
        assert_eq!(db.list_reminders().unwrap()[1].fired_at, None, "re-armed");
        db.clear_reminders(&["a".into()], Some(500)).unwrap();
        assert_eq!(db.list_reminders().unwrap().len(), 2, "stale clear is ignored");
        db.clear_reminders(&["a".into(), "b".into()], None).unwrap();
        assert!(db.list_reminders().unwrap().is_empty());
    }

    #[test]
    fn unsent_threads_cannot_get_reminders() {
        let db = db_with(&["a"]);
        let blank = SessionRow {
            id: "blank".into(),
            cwd: "/p".into(),
            harness: "claude".into(),
            ..Default::default()
        };
        db.upsert_session(&blank).unwrap();
        let err = db.set_reminders(&["a".into(), "blank".into()], 500, 1).unwrap_err();
        assert!(err.to_string().contains("must be saved"));
        assert!(db.list_reminders().unwrap().is_empty(), "all or nothing");
    }
}
