//! Worktree deletion journal, ported from MonoCode's
//! `src-tauri/src/worktrees.rs` (`session_ids`, `prepare_removal`,
//! `finish_removal`, `reconcile_removals`). Threads are detached *before*
//! git deletes the folder, with their old context saved in
//! `worktree_removals`; a failed deletion restores them, and one cut off
//! by a crash is settled the next time either app opens the database.
//! The journal's JSON is MonoCode's, so either app can finish the other's.

use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::AppDb;

/// MonoCode `migrate`: the journal and the in-flight table it restores.
pub(super) const WORKTREE_REMOVALS_SQL: &str = "
    CREATE TABLE IF NOT EXISTS in_flight_sessions (
        session_id TEXT PRIMARY KEY,
        cwd TEXT NOT NULL,
        sort_index INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS worktree_removals (
        path TEXT PRIMARY KEY,
        sessions_json TEXT NOT NULL
    );
";

/// MonoCode `SessionBeforeRemoval`: a thread's context before it was
/// detached, and the `cwd` it was detached to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBeforeRemoval {
    pub id: String,
    pub cwd: String,
    pub worktree_cwd: Option<String>,
    pub branch: Option<String>,
    pub provider_session_id: Option<String>,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub in_flight_cwd: Option<String>,
    pub in_flight_sort_index: Option<i64>,
    pub detached_cwd: String,
}

fn expand_home(path: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if path == "~" {
        return home().unwrap_or_else(|| PathBuf::from(path));
    }
    match (path.strip_prefix("~/"), home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// MonoCode `contains_working_dir`: `cwd` is `root` or inside it.
fn contains_working_dir(root: &str, cwd: &str) -> bool {
    crate::app::is_path_in_project(&expand_home(cwd).to_string_lossy(), root)
}

impl AppDb {
    /// MonoCode `session_ids`: threads still working in the worktree at
    /// `path`, archived ones included.
    pub fn session_ids_in_worktree(&self, path: &str) -> Result<Vec<String>> {
        let mut query = self.conn.prepare(
            "SELECT id, COALESCE(NULLIF(worktree_cwd, ''), cwd) FROM sessions WHERE worktree_removed = 0",
        )?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, cwd) = row?;
            if contains_working_dir(path, &cwd) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// MonoCode `prepare_removal`: detaches `ids` from the worktree at
    /// `path` and journals what they were, before git touches the folder.
    /// Threads not in the database (unsaved drafts) are skipped.
    pub fn prepare_worktree_removal(
        &self,
        path: &str,
        project_cwd: &str,
        ids: &[String],
    ) -> Result<Vec<SessionBeforeRemoval>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut saved = Vec::new();
        for id in ids {
            let session = tx
                .query_row(
                    "SELECT s.cwd, s.worktree_cwd, s.branch, s.provider_session_id,
                        s.context_used, s.context_window, f.cwd, f.sort_index
                     FROM sessions s LEFT JOIN in_flight_sessions f ON f.session_id = s.id
                     WHERE s.id = ?1",
                    [id],
                    |row| {
                        Ok(SessionBeforeRemoval {
                            id: id.clone(),
                            cwd: row.get(0)?,
                            worktree_cwd: row.get(1)?,
                            branch: row.get(2)?,
                            provider_session_id: row.get(3)?,
                            context_used: row.get(4)?,
                            context_window: row.get(5)?,
                            in_flight_cwd: row.get(6)?,
                            in_flight_sort_index: row.get(7)?,
                            detached_cwd: String::new(),
                        })
                    },
                )
                .optional()?;
            let Some(mut session) = session else {
                continue;
            };
            session.detached_cwd = if contains_working_dir(path, &session.cwd) {
                project_cwd.to_owned()
            } else {
                session.cwd.clone()
            };
            tx.execute(
                "UPDATE sessions SET worktree_removed = 1,
                   worktree_cwd = COALESCE(NULLIF(worktree_cwd, ''), cwd),
                   cwd = ?2, branch = NULL, provider_session_id = NULL,
                   context_used = NULL, context_window = NULL WHERE id = ?1",
                params![id, session.detached_cwd],
            )?;
            tx.execute("DELETE FROM in_flight_sessions WHERE session_id = ?1", [id])?;
            saved.push(session);
        }
        tx.execute(
            "INSERT INTO worktree_removals (path, sessions_json) VALUES (?1, ?2)",
            params![path, serde_json::to_string(&saved)?],
        )?;
        tx.commit()?;
        Ok(saved)
    }

    /// MonoCode `finish_removal`: restores `restore` (empty once git
    /// succeeded) and drops the journal entry. A thread that was since
    /// reattached or deleted is left alone.
    pub fn finish_worktree_removal(
        &self,
        path: &str,
        restore: &[SessionBeforeRemoval],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for session in restore {
            let changed = tx.execute(
                "UPDATE sessions SET cwd = ?2, worktree_cwd = ?3, branch = ?4,
                   provider_session_id = ?5, context_used = ?6, context_window = ?7,
                   worktree_removed = 0
                 WHERE id = ?1 AND worktree_removed = 1 AND cwd = ?8 AND worktree_cwd = ?9",
                params![
                    session.id,
                    session.cwd,
                    session.worktree_cwd,
                    session.branch,
                    session.provider_session_id,
                    session.context_used,
                    session.context_window,
                    session.detached_cwd,
                    session
                        .worktree_cwd
                        .as_deref()
                        .filter(|cwd| !cwd.is_empty())
                        .unwrap_or(&session.cwd)
                ],
            )?;
            if changed > 0
                && let (Some(cwd), Some(index)) =
                    (&session.in_flight_cwd, session.in_flight_sort_index)
            {
                tx.execute(
                    "INSERT OR REPLACE INTO in_flight_sessions (session_id, cwd, sort_index) VALUES (?1, ?2, ?3)",
                    params![session.id, cwd, index],
                )?;
            }
        }
        tx.execute("DELETE FROM worktree_removals WHERE path = ?1", [path])?;
        tx.commit()?;
        Ok(())
    }

    /// MonoCode `reconcile_removals`: a worktree whose git link survived
    /// was not deleted, so its threads get their context back; otherwise
    /// the detached threads are final. Unlike MonoCode, an entry that cannot
    /// be settled does not keep the later ones waiting.
    pub fn reconcile_worktree_removals(&self) -> Result<()> {
        let pending = self
            .conn
            .prepare("SELECT path, sessions_json FROM worktree_removals")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut failed = 0;
        for (path, json) in pending {
            if let Err(err) = self.settle_worktree_removal(&path, &json) {
                // Left in the journal for the next launch; the others settle now.
                log::error!("could not settle the deletion of {path}: {err:#}");
                failed += 1;
            }
        }
        if failed > 0 {
            anyhow::bail!("{failed} interrupted worktree deletion(s) left for the next launch");
        }
        Ok(())
    }

    fn settle_worktree_removal(&self, path: &str, json: &str) -> Result<()> {
        let restore = if Path::new(path).join(".git").try_exists()? {
            serde_json::from_str::<Vec<SessionBeforeRemoval>>(json)?
        } else {
            Vec::new()
        };
        self.finish_worktree_removal(path, &restore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SessionRow;

    fn db_with(rows: &[SessionRow]) -> AppDb {
        let db = AppDb::open_in_memory().unwrap();
        for row in rows {
            db.upsert_session(row).unwrap();
        }
        db
    }

    fn row(id: &str, cwd: &str, worktree: Option<&str>) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: "T".into(),
            cwd: cwd.into(),
            harness: "codex".into(),
            model: "m".into(),
            worktree_cwd: worktree.map(Into::into),
            branch: Some("feature".into()),
            provider_session_id: Some("provider".into()),
            context_used: Some(12),
            context_window: Some(100),
            ..Default::default()
        }
    }

    type Context = (String, Option<String>, bool, Option<String>, Option<String>);

    fn context(db: &AppDb, id: &str) -> Context {
        let s = db.get_session(id).unwrap().unwrap();
        (
            s.cwd,
            s.worktree_cwd,
            s.worktree_removed,
            s.branch,
            s.provider_session_id,
        )
    }

    fn journal_count(db: &AppDb) -> i64 {
        db.conn
            .query_row("SELECT COUNT(*) FROM worktree_removals", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn session_ids_cover_linked_and_nested_threads() {
        let db = db_with(&[
            row("linked", "/p", Some("/p-worktrees/a")),
            row("nested", "/p-worktrees/a/src", None),
            row("other", "/p", None),
        ]);
        let mut ids = db.session_ids_in_worktree("/p-worktrees/a").unwrap();
        ids.sort();
        assert_eq!(ids, ["linked", "nested"]);
    }

    #[test]
    fn prepare_detaches_and_failure_restores() {
        let db = db_with(&[
            row("linked", "/p", Some("/p-worktrees/a")),
            row("nested", "/p-worktrees/a/src", None),
        ]);
        db.conn
            .execute(
                "INSERT INTO in_flight_sessions VALUES ('nested', '/p-worktrees/a/src', 3)",
                [],
            )
            .unwrap();
        let ids = [
            "linked".to_string(),
            "nested".to_string(),
            "unsaved".to_string(),
        ];

        let saved = db
            .prepare_worktree_removal("/p-worktrees/a", "/p", &ids)
            .unwrap();

        assert_eq!(saved.len(), 2);
        assert_eq!(
            context(&db, "linked"),
            ("/p".into(), Some("/p-worktrees/a".into()), true, None, None)
        );
        assert_eq!(
            context(&db, "nested"),
            (
                "/p".into(),
                Some("/p-worktrees/a/src".into()),
                true,
                None,
                None
            )
        );
        assert_eq!(journal_count(&db), 1);
        assert!(
            db.session_ids_in_worktree("/p-worktrees/a")
                .unwrap()
                .is_empty()
        );

        db.finish_worktree_removal("/p-worktrees/a", &saved)
            .unwrap();

        assert_eq!(
            context(&db, "linked"),
            (
                "/p".into(),
                Some("/p-worktrees/a".into()),
                false,
                Some("feature".into()),
                Some("provider".into())
            )
        );
        assert_eq!(
            context(&db, "nested"),
            (
                "/p-worktrees/a/src".into(),
                None,
                false,
                Some("feature".into()),
                Some("provider".into())
            )
        );
        let in_flight: (String, i64) = db
            .conn
            .query_row(
                "SELECT cwd, sort_index FROM in_flight_sessions WHERE session_id = 'nested'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(in_flight, ("/p-worktrees/a/src".into(), 3));
        assert_eq!(journal_count(&db), 0);
    }

    #[test]
    fn restore_skips_threads_reattached_since() {
        let db = db_with(&[row("linked", "/p", Some("/p-worktrees/a"))]);
        let saved = db
            .prepare_worktree_removal("/p-worktrees/a", "/p", &["linked".into()])
            .unwrap();
        db.reattach_session("linked", Some("/p-worktrees/b"))
            .unwrap();

        db.finish_worktree_removal("/p-worktrees/a", &saved)
            .unwrap();

        let s = db.get_session("linked").unwrap().unwrap();
        assert_eq!(s.worktree_cwd.as_deref(), Some("/p-worktrees/b"));
        assert_eq!(s.branch, None);
    }

    /// MonoCode `interrupted_removal_restores_or_finalizes_on_open`.
    #[test]
    fn reconcile_restores_only_when_the_worktree_survived() {
        let base = std::env::temp_dir().join(format!(
            "bencode-wt-journal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let alive = base.join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        std::fs::write(alive.join(".git"), "gitdir: /x").unwrap();
        let (alive, gone) = (
            alive.to_string_lossy().into_owned(),
            base.join("gone").to_string_lossy().into_owned(),
        );
        let db = db_with(&[row("a", "/p", Some(&alive)), row("g", "/p", Some(&gone))]);
        db.prepare_worktree_removal(&alive, "/p", &["a".into()])
            .unwrap();
        db.prepare_worktree_removal(&gone, "/p", &["g".into()])
            .unwrap();

        db.reconcile_worktree_removals().unwrap();

        assert!(!context(&db, "a").2);
        assert_eq!(context(&db, "a").3.as_deref(), Some("feature"));
        assert_eq!(
            context(&db, "g"),
            ("/p".into(), Some(gone), true, None, None)
        );
        assert_eq!(journal_count(&db), 0);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_unreadable_entry_does_not_block_the_others() {
        let base = std::env::temp_dir().join(format!(
            "bencode-wt-journal-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let alive = base.join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        std::fs::write(alive.join(".git"), "gitdir: /x").unwrap();
        let (alive, gone) = (
            alive.to_string_lossy().into_owned(),
            base.join("gone").to_string_lossy().into_owned(),
        );
        let db = db_with(&[row("a", "/p", Some(&alive)), row("g", "/p", Some(&gone))]);
        db.prepare_worktree_removal(&alive, "/p", &["a".into()])
            .unwrap();
        db.prepare_worktree_removal(&gone, "/p", &["g".into()])
            .unwrap();
        db.conn
            .execute(
                "UPDATE worktree_removals SET sessions_json = 'not json' WHERE path = ?1",
                [&alive],
            )
            .unwrap();

        assert!(db.reconcile_worktree_removals().is_err());

        assert_eq!(
            context(&db, "g"),
            ("/p".into(), Some(gone), true, None, None)
        );
        assert_eq!(journal_count(&db), 1);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn journal_json_matches_monocode() {
        let saved = SessionBeforeRemoval {
            id: "s1".into(),
            cwd: "/p".into(),
            worktree_cwd: None,
            branch: None,
            provider_session_id: None,
            context_used: None,
            context_window: None,
            in_flight_cwd: None,
            in_flight_sort_index: None,
            detached_cwd: "/p".into(),
        };
        let json = serde_json::to_value(&saved).unwrap();
        for key in [
            "id",
            "cwd",
            "worktree_cwd",
            "branch",
            "provider_session_id",
            "context_used",
            "context_window",
            "in_flight_cwd",
            "in_flight_sort_index",
            "detached_cwd",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }
}
