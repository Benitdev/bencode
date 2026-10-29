//! A Bot's artifacts, ported from MonoCode's `src-tauri/src/artifacts.rs`:
//! documents (Markdown) and visuals (a self-contained HTML page) kept in the
//! `notes` table as `content_kind = 'artifact'`, apart from the user's notes.
//! A chat refers to one by a card on the turn that wrote it
//! (`artifactCards` on the user block, `app/artifacts.rs`).

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{AppDb, now_millis, unique_slug};

/// MonoCode `ensure_notes_columns`: the column that keeps artifacts apart
/// from notes, and the one that says which kind an artifact is. Rows
/// written before (BenCode's, or an older MonoCode's) are notes.
pub(super) fn ensure_note_columns(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(notes)")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has = |name: &str| existing.iter().any(|col| col == name);
    if !has("content_kind") {
        conn.execute_batch(
            "ALTER TABLE notes ADD COLUMN content_kind TEXT NOT NULL DEFAULT 'note';",
        )?;
    }
    if !has("artifact_kind") {
        conn.execute_batch("ALTER TABLE notes ADD COLUMN artifact_kind TEXT;")?;
    }
    Ok(())
}

/// MonoCode `ArtifactKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    Document,
    Html,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Html => "html",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "document" => Some(Self::Document),
            "html" => Some(Self::Html),
            _ => None,
        }
    }

    /// MonoCode `artifactLabel`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Html => "Visual",
        }
    }
}

/// MonoCode `Artifact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_cwd: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// MonoCode `ArtifactUpsert`.
#[derive(Debug, Clone)]
pub struct ArtifactUpsert {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    pub body: String,
    pub source_session_id: Option<String>,
    pub source_cwd: Option<String>,
}

const SELECT: &str = "SELECT id, artifact_kind, title, body, source_session_id, source_cwd,
        created_at, updated_at
     FROM notes";

fn read_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let kind: Option<String> = row.get(1)?;
    let kind = kind
        .as_deref()
        .and_then(ArtifactKind::parse)
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Unsupported artifact kind".into()))?;
    Ok(Artifact {
        id: row.get(0)?,
        kind,
        title: row.get(2)?,
        body: row.get(3)?,
        source_session_id: row.get(4)?,
        source_cwd: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

impl AppDb {
    /// MonoCode `list_artifacts`: newest first. A row of a kind this
    /// version does not know is left out, not an error.
    pub fn list_artifacts(&self) -> Result<Vec<Artifact>> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT} WHERE content_kind = 'artifact' ORDER BY updated_at DESC, id ASC"
        ))?;
        let rows = stmt.query_map([], read_artifact)?;
        Ok(rows.filter_map(|row| row.ok()).collect())
    }

    pub fn get_artifact(&self, id: &str) -> Result<Option<Artifact>> {
        Ok(self
            .conn
            .query_row(
                &format!("{SELECT} WHERE id = ?1 AND content_kind = 'artifact'"),
                params![id],
                read_artifact,
            )
            .optional()?)
    }

    /// MonoCode `artifacts_upsert` (`upsert_content` with `kind =
    /// "artifact"`): a new one, or the saved one changed. A row that is a
    /// note, or an artifact of another kind, is never reclassified.
    pub fn upsert_artifact(&self, artifact: &ArtifactUpsert) -> Result<Artifact> {
        let existing: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT content_kind, artifact_kind FROM notes WHERE id = ?1",
                params![artifact.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if existing.as_ref().is_some_and(|(category, subtype)| {
            category != "artifact" || subtype.as_deref() != Some(artifact.kind.as_str())
        }) {
            bail!("Content belongs to a different kind");
        }
        let title = artifact.title.trim().to_string();
        let body = artifact.body.replace("\r\n", "\n").replace('\r', "\n");
        let source_session_id = artifact
            .source_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let source_cwd = artifact
            .source_cwd
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let now = now_millis();
        if let Some(current) = self.get_artifact(&artifact.id)? {
            let updated_at = if title == current.title && body == current.body {
                current.updated_at
            } else {
                now
            };
            let cwd = source_cwd.map(str::to_string).or(current.source_cwd);
            self.conn.execute(
                "UPDATE notes SET title = ?1, body = ?2, updated_at = ?3, source_cwd = ?4
                 WHERE id = ?5 AND content_kind = 'artifact'",
                params![title, body, updated_at, cwd, artifact.id],
            )?;
            return Ok(Artifact {
                id: artifact.id.clone(),
                kind: artifact.kind,
                title,
                body,
                source_session_id: current.source_session_id,
                source_cwd: cwd,
                created_at: current.created_at,
                updated_at,
            });
        }
        let slug = unique_slug(&self.conn, &title)?;
        self.conn.execute(
            "INSERT INTO notes (
                 id, slug, title, body, source_session_id, source_cwd, tags_json,
                 created_at, updated_at, content_kind, artifact_kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7, ?7, 'artifact', ?8)",
            params![
                artifact.id,
                slug,
                title,
                body,
                source_session_id,
                source_cwd,
                now,
                artifact.kind.as_str(),
            ],
        )?;
        Ok(Artifact {
            id: artifact.id.clone(),
            kind: artifact.kind,
            title,
            body,
            source_session_id: source_session_id.map(str::to_string),
            source_cwd: source_cwd.map(str::to_string),
            created_at: now,
            updated_at: now,
        })
    }

    /// MonoCode `delete_artifact`: the artifact, and every chat's card for
    /// it, in one transaction. Replies and the other cards stay.
    pub fn delete_artifact(&self, id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM notes WHERE id = ?1 AND content_kind = 'artifact'",
            [id],
        )?;
        // `json(..)` around the cards' subquery: the bundled SQLite (3.54.1)
        // loses the JSON subtype of its result, and `json_set` would store
        // the array as a string.
        tx.execute(
            "UPDATE sessions SET blocks_json = (
               SELECT json_group_array(json(CASE WHEN json_type(block.value, '$.artifactCards') = 'array'
                 THEN json_set(block.value, '$.artifactCards',
                   json((SELECT json_group_array(json(card.value))
                    FROM json_each(block.value, '$.artifactCards') AS card
                    WHERE json_extract(card.value, '$.id') != ?1)))
                 ELSE block.value END))
               FROM json_each(blocks_json) AS block)
             WHERE instr(blocks_json, '\"artifactCards\"') > 0
               AND EXISTS (SELECT 1 FROM json_each(blocks_json) AS block,
                             json_each(block.value, '$.artifactCards') AS card
                           WHERE json_extract(card.value, '$.id') = ?1)",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NoteUpsert;

    fn upsert(id: &str, kind: ArtifactKind, body: &str) -> ArtifactUpsert {
        ArtifactUpsert {
            id: id.into(),
            kind,
            title: "Report".into(),
            body: body.into(),
            source_session_id: Some("mono-1".into()),
            source_cwd: None,
        }
    }

    #[test]
    fn artifacts_keep_their_kind_and_stay_out_of_notes() {
        let db = AppDb::open_in_memory().unwrap();
        db.upsert_note(&NoteUpsert {
            id: "note-1".into(),
            title: "Personal".into(),
            body: "mine".into(),
            tags: Vec::new(),
            source_session_id: None,
            source_cwd: None,
        })
        .unwrap();
        db.upsert_artifact(&upsert("doc", ArtifactKind::Document, "# Report"))
            .unwrap();
        db.upsert_artifact(&upsert("chart", ArtifactKind::Html, "<p>hi</p>"))
            .unwrap();
        assert_eq!(
            db.get_artifact("chart").unwrap().unwrap().kind,
            ArtifactKind::Html
        );
        assert_eq!(db.list_artifacts().unwrap().len(), 2);
        let notes = db.list_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(db.get_note("doc").unwrap().is_none());
        assert!(db.get_artifact("note-1").unwrap().is_none());
        // Never reclassified, either way.
        assert!(
            db.upsert_artifact(&upsert("note-1", ArtifactKind::Document, "x"))
                .is_err()
        );
        assert!(
            db.upsert_artifact(&upsert("doc", ArtifactKind::Html, "x"))
                .is_err()
        );
        let json = serde_json::to_value(db.get_artifact("doc").unwrap().unwrap()).unwrap();
        assert_eq!(json["kind"], "document");
        assert_eq!(json["sourceSessionId"], "mono-1");
        assert!(json.get("tags").is_none());
    }

    #[test]
    fn a_revision_keeps_where_it_came_from() {
        let db = AppDb::open_in_memory().unwrap();
        let first = db
            .upsert_artifact(&upsert("doc", ArtifactKind::Document, "one"))
            .unwrap();
        let same = db
            .upsert_artifact(&upsert("doc", ArtifactKind::Document, "one"))
            .unwrap();
        assert_eq!(same.updated_at, first.updated_at);
        let revised = db
            .upsert_artifact(&ArtifactUpsert {
                source_session_id: None,
                ..upsert("doc", ArtifactKind::Document, "two")
            })
            .unwrap();
        assert_eq!(revised.body, "two");
        assert_eq!(revised.source_session_id.as_deref(), Some("mono-1"));
        assert_eq!(revised.created_at, first.created_at);
    }

    #[test]
    fn deleting_one_takes_its_cards_out_of_every_chat() {
        let db = AppDb::open_in_memory().unwrap();
        for id in ["remove", "keep"] {
            db.upsert_artifact(&upsert(id, ArtifactKind::Document, "body"))
                .unwrap();
        }
        let blocks = serde_json::json!([
            { "id": "turn", "role": "user", "text": "Review", "artifactCards": [
                { "id": "remove", "kind": "document", "title": "Report" },
                { "id": "keep", "kind": "document", "title": "Other" }
            ] },
            { "id": "reply", "role": "assistant", "text": "I created a report." }
        ]);
        db.conn
            .execute(
                "INSERT INTO sessions (id, cwd, harness, model, title, blocks_json, created_at, updated_at, runtime_mode)
                 VALUES ('s', '/tmp', 'claude', 'claude:opus', 'Chat', ?1, 1, 1, 'auto')",
                [blocks.to_string()],
            )
            .unwrap();
        db.delete_artifact("remove").unwrap();
        db.delete_artifact("remove").unwrap();
        assert!(db.get_artifact("remove").unwrap().is_none());
        assert!(db.get_artifact("keep").unwrap().is_some());
        let raw: String = db
            .conn
            .query_row(
                "SELECT blocks_json FROM sessions WHERE id = 's'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let saved: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            saved[0]["artifactCards"],
            serde_json::json!([{ "id": "keep", "kind": "document", "title": "Other" }])
        );
        assert_eq!(saved[1], blocks[1]);
    }
}
