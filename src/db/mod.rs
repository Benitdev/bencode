//! Direct access to MonoCode's SQLite database.
//!
//! BenCode shares MonoCode's live database, so every write here must preserve
//! data BenCode does not model: unknown block fields, unknown automation
//! definition fields, and session columns BenCode never reads.

mod schedule;

pub use schedule::DEFAULT_GRACE_MINUTES;

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// MonoCode's `DEFAULT_RUNTIME_MODE` (src/features/sessions/model/session.ts).
pub const DEFAULT_SESSION_RUNTIME_MODE: &str = "supervised";

/// Relative path of MonoCode's database under `$HOME`.
const MONOCODE_DB_RELATIVE_PATH: &str =
    "Library/Application Support/com.monocode.desktop/monocode.db";

/// Maximum automation runs returned by `list_automation_runs`.
const AUTOMATION_RUN_HISTORY_LIMIT: i64 = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub source_session_id: Option<String>,
    pub source_cwd: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteUpsert {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub source_session_id: Option<String>,
    pub source_cwd: Option<String>,
}

/// BenCode's view of a MonoCode automation definition (`definition_json`).
///
/// Only the fields BenCode edits are modelled; everything else MonoCode stores
/// is kept in `extra`. `save_automation` patches the stored JSON rather than
/// replacing it, so stale or partial rows cannot destroy MonoCode fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRow {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub harness: String,
    pub model: String,
    pub cwd: String,
    pub schedule_kind: String,
    pub time: String,
    pub minute: i64,
    pub day_of_week: i64,
    pub enabled: bool,
    pub next_run_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Every MonoCode field BenCode does not model (workspaceMode, triggers, ...).
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunRow {
    pub id: String,
    pub automation_id: String,
    pub trigger: String,
    pub scheduled_for: i64,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub harness: String,
    pub model: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub branch: Option<String>,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub pinned: bool,
    pub archived: bool,
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// Provider-side session id (e.g. for `claude --resume`).
    #[serde(default)]
    pub provider_session_id: Option<String>,
    /// MonoCode runtime mode. `None` keeps the stored value on update and
    /// uses `DEFAULT_SESSION_RUNTIME_MODE` on insert.
    #[serde(default)]
    pub runtime_mode: Option<String>,
    /// Git worktree the thread runs in; `cwd` stays the project folder.
    #[serde(default)]
    pub worktree_cwd: Option<String>,
    /// Set when `blocks_json` could not be parsed. Such rows carry an empty
    /// `blocks` vector and `upsert_session` refuses to write them back.
    #[serde(skip)]
    pub blocks_parse_failed: bool,
}

impl SessionRow {
    /// Directory the thread's agent runs in: its worktree, else `cwd`.
    pub fn work_dir(&self) -> &str {
        self.worktree_cwd
            .as_deref()
            .filter(|path| !path.is_empty())
            .unwrap_or(&self.cwd)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One transcript block, mirroring MonoCode's `Block` type
/// (src/features/sessions/model/session.ts).
///
/// Unknown MonoCode fields (attachments, approval, agentRun, draft, ...) are
/// preserved in `extra`. `text` is required by MonoCode and always serializes
/// as a string (`""` when `None`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub role: String,
    #[serde(default, serialize_with = "serialize_text")]
    pub text: Option<String>,
    #[serde(rename = "turnModel", default, skip_serializing_if = "Option::is_none")]
    pub turn_model: Option<TurnModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<serde_json::Value>,
    #[serde(
        rename = "secondOpinion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub second_opinion: Option<serde_json::Value>,
    #[serde(rename = "startedAt", default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(
        rename = "durationMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub duration_ms: Option<i64>,
    /// Every MonoCode block field BenCode does not model.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Block {
    /// Convenience constructor for a plain text block.
    pub fn new(id: impl Into<String>, role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: role.into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }
}

fn serialize_text<S: Serializer>(text: &Option<String>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(text.as_deref().unwrap_or(""))
}

/// Full MonoCode schema for the tables BenCode touches. `IF NOT EXISTS` makes
/// this a no-op on MonoCode's real database.
const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        cwd TEXT NOT NULL,
        harness TEXT NOT NULL,
        model TEXT NOT NULL,
        model_settings TEXT NOT NULL DEFAULT '{}',
        runtime_mode TEXT NOT NULL,
        title TEXT NOT NULL,
        provider_session_id TEXT,
        blocks_json TEXT NOT NULL DEFAULT '[]',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        branch TEXT,
        context_used INTEGER,
        context_window INTEGER,
        archived INTEGER NOT NULL DEFAULT 0,
        worktree_cwd TEXT,
        has_user_message INTEGER NOT NULL DEFAULT 0,
        pinned INTEGER NOT NULL DEFAULT 0,
        linked_work_item_json TEXT,
        provider_account_id TEXT,
        worktree_removed INTEGER NOT NULL DEFAULT 0,
        is_draft INTEGER NOT NULL DEFAULT 0,
        automation_id TEXT,
        inbox_ask TEXT
    );
    CREATE TABLE IF NOT EXISTS notes (
        id TEXT PRIMARY KEY,
        slug TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        body TEXT NOT NULL DEFAULT '',
        tags_json TEXT NOT NULL DEFAULT '[]',
        source_session_id TEXT,
        source_cwd TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS notes_updated_idx ON notes (updated_at DESC, id);
    CREATE TABLE IF NOT EXISTS automations (
        id TEXT PRIMARY KEY,
        definition_json TEXT NOT NULL,
        enabled INTEGER NOT NULL,
        next_run_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS automations_due_idx ON automations (enabled, next_run_at);
    CREATE TABLE IF NOT EXISTS automation_runs (
        id TEXT PRIMARY KEY,
        automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        run_json TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS automation_runs_history_idx ON automation_runs (automation_id, created_at DESC);
";

/// Session columns added by later MonoCode migrations. Older BenCode fallback
/// databases may lack them; on MonoCode's real database these all exist and
/// `ensure_session_columns` does nothing.
const LATE_SESSION_COLUMNS: &[(&str, &str)] = &[
    ("worktree_cwd", "TEXT"),
    ("has_user_message", "INTEGER NOT NULL DEFAULT 0"),
    ("linked_work_item_json", "TEXT"),
    ("provider_account_id", "TEXT"),
    ("worktree_removed", "INTEGER NOT NULL DEFAULT 0"),
    ("is_draft", "INTEGER NOT NULL DEFAULT 0"),
    ("automation_id", "TEXT"),
    ("inbox_ask", "TEXT"),
];

const SESSION_SELECT: &str =
    "SELECT id, title, cwd, harness, model, created_at, updated_at, branch,
        blocks_json, context_used, context_window, pinned, archived, provider_session_id,
        runtime_mode, worktree_cwd
     FROM sessions";

pub struct MonoCodeDb {
    conn: Connection,
}

impl MonoCodeDb {
    /// Opens MonoCode's database, or `~/.bencode/bencode.db` if MonoCode is
    /// not installed. See `open_fallback` for a non-failing alternative.
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")?;
        let monocode_db_path = PathBuf::from(&home).join(MONOCODE_DB_RELATIVE_PATH);
        if monocode_db_path.exists() {
            return Self::open_at(&monocode_db_path);
        }
        Self::open_at(&bencode_db_path(&home)?)
    }

    /// Never fails: tries `~/.bencode/bencode.db`, then an in-memory database.
    /// Use this when `open_default` returns an error.
    pub fn open_fallback() -> Self {
        let file_db = std::env::var("HOME")
            .map_err(anyhow::Error::from)
            .and_then(|home| bencode_db_path(&home))
            .and_then(|path| Self::open_at(&path));
        match file_db {
            Ok(db) => db,
            Err(err) => {
                log::warn!("Falling back to in-memory database: {err:#}");
                Self::open_in_memory().unwrap_or_else(|err| {
                    // SQLite in-memory databases only fail on allocation failure.
                    panic!("Could not open in-memory SQLite database: {err:#}")
                })
            }
        }
    }

    /// Opens a fresh in-memory database with MonoCode's schema.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Opens the database at `path`, creating MonoCode's schema if missing.
    pub fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        ensure_session_columns(&conn)?;
        Ok(Self { conn })
    }

    pub fn list_recent_sessions(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let sql =
            format!("{SESSION_SELECT} WHERE inbox_ask IS NULL ORDER BY updated_at DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([limit as i64], session_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_sessions_for_cwd(&self, cwd: &str, limit: usize) -> Result<Vec<SessionRow>> {
        let sql = format!(
            "{SESSION_SELECT} WHERE (cwd = ?1 OR substr(cwd, 1, length(?2)) = ?2) \
             AND inbox_ask IS NULL ORDER BY updated_at DESC LIMIT ?3"
        );
        // A prefix comparison, not LIKE: `_` and `%` are common in folder names.
        let prefix = format!("{}/", cwd.trim_end_matches('/'));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![cwd, prefix, limit as i64],
            session_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    #[cfg(test)]
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let sql = format!("{SESSION_SELECT} WHERE id = ?1 AND inbox_ask IS NULL");
        Ok(self
            .conn
            .query_row(&sql, [session_id], session_from_row)
            .optional()?)
    }

    /// Inserts or updates a session. Columns BenCode does not model are left
    /// untouched on update and get MonoCode's defaults on insert.
    pub fn upsert_session(&self, session: &SessionRow) -> Result<()> {
        if session.blocks_parse_failed {
            bail!(
                "refusing to save session {}: its stored transcript could not be parsed",
                session.id
            );
        }
        let blocks_value = serde_json::to_value(&session.blocks)?;
        let blocks_json = serde_json::to_string(&blocks_value)?;
        let has_user_message = has_user_block(&blocks_value);
        let is_draft = has_draft_block(&blocks_value);

        self.conn.execute(
            "INSERT INTO sessions (
                id, cwd, harness, model, title, blocks_json, created_at, updated_at,
                branch, context_used, context_window, pinned, archived,
                provider_session_id, runtime_mode, has_user_message, is_draft, worktree_cwd
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                       COALESCE(?15, ?16), ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                cwd = excluded.cwd,
                harness = excluded.harness,
                model = excluded.model,
                blocks_json = excluded.blocks_json,
                updated_at = excluded.updated_at,
                branch = excluded.branch,
                context_used = excluded.context_used,
                context_window = excluded.context_window,
                pinned = excluded.pinned,
                archived = excluded.archived,
                provider_session_id = excluded.provider_session_id,
                runtime_mode = COALESCE(?15, sessions.runtime_mode),
                has_user_message = excluded.has_user_message,
                is_draft = excluded.is_draft,
                worktree_cwd = excluded.worktree_cwd",
            params![
                session.id,
                session.cwd,
                session.harness,
                session.model,
                session.title,
                blocks_json,
                session.created_at,
                session.updated_at,
                session.branch,
                session.context_used,
                session.context_window,
                i64::from(session.pinned),
                i64::from(session.archived),
                session.provider_session_id,
                session.runtime_mode,
                DEFAULT_SESSION_RUNTIME_MODE,
                i64::from(has_user_message),
                i64::from(is_draft),
                session.worktree_cwd,
            ],
        )?;
        Ok(())
    }

    pub fn toggle_pinned(&self, session_id: &str, current: bool) -> Result<()> {
        let new_val = if current { 0 } else { 1 };
        self.conn.execute(
            "UPDATE sessions SET pinned = ?1 WHERE id = ?2",
            params![new_val, session_id],
        )?;
        Ok(())
    }

    pub fn toggle_archived(&self, session_id: &str, current: bool) -> Result<()> {
        let new_val = if current { 0 } else { 1 };
        self.conn.execute(
            "UPDATE sessions SET archived = ?1 WHERE id = ?2",
            params![new_val, session_id],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn list_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, slug, title, body, tags_json, source_session_id, source_cwd, created_at, updated_at
             FROM notes
             ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let tags_json: String = row.get(4)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(Note {
                id: row.get(0)?,
                slug: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
                tags,
                source_session_id: row.get(5)?,
                source_cwd: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn get_note(&self, id: &str) -> Result<Option<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, slug, title, body, tags_json, source_session_id, source_cwd, created_at, updated_at
             FROM notes
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let tags_json: String = row.get(4)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(Note {
                id: row.get(0)?,
                slug: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
                tags,
                source_session_id: row.get(5)?,
                source_cwd: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        if let Some(r) = rows.next() {
            Ok(Some(r?))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_note(&self, note: &NoteUpsert) -> Result<Note> {
        let now = now_millis();
        let tags_json = serde_json::to_string(&note.tags)?;
        let existing = self.get_note(&note.id)?;
        if let Some(existing) = existing {
            let updated_at = if note.title == existing.title
                && note.body == existing.body
                && note.tags == existing.tags
            {
                existing.updated_at
            } else {
                now
            };
            let project_cwd = note
                .source_cwd
                .as_deref()
                .or(existing.source_cwd.as_deref());
            self.conn.execute(
                "UPDATE notes
                 SET title = ?1, body = ?2, tags_json = ?3, updated_at = ?4, source_cwd = ?5
                 WHERE id = ?6",
                params![
                    note.title,
                    note.body,
                    tags_json,
                    updated_at,
                    project_cwd,
                    note.id
                ],
            )?;
            Ok(Note {
                id: note.id.clone(),
                slug: existing.slug,
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                source_session_id: existing.source_session_id,
                source_cwd: project_cwd.map(str::to_string),
                created_at: existing.created_at,
                updated_at,
            })
        } else {
            let slug = unique_slug(&self.conn, &note.title)?;
            self.conn.execute(
                "INSERT INTO notes (
                     id, slug, title, body, tags_json, source_session_id, source_cwd, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    note.id,
                    slug,
                    note.title,
                    note.body,
                    tags_json,
                    note.source_session_id,
                    note.source_cwd,
                    now,
                    now,
                ],
            )?;
            Ok(Note {
                id: note.id.clone(),
                slug,
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                source_session_id: note.source_session_id.clone(),
                source_cwd: note.source_cwd.clone(),
                created_at: now,
                updated_at: now,
            })
        }
    }

    pub fn delete_note(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_automations(&self) -> Result<Vec<AutomationRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, definition_json FROM automations ORDER BY updated_at DESC, id")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut automations = Vec::new();
        for row in rows {
            let (id, raw) = row?;
            match serde_json::from_str::<AutomationRow>(&raw) {
                Ok(auto) => automations.push(auto),
                Err(err) => {
                    log::warn!("Skipping automation {id}: unreadable definition_json: {err}")
                }
            }
        }
        Ok(automations)
    }

    /// Saves an automation by patching the stored `definition_json` with the
    /// fields BenCode models. Unknown MonoCode fields are preserved. New
    /// automations get MonoCode's defaults for every required field.
    pub fn save_automation(&self, auto: &AutomationRow) -> Result<()> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT definition_json FROM automations WHERE id = ?1",
                [&auto.id],
                |row| row.get(0),
            )
            .optional()?;
        let definition = match existing {
            Some(raw) => patch_existing_definition(&raw, auto)?,
            None => new_automation_definition(auto)?,
        };
        let json = serde_json::to_string(&Value::Object(definition))?;
        self.conn.execute(
            "INSERT INTO automations (id, definition_json, enabled, next_run_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 definition_json = excluded.definition_json,
                 enabled = excluded.enabled,
                 next_run_at = excluded.next_run_at,
                 updated_at = excluded.updated_at",
            params![
                auto.id,
                json,
                i64::from(auto.enabled),
                auto.next_run_at,
                auto.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_automation(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM automations WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Updates the `enabled` column and the `enabled` field inside
    /// `definition_json` (MonoCode reads the latter).
    pub fn toggle_automation(&self, id: &str, enabled: bool) -> Result<()> {
        let enabled_json = if enabled { "true" } else { "false" };
        self.conn.execute(
            "UPDATE automations
             SET enabled = ?1,
                 updated_at = ?2,
                 definition_json = CASE WHEN json_valid(definition_json)
                     THEN json_set(definition_json, '$.enabled', json(?3), '$.updatedAt', ?2)
                     ELSE definition_json END
             WHERE id = ?4",
            params![i64::from(enabled), now_millis(), enabled_json, id],
        )?;
        Ok(())
    }

    pub fn list_automation_runs(&self, automation_id: &str) -> Result<Vec<AutomationRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_json FROM automation_runs WHERE automation_id = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![automation_id, AUTOMATION_RUN_HISTORY_LIMIT],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;

        let mut runs = Vec::new();
        for row in rows {
            let (id, raw) = row?;
            match serde_json::from_str::<AutomationRunRow>(&raw) {
                Ok(run) => runs.push(run),
                Err(err) => log::warn!("Skipping automation run {id}: unreadable run_json: {err}"),
            }
        }
        Ok(runs)
    }

    pub fn create_automation_run(&self, run: &AutomationRunRow) -> Result<()> {
        let json = serde_json::to_string(run)?;
        self.conn.execute(
            "INSERT INTO automation_runs (id, automation_id, created_at, run_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![run.id, run.automation_id, run.created_at, json],
        )?;
        Ok(())
    }

    /// Marks a run terminal (`succeeded`, `failed` or `cancelled`), patching
    /// `run_json` in place so fields BenCode does not model survive.
    pub fn finish_automation_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let json: String = self.conn.query_row(
            "SELECT run_json FROM automation_runs WHERE id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        let mut run: Value = serde_json::from_str(&json)
            .map_err(|err| anyhow!("automation run {run_id} has invalid run_json: {err}"))?;
        let Some(fields) = run.as_object_mut() else {
            bail!("automation run {run_id} is not a JSON object")
        };
        fields.insert("status".into(), Value::from(status));
        fields.insert("completedAt".into(), Value::from(now_millis()));
        match error {
            Some(message) => fields.insert("error".into(), Value::from(message)),
            None => fields.remove("error"),
        };
        self.conn.execute(
            "UPDATE automation_runs SET run_json = ?1 WHERE id = ?2",
            params![run.to_string(), run_id],
        )?;
        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in title.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let slug = out.trim_end_matches('-').to_string();
    if slug.is_empty() { "note".into() } else { slug }
}

fn unique_slug(conn: &Connection, title: &str) -> Result<String> {
    let base = slugify(title);
    let mut candidate = base.clone();
    let mut counter = 2;
    loop {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE slug = ?1",
            params![candidate],
            |row| row.get(0),
        )?;
        if count == 0 {
            return Ok(candidate);
        }
        candidate = format!("{}-{}", base, counter);
        counter += 1;
    }
}

fn bencode_db_path(home: &str) -> Result<PathBuf> {
    let bencode_dir = PathBuf::from(home).join(".bencode");
    std::fs::create_dir_all(&bencode_dir)?;
    Ok(bencode_dir.join("bencode.db"))
}

fn ensure_session_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(sessions)")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, decl) in LATE_SESSION_COLUMNS {
        if !existing.iter().any(|col| col == name) {
            conn.execute_batch(&format!("ALTER TABLE sessions ADD COLUMN {name} {decl};"))?;
        }
    }
    Ok(())
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let id: String = row.get(0)?;
    let blocks_json: String = row.get(8)?;
    let (blocks, blocks_parse_failed) = match serde_json::from_str::<Vec<Block>>(&blocks_json) {
        Ok(blocks) => (blocks, false),
        Err(err) => {
            log::warn!("Session {id}: could not parse blocks_json ({err}); row is read-only");
            (Vec::new(), true)
        }
    };
    Ok(SessionRow {
        id,
        title: row.get(1)?,
        cwd: row.get(2)?,
        harness: row.get(3)?,
        model: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        branch: row.get(7)?,
        context_used: row.get(9)?,
        context_window: row.get(10)?,
        pinned: row.get::<_, i64>(11)? != 0,
        archived: row.get::<_, i64>(12)? != 0,
        blocks,
        provider_session_id: row.get(13)?,
        runtime_mode: row.get(14)?,
        worktree_cwd: row.get(15)?,
        blocks_parse_failed,
    })
}

/// Mirrors MonoCode's `has_user_block` in session_store.rs.
fn has_user_block(blocks: &Value) -> bool {
    blocks.as_array().is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| block.get("role").and_then(Value::as_str) == Some("user"))
    })
}

/// Mirrors MonoCode's `has_draft_block` in session_store.rs.
fn has_draft_block(blocks: &Value) -> bool {
    blocks.as_array().is_some_and(|blocks| {
        blocks.iter().any(|block| {
            block.get("role").and_then(Value::as_str) == Some("user")
                && block.get("draft").and_then(Value::as_bool) == Some(true)
        })
    })
}

/// Required MonoCode `Automation` fields BenCode does not model, with the
/// defaults of MonoCode's `newAutomationDraft` (features/automations/model).
fn monocode_automation_defaults() -> Map<String, Value> {
    let defaults = serde_json::json!({
        "modelSettings": {},
        "workspaceMode": "worktree",
        "worktreeCwd": "",
        "sessionFolderId": "",
        "reuseSession": false,
        "runtimeMode": "auto",
        "triggerKind": "time",
        "triggerEvent": "",
        "missedRunGraceMinutes": 720
    });
    match defaults {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The fields BenCode models, as camelCase JSON (without `extra`).
fn known_automation_fields(auto: &AutomationRow) -> Result<Map<String, Value>> {
    let known = AutomationRow {
        extra: Map::new(),
        ..auto.clone()
    };
    match serde_json::to_value(known)? {
        Value::Object(map) => Ok(map),
        _ => Err(anyhow!(
            "automation {} did not serialize to an object",
            auto.id
        )),
    }
}

fn new_automation_definition(auto: &AutomationRow) -> Result<Map<String, Value>> {
    let mut definition = monocode_automation_defaults();
    definition.extend(auto.extra.clone());
    definition.extend(known_automation_fields(auto)?);
    Ok(definition)
}

fn patch_existing_definition(raw: &str, auto: &AutomationRow) -> Result<Map<String, Value>> {
    let mut definition = match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => map,
        Ok(_) => bail!(
            "refusing to overwrite automation {}: definition_json is not an object",
            auto.id
        ),
        Err(err) => bail!(
            "refusing to overwrite automation {}: unreadable definition_json: {err}",
            auto.id
        ),
    };
    // Extra fields only fill gaps: the stored values are authoritative.
    for (key, value) in &auto.extra {
        definition
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    let mut known = known_automation_fields(auto)?;
    // createdAt never changes once stored.
    if definition.contains_key("createdAt") {
        known.remove("createdAt");
    }
    definition.extend(known);
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Copied from `sqlite3 -readonly monocode.db .schema` (MonoCode's real schema).
    const MONOCODE_SCHEMA: &str = "
        CREATE TABLE sessions (
          id TEXT PRIMARY KEY,
          cwd TEXT NOT NULL,
          harness TEXT NOT NULL,
          model TEXT NOT NULL,
          model_settings TEXT NOT NULL DEFAULT '{}',
          runtime_mode TEXT NOT NULL,
          title TEXT NOT NULL,
          provider_session_id TEXT,
          blocks_json TEXT NOT NULL DEFAULT '[]',
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        , branch TEXT, context_used INTEGER, context_window INTEGER, archived INTEGER NOT NULL DEFAULT 0, worktree_cwd TEXT, has_user_message INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, linked_work_item_json TEXT, provider_account_id TEXT, worktree_removed INTEGER NOT NULL DEFAULT 0, is_draft INTEGER NOT NULL DEFAULT 0, automation_id TEXT, inbox_ask TEXT);
        CREATE INDEX sessions_legacy_inbox ON sessions (id) WHERE inbox_ask IS NOT NULL;
        CREATE TABLE automations (
           id TEXT PRIMARY KEY,
           definition_json TEXT NOT NULL,
           enabled INTEGER NOT NULL,
           next_run_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
        CREATE INDEX automations_due_idx ON automations (enabled, next_run_at);
        CREATE TABLE automation_runs (
           id TEXT PRIMARY KEY,
           automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
           created_at INTEGER NOT NULL,
           run_json TEXT NOT NULL
         );
        CREATE INDEX automation_runs_history_idx ON automation_runs (automation_id, created_at DESC);
        CREATE TABLE notes (
           id TEXT PRIMARY KEY,
           slug TEXT NOT NULL UNIQUE,
           title TEXT NOT NULL,
           body TEXT NOT NULL DEFAULT '',
           tags_json TEXT NOT NULL DEFAULT '[]',
           source_session_id TEXT,
           source_cwd TEXT,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
    ";

    pub(super) fn monocode_db() -> MonoCodeDb {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MONOCODE_SCHEMA).unwrap();
        MonoCodeDb::from_connection(conn).unwrap()
    }

    fn insert_monocode_session(db: &MonoCodeDb, id: &str, blocks_json: &str) {
        db.conn
            .execute(
                "INSERT INTO sessions (id, cwd, harness, model, model_settings, runtime_mode, title,
                    provider_session_id, blocks_json, created_at, updated_at, worktree_cwd,
                    linked_work_item_json, provider_account_id, automation_id)
                 VALUES (?1, '/repo', 'claude', 'opus', '{\"effort\":\"high\"}', 'auto', 'T',
                    'prov-1', ?2, 1, 2, '/wt', '{\"k\":1}', 'acct', 'auto-1')",
                params![id, blocks_json],
            )
            .unwrap();
    }

    fn session(id: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: "New".into(),
            cwd: "/repo".into(),
            harness: "claude".into(),
            model: "opus".into(),
            created_at: 10,
            updated_at: 20,
            blocks: vec![Block::new("b1", "user", "hi")],
            ..Default::default()
        }
    }

    // ---- 1. Block round-trip ----

    #[test]
    fn block_preserves_unknown_fields_on_round_trip() {
        let raw = json!({
            "id": "b1", "role": "user", "text": "hello",
            "attachments": [{"name": "a.png"}], "draft": true,
            "approval": {"requestId": 3}, "turnModel": {"harness": "claude", "id": "opus", "name": "Opus"}
        });
        let block: Block = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&block).unwrap(), raw);
    }

    #[test]
    fn block_serializes_missing_text_as_empty_string_and_skips_none_fields() {
        let block = Block {
            id: "b".into(),
            role: "tool".into(),
            ..Default::default()
        };
        let value = serde_json::to_value(&block).unwrap();
        assert_eq!(value, json!({"id": "b", "role": "tool", "text": ""}));
    }

    #[test]
    fn session_blocks_survive_load_and_save() {
        let db = monocode_db();
        let blocks =
            r#"[{"id":"b1","role":"user","text":"x","appRequestId":"r1","btwThreads":[]}]"#;
        insert_monocode_session(&db, "s1", blocks);
        let loaded = db.get_session("s1").unwrap().unwrap();
        db.upsert_session(&loaded).unwrap();
        let stored: String = db
            .conn
            .query_row("SELECT blocks_json FROM sessions WHERE id='s1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let stored: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored, serde_json::from_str::<Value>(blocks).unwrap());
    }

    // ---- 2. Automations ----

    fn monocode_automation_json() -> Value {
        json!({
            "id": "a1", "name": "Old", "prompt": "p", "harness": "claude", "model": "opus",
            "modelSettings": {"effort": "high"}, "cwd": "/repo", "workspaceMode": "existing",
            "worktreeCwd": "/wt", "sessionFolderId": "f1", "reuseSession": true,
            "runtimeMode": "supervised", "triggerKind": "time", "triggerEvent": "",
            "scheduleKind": "weekdays", "minute": 0, "time": "09:00", "dayOfWeek": 1,
            "triggers": [{"id": "t1", "kind": "time"}], "missedRunGraceMinutes": 60,
            "enabled": true, "nextRunAt": 100, "lastSessionId": "s9",
            "createdAt": 1, "updatedAt": 2
        })
    }

    #[test]
    fn save_automation_patches_existing_definition_without_losing_fields() {
        let db = monocode_db();
        let original = monocode_automation_json();
        db.conn
            .execute(
                "INSERT INTO automations VALUES ('a1', ?1, 1, 100, 2)",
                [original.to_string()],
            )
            .unwrap();
        // A partial row, as built by the UI without `extra`.
        let edit = AutomationRow {
            id: "a1".into(),
            name: "Renamed".into(),
            prompt: "new prompt".into(),
            harness: "claude".into(),
            model: "opus".into(),
            cwd: "/repo".into(),
            schedule_kind: "weekdays".into(),
            time: "10:00".into(),
            day_of_week: 1,
            enabled: true,
            next_run_at: 100,
            created_at: 999,
            updated_at: 50,
            ..Default::default()
        };
        db.save_automation(&edit).unwrap();

        let raw: String = db
            .conn
            .query_row(
                "SELECT definition_json FROM automations WHERE id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        let mut expected = original;
        expected["name"] = json!("Renamed");
        expected["prompt"] = json!("new prompt");
        expected["time"] = json!("10:00");
        expected["updatedAt"] = json!(50);
        assert_eq!(saved, expected);

        let listed = db.list_automations().unwrap();
        assert_eq!(listed[0].extra["workspaceMode"], json!("existing"));
    }

    #[test]
    fn new_automation_definition_contains_monocode_required_fields() {
        let db = monocode_db();
        let auto = AutomationRow {
            id: "new".into(),
            name: "N".into(),
            schedule_kind: "daily".into(),
            time: "09:00".into(),
            enabled: true,
            ..Default::default()
        };
        db.save_automation(&auto).unwrap();
        let raw: String = db
            .conn
            .query_row(
                "SELECT definition_json FROM automations WHERE id='new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        for key in [
            "id",
            "name",
            "prompt",
            "harness",
            "model",
            "cwd",
            "workspaceMode",
            "reuseSession",
            "runtimeMode",
            "scheduleKind",
            "minute",
            "time",
            "dayOfWeek",
            "missedRunGraceMinutes",
            "enabled",
            "nextRunAt",
            "createdAt",
            "updatedAt",
        ] {
            assert!(
                saved.get(key).is_some(),
                "missing required MonoCode field {key}"
            );
        }
        assert_eq!(saved["workspaceMode"], json!("worktree"));
        assert!(saved.get("lastRunAt").is_none());
    }

    #[test]
    fn save_automation_refuses_to_overwrite_unparseable_definition() {
        let db = monocode_db();
        db.conn
            .execute(
                "INSERT INTO automations VALUES ('a1', 'not json', 1, 0, 0)",
                [],
            )
            .unwrap();
        let auto = AutomationRow {
            id: "a1".into(),
            ..Default::default()
        };
        assert!(db.save_automation(&auto).is_err());
        assert!(db.list_automations().unwrap().is_empty());
    }

    #[test]
    fn toggle_automation_updates_definition_json() {
        let db = monocode_db();
        db.conn
            .execute(
                "INSERT INTO automations VALUES ('a1', ?1, 1, 100, 2)",
                [monocode_automation_json().to_string()],
            )
            .unwrap();
        db.toggle_automation("a1", false).unwrap();
        let auto = &db.list_automations().unwrap()[0];
        assert!(!auto.enabled);
        assert_eq!(auto.extra["lastSessionId"], json!("s9"));
    }

    #[test]
    fn list_automation_runs_skips_bad_rows() {
        let db = monocode_db();
        db.conn
            .execute(
                "INSERT INTO automations VALUES ('a1', ?1, 1, 100, 2)",
                [monocode_automation_json().to_string()],
            )
            .unwrap();
        let run = AutomationRunRow {
            id: "r1".into(),
            automation_id: "a1".into(),
            trigger: "manual".into(),
            scheduled_for: 1,
            created_at: 1,
            started_at: None,
            completed_at: None,
            status: "queued".into(),
            session_id: None,
            error: None,
        };
        db.create_automation_run(&run).unwrap();
        db.conn
            .execute(
                "INSERT INTO automation_runs VALUES ('r2', 'a1', 2, '{bad')",
                [],
            )
            .unwrap();
        let runs = db.list_automation_runs("a1").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "r1");
    }

    // ---- 3. Sessions: NOT NULL columns and no clobbering ----

    #[test]
    fn upsert_new_session_fills_monocode_defaults() {
        let db = monocode_db();
        db.upsert_session(&session("s1")).unwrap();
        let (runtime_mode, has_user, settings): (String, i64, String) = db
            .conn
            .query_row(
                "SELECT runtime_mode, has_user_message, model_settings FROM sessions WHERE id='s1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(runtime_mode, DEFAULT_SESSION_RUNTIME_MODE);
        assert_eq!(has_user, 1);
        assert_eq!(settings, "{}");
    }

    #[test]
    fn upsert_existing_session_keeps_unmodelled_columns() {
        let db = monocode_db();
        insert_monocode_session(&db, "s1", "[]");
        let mut loaded = db.get_session("s1").unwrap().unwrap();
        assert_eq!(loaded.provider_session_id.as_deref(), Some("prov-1"));
        assert_eq!(loaded.runtime_mode.as_deref(), Some("auto"));
        loaded.runtime_mode = None;
        loaded.title = "Renamed".into();
        db.upsert_session(&loaded).unwrap();

        let row: (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        ) = db
            .conn
            .query_row(
                "SELECT title, runtime_mode, model_settings, provider_session_id, worktree_cwd,
                        linked_work_item_json, provider_account_id, automation_id
                 FROM sessions WHERE id='s1'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "Renamed".into(),
                "auto".into(),
                "{\"effort\":\"high\"}".into(),
                "prov-1".into(),
                "/wt".into(),
                "{\"k\":1}".into(),
                "acct".into(),
                "auto-1".into()
            )
        );
    }

    #[test]
    fn upsert_writes_provider_session_id() {
        let db = monocode_db();
        let mut row = session("s1");
        row.provider_session_id = Some("claude-abc".into());
        db.upsert_session(&row).unwrap();
        let loaded = db.get_session("s1").unwrap().unwrap();
        assert_eq!(loaded.provider_session_id.as_deref(), Some("claude-abc"));
    }

    // ---- 4. Parse failures must not wipe history ----

    #[test]
    fn corrupt_blocks_are_flagged_and_never_written_back() {
        let db = monocode_db();
        insert_monocode_session(&db, "s1", "{corrupt");
        let loaded = db.get_session("s1").unwrap().unwrap();
        assert!(loaded.blocks_parse_failed);
        assert!(loaded.blocks.is_empty());
        assert!(db.upsert_session(&loaded).is_err());
        let stored: String = db
            .conn
            .query_row("SELECT blocks_json FROM sessions WHERE id='s1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "{corrupt");
    }

    // ---- 5. inbox_ask filtering ----

    #[test]
    fn session_queries_skip_inbox_ask_rows() {
        let db = monocode_db();
        insert_monocode_session(&db, "normal", "[]");
        insert_monocode_session(&db, "ask", "[]");
        db.conn
            .execute("UPDATE sessions SET inbox_ask = '{}' WHERE id = 'ask'", [])
            .unwrap();
        let ids: Vec<String> = db
            .list_recent_sessions(10)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["normal".to_string()]);
        assert!(db.get_session("ask").unwrap().is_none());
    }

    #[test]
    fn worktree_cwd_round_trips_and_sets_work_dir() {
        let db = monocode_db();
        let mut row = session("wt");
        row.cwd = "/projects/app".to_string();
        row.worktree_cwd = Some("/projects/app-worktrees/feature".to_string());
        db.upsert_session(&row).unwrap();

        let loaded = db.get_session("wt").unwrap().unwrap();
        assert_eq!(loaded.cwd, "/projects/app");
        assert_eq!(loaded.work_dir(), "/projects/app-worktrees/feature");

        row.worktree_cwd = Some(String::new());
        assert_eq!(row.work_dir(), "/projects/app", "empty worktree falls back");
    }

    #[test]
    fn list_sessions_for_cwd_filters_by_project_and_subdirectories() {
        let db = monocode_db();
        let mut s1 = session("s1");
        s1.cwd = "/projects/app".to_string();
        let mut s2 = session("s2");
        s2.cwd = "/projects/app/backend".to_string();
        let mut s3 = session("s3");
        s3.cwd = "/projects/other".to_string();
        let mut s4 = session("s4");
        s4.cwd = "/projects/apps".to_string();

        db.upsert_session(&s1).unwrap();
        db.upsert_session(&s2).unwrap();
        db.upsert_session(&s3).unwrap();
        db.upsert_session(&s4).unwrap();

        let app_sessions = db.list_sessions_for_cwd("/projects/app", 10).unwrap();
        let ids: Vec<String> = app_sessions.into_iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"s1".to_string()));
        assert!(ids.contains(&"s2".to_string()));
        assert!(!ids.contains(&"s3".to_string()));

        let other_sessions = db.list_sessions_for_cwd("/projects/other", 10).unwrap();
        assert_eq!(other_sessions.len(), 1);
        assert_eq!(other_sessions[0].id, "s3");
    }

    // ---- 6. Fallback ----

    #[test]
    fn in_memory_db_has_full_schema_and_is_usable() {
        let db = MonoCodeDb::open_in_memory().unwrap();
        db.upsert_session(&session("s1")).unwrap();
        assert_eq!(db.list_recent_sessions(5).unwrap().len(), 1);
    }

    #[test]
    fn legacy_bencode_schema_is_migrated() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, cwd TEXT NOT NULL, harness TEXT NOT NULL,
                model TEXT NOT NULL, model_settings TEXT NOT NULL DEFAULT '{}',
                runtime_mode TEXT NOT NULL DEFAULT 'auto', title TEXT NOT NULL,
                provider_session_id TEXT, blocks_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, branch TEXT,
                context_used INTEGER, context_window INTEGER,
                archived INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        let db = MonoCodeDb::from_connection(conn).unwrap();
        db.upsert_session(&session("s1")).unwrap();
        assert_eq!(db.list_recent_sessions(5).unwrap().len(), 1);
    }

    #[test]
    fn finish_automation_run_patches_status_and_keeps_unknown_fields() {
        let db = MonoCodeDb::open_in_memory().unwrap();
        db.save_automation(&AutomationRow {
            id: "a1".into(),
            name: "A".into(),
            ..Default::default()
        })
        .unwrap();
        db.conn
            .execute(
                "INSERT INTO automation_runs (id, automation_id, created_at, run_json) VALUES ('r1', 'a1', 1, ?1)",
                params![r#"{"id":"r1","automationId":"a1","trigger":"manual","scheduledFor":1,"createdAt":1,"status":"running","futureField":7}"#],
            )
            .unwrap();
        db.finish_automation_run("r1", "failed", Some("boom"))
            .unwrap();
        let json: String = db
            .conn
            .query_row(
                "SELECT run_json FROM automation_runs WHERE id = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let run: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(run["status"], "failed");
        assert_eq!(run["error"], "boom");
        assert_eq!(run["futureField"], 7);
        assert!(run["completedAt"].as_i64().is_some());
    }
}
