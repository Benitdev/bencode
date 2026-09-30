use anyhow::Result;
use rusqlite::Connection;
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
pub struct Block {
    pub id: String,
    pub role: String,
    pub text: Option<String>,
    #[serde(rename = "turnModel")]
    pub turn_model: Option<TurnModel>,
    pub tool: Option<serde_json::Value>,
    #[serde(rename = "startedAt")]
    pub started_at: Option<i64>,
}

pub struct MonoCodeDb {
    conn: Connection,
}

impl MonoCodeDb {
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")?;
        let db_path = PathBuf::from(home)
            .join("Library/Application Support/com.monocode.desktop/monocode.db");

        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        Ok(Self { conn })
    }

    pub fn list_recent_sessions(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, cwd, harness, model, created_at, updated_at, branch, blocks_json 
             FROM sessions 
             ORDER BY updated_at DESC 
             LIMIT ?1",
        )?;

        let session_iter = stmt.query_map([limit], |row| {
            let blocks_json: String = row.get(8)?;
            let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();

            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                branch: row.get(7)?,
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
            "SELECT id, title, cwd, harness, model, created_at, updated_at, branch, blocks_json 
             FROM sessions 
             WHERE id = ?1",
        )?;

        let mut rows = stmt.query([session_id])?;
        if let Some(row) = rows.next()? {
            let blocks_json: String = row.get(8)?;
            let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();

            Ok(Some(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                branch: row.get(7)?,
                blocks,
            }))
        } else {
            Ok(None)
        }
    }
}
