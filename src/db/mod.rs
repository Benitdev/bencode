use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub last_run_at: Option<i64>,
    pub last_run_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunRow {
    pub id: String,
    pub automation_id: String,
    pub trigger: String,
    pub scheduled_for: i64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub status: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnModel {
    pub harness: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(rename = "callId")]
    pub call_id: Option<String>,
    pub kind: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondOpinion {
    pub from: Option<String>,
    pub to: Option<String>,
    pub files: Option<u64>,
    pub request: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub role: String,
    pub text: Option<String>,
    #[serde(rename = "turnModel")]
    pub turn_model: Option<TurnModel>,
    pub tool: Option<serde_json::Value>,
    #[serde(rename = "secondOpinion")]
    pub second_opinion: Option<serde_json::Value>,
    #[serde(rename = "startedAt")]
    pub started_at: Option<i64>,
    #[serde(rename = "durationMs")]
    pub duration_ms: Option<i64>,
}

pub struct MonoCodeDb {
    conn: Connection,
}

impl MonoCodeDb {
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")?;
        let monocode_db_path = PathBuf::from(&home)
            .join("Library/Application Support/com.monocode.desktop/monocode.db");

        let db_path = if monocode_db_path.exists() {
            monocode_db_path
        } else {
            let bencode_dir = PathBuf::from(&home).join(".bencode");
            let _ = std::fs::create_dir_all(&bencode_dir);
            bencode_dir.join("bencode.db")
        };

        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 cwd TEXT NOT NULL,
                 harness TEXT NOT NULL,
                 model TEXT NOT NULL,
                 model_settings TEXT NOT NULL DEFAULT '{}',
                 runtime_mode TEXT NOT NULL DEFAULT 'auto',
                 title TEXT NOT NULL,
                 provider_session_id TEXT,
                 blocks_json TEXT NOT NULL DEFAULT '[]',
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 branch TEXT,
                 context_used INTEGER,
                 context_window INTEGER,
                 archived INTEGER NOT NULL DEFAULT 0,
                 pinned INTEGER NOT NULL DEFAULT 0
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
             CREATE INDEX IF NOT EXISTS automation_runs_history_idx ON automation_runs (automation_id, created_at DESC);"
        )?;

        Ok(Self { conn })
    }

    pub fn list_recent_sessions(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, cwd, harness, model, created_at, updated_at, branch, blocks_json,
                    context_used, context_window, pinned, archived 
             FROM sessions 
             ORDER BY updated_at DESC 
             LIMIT ?1",
        )?;

        let session_iter = stmt.query_map([limit], |row| {
            let blocks_json: String = row.get(8)?;
            let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();
            let pinned_int: i32 = row.get(11).unwrap_or(0);
            let archived_int: i32 = row.get(12).unwrap_or(0);

            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                branch: row.get(7)?,
                context_used: row.get(9)?,
                context_window: row.get(10)?,
                pinned: pinned_int != 0,
                archived: archived_int != 0,
                blocks,
            })
        })?;

        let mut sessions = Vec::new();
        for s in session_iter {
            sessions.push(s?);
        }
        Ok(sessions)
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, cwd, harness, model, created_at, updated_at, branch, blocks_json,
                    context_used, context_window, pinned, archived 
             FROM sessions 
             WHERE id = ?1",
        )?;

        let mut rows = stmt.query([session_id])?;
        if let Some(row) = rows.next()? {
            let blocks_json: String = row.get(8)?;
            let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();
            let pinned_int: i32 = row.get(11).unwrap_or(0);
            let archived_int: i32 = row.get(12).unwrap_or(0);

            Ok(Some(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                branch: row.get(7)?,
                context_used: row.get(9)?,
                context_window: row.get(10)?,
                pinned: pinned_int != 0,
                archived: archived_int != 0,
                blocks,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_session(&self, session: &SessionRow) -> Result<()> {
        let blocks_json = serde_json::to_string(&session.blocks).unwrap_or_else(|_| "[]".into());
        let pinned_int = if session.pinned { 1 } else { 0 };
        let archived_int = if session.archived { 1 } else { 0 };

        self.conn.execute(
            "INSERT INTO sessions (
                id, cwd, harness, model, title, blocks_json, created_at, updated_at,
                branch, context_used, context_window, pinned, archived
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
                archived = excluded.archived",
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
                pinned_int,
                archived_int,
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
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id],
        )?;
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
            let updated_at = if note.title == existing.title && note.body == existing.body && note.tags == existing.tags {
                existing.updated_at
            } else {
                now
            };
            let project_cwd = note.source_cwd.as_deref().or(existing.source_cwd.as_deref());
            self.conn.execute(
                "UPDATE notes
                 SET title = ?1, body = ?2, tags_json = ?3, updated_at = ?4, source_cwd = ?5
                 WHERE id = ?6",
                params![note.title, note.body, tags_json, updated_at, project_cwd, note.id],
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
        self.conn.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_automations(&self) -> Result<Vec<AutomationRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, definition_json FROM automations ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let json: String = row.get(1)?;
            Ok((id, json))
        })?;

        let mut automations = Vec::new();
        for row in rows {
            let (_id, raw) = row?;
            if let Ok(auto) = serde_json::from_str::<AutomationRow>(&raw) {
                automations.push(auto);
            }
        }
        Ok(automations)
    }

    pub fn save_automation(&self, auto: &AutomationRow) -> Result<()> {
        let json = serde_json::to_string(auto)?;
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
                if auto.enabled { 1 } else { 0 },
                auto.next_run_at,
                auto.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_automation(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM automations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn toggle_automation(&self, id: &str, enabled: bool) -> Result<()> {
        let enabled_int = if enabled { 1 } else { 0 };
        self.conn.execute(
            "UPDATE automations SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![enabled_int, now_millis(), id],
        )?;
        Ok(())
    }

    pub fn list_automation_runs(&self, automation_id: &str) -> Result<Vec<AutomationRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_json FROM automation_runs WHERE automation_id = ?1 ORDER BY created_at DESC LIMIT 50"
        )?;
        let rows = stmt.query_map(params![automation_id], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;

        let mut runs = Vec::new();
        for row in rows {
            let raw = row?;
            if let Ok(run) = serde_json::from_str::<AutomationRunRow>(&raw) {
                runs.push(run);
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
    if slug.is_empty() {
        "note".into()
    } else {
        slug
    }
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
