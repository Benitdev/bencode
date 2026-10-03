//! Claiming scheduled automation runs, ported from MonoCode's
//! `automations_claim_due` / `automation_runs_recover`
//! (`src-tauri/src/automations.rs`). Claims are compare-and-set on
//! `next_run_at`, so one occurrence starts at most one run even if BenCode
//! and MonoCode share the database.

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;

use super::{AutomationRunRow, MonoCodeDb};

/// MonoCode's default `missedRunGraceMinutes`.
pub const DEFAULT_GRACE_MINUTES: i64 = 720;
const INTERRUPTED: &str = "Interrupted when BenCode last stopped.";
const NOT_STARTED: &str = "Not started before BenCode last stopped.";
const MISSED: &str = "Missed the scheduled run beyond its grace period.";

impl MonoCodeDb {
    /// Advances automation `id` from `expected_next` to `next` and records a
    /// `scheduled` run, if the occurrence is still unclaimed and due at
    /// `now`. A run later than `grace_minutes` is recorded as `skipped`.
    pub fn claim_due_automation(
        &self,
        id: &str,
        expected_next: i64,
        next: i64,
        now: i64,
        grace_minutes: i64,
    ) -> Result<Option<AutomationRunRow>> {
        if next <= expected_next {
            anyhow::bail!("the next occurrence must advance the schedule");
        }
        let tx = self.conn.unchecked_transaction()?;
        let claimed = tx.execute(
            "UPDATE automations
             SET next_run_at = ?1,
                 definition_json = CASE WHEN json_valid(definition_json)
                     THEN json_set(definition_json, '$.nextRunAt', ?1, '$.lastRunAt', ?4)
                     ELSE definition_json END
             WHERE id = ?2 AND enabled = 1 AND next_run_at = ?3 AND next_run_at <= ?4",
            params![next, id, expected_next, now],
        )?;
        if claimed == 0 {
            return Ok(None);
        }
        let missed = now.saturating_sub(expected_next) > grace_minutes.saturating_mul(60_000);
        let run = AutomationRunRow {
            id: format!("run-{now}-{id}"),
            automation_id: id.to_string(),
            trigger: "scheduled".to_string(),
            scheduled_for: expected_next,
            created_at: now,
            started_at: None,
            completed_at: missed.then_some(now),
            status: if missed { "skipped" } else { "pending" }.to_string(),
            session_id: None,
            error: missed.then(|| MISSED.to_string()),
        };
        tx.execute(
            "INSERT INTO automation_runs (id, automation_id, created_at, run_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run.id,
                run.automation_id,
                run.created_at,
                serde_json::to_string(&run)?
            ],
        )?;
        tx.commit()?;
        Ok(Some(run))
    }

    /// Marks a pending run as started in `session_id`.
    pub fn start_automation_run(&self, run_id: &str, session_id: &str, now: i64) -> Result<()> {
        self.patch_run(run_id, |fields| {
            fields.insert("status".into(), Value::from("running"));
            fields.insert("startedAt".into(), Value::from(now));
            fields.insert("sessionId".into(), Value::from(session_id));
        })
    }

    /// Closes runs a previous BenCode left `running` or `pending`. Returns
    /// how many were closed.
    pub fn recover_automation_runs(&self, started_before: i64, now: i64) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT id, json_extract(run_json, '$.status') FROM automation_runs
             WHERE created_at <= ?1
               AND json_valid(run_json)
               AND json_extract(run_json, '$.status') IN ('running', 'pending')",
        )?;
        let stale: Vec<(String, String)> = stmt
            .query_map([started_before], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, status) in &stale {
            let (status, error) = if status == "running" {
                ("cancelled", INTERRUPTED)
            } else {
                ("skipped", NOT_STARTED)
            };
            self.patch_run(id, |fields| {
                fields.insert("status".into(), Value::from(status));
                fields.insert("completedAt".into(), Value::from(now));
                fields.insert("error".into(), Value::from(error));
            })?;
        }
        Ok(stale.len())
    }

    /// Edits `run_json` in place so fields BenCode does not model survive.
    fn patch_run(
        &self,
        run_id: &str,
        edit: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> Result<()> {
        let raw: String = self.conn.query_row(
            "SELECT run_json FROM automation_runs WHERE id = ?1",
            [run_id],
            |row| row.get(0),
        )?;
        let mut run: Value = serde_json::from_str(&raw)?;
        let Some(fields) = run.as_object_mut() else {
            anyhow::bail!("automation run {run_id} is not a JSON object");
        };
        edit(fields);
        self.conn.execute(
            "UPDATE automation_runs SET run_json = ?1 WHERE id = ?2",
            params![run.to_string(), run_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::monocode_db;
    use super::*;

    fn insert_automation(db: &MonoCodeDb, id: &str, next: i64) {
        db.conn
            .execute(
                "INSERT INTO automations (id, definition_json, enabled, next_run_at, updated_at)
                 VALUES (?1, '{\"nextRunAt\":0}', 1, ?2, 1)",
                params![id, next],
            )
            .unwrap();
    }

    #[test]
    fn an_occurrence_is_claimed_once() {
        let db = monocode_db();
        insert_automation(&db, "a", 1_000);
        let run = db.claim_due_automation("a", 1_000, 2_000, 1_500, 720);
        assert_eq!(run.unwrap().unwrap().status, "pending");
        let again = db.claim_due_automation("a", 1_000, 2_000, 1_600, 720);
        assert!(again.unwrap().is_none());
        let early = db.claim_due_automation("a", 2_000, 3_000, 1_700, 720);
        assert!(early.unwrap().is_none(), "not due yet");
    }

    #[test]
    fn late_runs_beyond_grace_are_skipped() {
        let db = monocode_db();
        insert_automation(&db, "a", 1);
        let run = db
            .claim_due_automation("a", 1, 100_000_000, 31 * 60_000, 30)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "skipped");
    }

    #[test]
    fn recovery_closes_stale_runs() {
        let db = monocode_db();
        insert_automation(&db, "a", 1_000);
        let run = db
            .claim_due_automation("a", 1_000, 2_000, 1_500, 720)
            .unwrap()
            .unwrap();
        db.start_automation_run(&run.id, "s1", 1_500).unwrap();
        assert_eq!(db.recover_automation_runs(1_500, 9_000).unwrap(), 1);
        let runs = db.list_automation_runs("a").unwrap();
        assert_eq!(runs[0].status, "cancelled");
        assert_eq!(db.recover_automation_runs(1_500, 9_000).unwrap(), 0);
    }
}
