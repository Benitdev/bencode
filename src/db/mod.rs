use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
             );"
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
}
