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
            "SELECT id, title, cwd, harness, model, created_at, updated_at, branch 
             FROM sessions 
             ORDER BY updated_at DESC 
             LIMIT ?1"
        )?;

        let session_iter = stmt.query_map([limit], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                branch: row.get(7)?,
            })
        })?;

        let mut sessions = Vec::new();
        for s in session_iter {
            sessions.push(s?);
        }
        Ok(sessions)
    }

    pub fn get_session_blocks_json(&self, session_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT blocks_json FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query([session_id])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            Ok(Some(json))
        } else {
            Ok(None)
        }
    }
}
