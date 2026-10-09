//! MonoCode's `in_flight_sessions` (`session_store.rs` `replace_in_flight`,
//! `list_in_flight`): the threads whose turn was running, kept while it
//! runs, so a quit, a restart or a crash that cuts it off can resume it on
//! the next launch. The table is created with the worktree journal.

use anyhow::Result;
use rusqlite::params;

use super::AppDb;

/// One running thread: its id and its project folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlightSession {
    pub session_id: String,
    pub cwd: String,
}

impl AppDb {
    /// Replaces the whole list, in order.
    pub fn replace_in_flight(&self, sessions: &[InFlightSession]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM in_flight_sessions", [])?;
        for (ix, session) in sessions.iter().enumerate() {
            tx.execute(
                "INSERT OR REPLACE INTO in_flight_sessions (session_id, cwd, sort_index) VALUES (?1, ?2, ?3)",
                params![session.session_id, session.cwd, ix as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_in_flight(&self) -> Result<Vec<InFlightSession>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, cwd FROM in_flight_sessions ORDER BY sort_index, session_id")?;
        let rows = stmt.query_map([], |row| {
            Ok(InFlightSession {
                session_id: row.get(0)?,
                cwd: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> InFlightSession {
        InFlightSession {
            session_id: id.into(),
            cwd: "/p".into(),
        }
    }

    #[test]
    fn the_list_is_replaced_whole_and_keeps_its_order() {
        let db = AppDb::open_in_memory().unwrap();
        db.replace_in_flight(&[entry("b"), entry("a")]).unwrap();
        assert_eq!(db.list_in_flight().unwrap(), vec![entry("b"), entry("a")]);
        db.replace_in_flight(&[entry("c")]).unwrap();
        assert_eq!(db.list_in_flight().unwrap(), vec![entry("c")]);
        db.replace_in_flight(&[]).unwrap();
        assert!(db.list_in_flight().unwrap().is_empty());
    }
}
