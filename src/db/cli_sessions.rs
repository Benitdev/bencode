//! BenCode's own: the CLI sessions BenCode has or had, so Settings ›
//! Providers › Import sessions brings in only the ones it never knew.
//! A thread that lets go of its session (deleted, moved to another
//! harness, account or session) leaves the CLI's files behind; triggers
//! note it in `left_cli_sessions`, so every path that does so is covered.

use std::collections::HashSet;

use anyhow::Result;

use super::{AppDb, SessionRow};

pub(super) const LEFT_CLI_SESSIONS_SQL: &str = "
    CREATE TABLE IF NOT EXISTS left_cli_sessions (
        provider_session_id TEXT PRIMARY KEY
    );
    CREATE TRIGGER IF NOT EXISTS left_cli_session_on_delete
    AFTER DELETE ON sessions
    WHEN OLD.provider_session_id IS NOT NULL
    BEGIN
        INSERT OR IGNORE INTO left_cli_sessions VALUES (OLD.provider_session_id);
    END;
    CREATE TRIGGER IF NOT EXISTS left_cli_session_on_update
    AFTER UPDATE OF provider_session_id ON sessions
    WHEN OLD.provider_session_id IS NOT NULL
        AND OLD.provider_session_id IS NOT NEW.provider_session_id
    BEGIN
        INSERT OR IGNORE INTO left_cli_sessions VALUES (OLD.provider_session_id);
    END;
";

impl AppDb {
    /// Every CLI session a thread resumes or once did.
    pub fn known_provider_session_ids(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider_session_id FROM sessions WHERE provider_session_id IS NOT NULL
             UNION SELECT provider_session_id FROM left_cli_sessions",
        )?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(ids.collect::<rusqlite::Result<_>>()?)
    }

    /// Saves the imported threads whose session BenCode never had, in one
    /// transaction; their folders, and how many it already had.
    pub fn import_sessions(&self, rows: Vec<SessionRow>) -> Result<(Vec<String>, usize)> {
        let known = self.known_provider_session_ids()?;
        let (mut added, mut already) = (Vec::new(), 0);
        let tx = self.conn.unchecked_transaction()?;
        for row in rows {
            if row
                .provider_session_id
                .as_ref()
                .is_some_and(|id| known.contains(id))
            {
                already += 1;
                continue;
            }
            self.upsert_session(&row)?;
            added.push(row.cwd);
        }
        tx.commit()?;
        Ok((added, already))
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{AppDb, SessionRow};

    fn row(id: &str, provider: Option<&str>) -> SessionRow {
        SessionRow {
            id: id.into(),
            cwd: "/repo".into(),
            harness: "claude".into(),
            model: "claude:default".into(),
            provider_session_id: provider.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn a_session_a_thread_let_go_of_stays_known() {
        let db = AppDb::open_in_memory().unwrap();
        db.upsert_session(&row("a", Some("s-1"))).unwrap();
        db.upsert_session(&row("b", Some("s-2"))).unwrap();
        // Moved to another session, then deleted.
        db.upsert_session(&row("a", Some("s-3"))).unwrap();
        db.delete_session("b").unwrap();
        db.upsert_session(&row("c", None)).unwrap();
        let mut known: Vec<_> = db
            .known_provider_session_ids()
            .unwrap()
            .into_iter()
            .collect();
        known.sort();
        assert_eq!(known, ["s-1", "s-2", "s-3"]);
        // A deleted thread's session is not imported again.
        let (added, already) = db
            .import_sessions(vec![row("d", Some("s-2")), row("e", Some("s-4"))])
            .unwrap();
        assert_eq!((added, already), (vec!["/repo".to_string()], 1));
    }
}
