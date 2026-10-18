//! The `notes.*` actions (MonoCode `agentApp.ts`, `notes.list` /
//! `notes.read` / `notes.write`): the user's Notes, read and written by an
//! agent with app access. A new note's id comes from the request's, so a
//! retry saves nothing twice. Database work, for the database thread.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::input::{body, id_chars, is_id, optional, page};
use crate::app::notes::{normalize_note_tags, note_title};
use crate::db::{AppDb, Note, NoteUpsert};

/// MonoCode `notePreview`: two short body paragraphs, capped.
pub fn note_preview(body: &str) -> String {
    let paragraphs: Vec<String> = body
        .trim()
        .split("\n\n")
        .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|paragraph| !paragraph.is_empty())
        .take(2)
        .collect();
    paragraphs.join("\n\n").chars().take(400).collect()
}

fn tags(value: &Value) -> Result<Vec<String>, String> {
    let list = value.as_array().filter(|list| {
        list.len() <= 20
            && list
                .iter()
                .all(|tag| tag.as_str().is_some_and(|tag| tag.chars().count() <= 48))
    });
    match list {
        Some(list) => Ok(normalize_note_tags(
            &list.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
        )),
        None => Err("tags must be an array of at most 20 strings under 48 characters each".into()),
    }
}

/// A note as the CLI returns it.
fn view(note: &Note) -> Value {
    let mut out = json!({
        "id": note.id,
        "title": note.title,
        "body": note.body,
        "tags": note.tags,
        "createdAt": note.created_at,
        "updatedAt": note.updated_at,
    });
    if let Some(session) = &note.source_session_id {
        out["sourceSessionId"] = json!(session);
    }
    if let Some(cwd) = &note.source_cwd {
        out["sourceCwd"] = json!(cwd);
    }
    out
}

/// One `notes.*` call, checked before it reaches the database.
#[derive(Debug, PartialEq)]
pub enum Call {
    List { limit: usize, offset: usize },
    Read { id: String },
    Write(Write),
}

#[derive(Debug, PartialEq)]
pub struct Write {
    pub id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
}

pub fn parse(action: &str, input: &Map<String, Value>) -> Result<Call, String> {
    match action {
        "notes.list" => {
            let (limit, offset) = page(input)?;
            Ok(Call::List { limit, offset })
        }
        "notes.read" => Ok(Call::Read {
            id: super::input::required(input.get("id"), "id", 256)?,
        }),
        _ => {
            let id = optional(input.get("id"), "id", 256)?;
            if id.as_deref().is_some_and(|id| !id_chars(id)) {
                return Err("Invalid note ID".into());
            }
            Ok(Call::Write(Write {
                id,
                title: optional(input.get("title"), "title", 200)?,
                body: input.get("body").map(body).transpose()?,
                tags: input.get("tags").map(tags).transpose()?,
            }))
        }
    }
}

/// Runs a checked call for the session `source` (working in `cwd`, when
/// that is a project).
pub fn run(
    db: &AppDb,
    call: &Call,
    source: &str,
    cwd: Option<&str>,
    request_id: &str,
) -> Result<Value> {
    match call {
        Call::List { limit, offset } => {
            let notes = db.list_notes()?;
            let page: Vec<Value> = notes
                .iter()
                .skip(*offset)
                .take(*limit)
                .map(|note| {
                    json!({
                        "id": note.id,
                        "title": note.title,
                        "preview": note_preview(&note.body),
                        "tags": note.tags,
                        "sourceCwd": note.source_cwd,
                    })
                })
                .collect();
            Ok(json!({ "total": notes.len(), "offset": offset, "notes": page }))
        }
        Call::Read { id } => db
            .get_note(id)?
            .map(|note| view(&note))
            .ok_or_else(|| anyhow!("Note was not found")),
        Call::Write(write) => {
            if let Some(id) = &write.id {
                if write.title.is_none() && write.body.is_none() && write.tags.is_none() {
                    bail!("Supply title, body or tags to update a note");
                }
                let current = db
                    .get_note(id)?
                    .ok_or_else(|| anyhow!("Note was not found"))?;
                let saved = db.upsert_note(&NoteUpsert {
                    id: id.clone(),
                    title: write.title.clone().unwrap_or(current.title),
                    body: write.body.clone().unwrap_or(current.body),
                    tags: write.tags.clone().unwrap_or(current.tags),
                    source_session_id: None,
                    source_cwd: None,
                })?;
                return Ok(view(&saved));
            }
            let Some(body) = &write.body else {
                bail!("body is required to create a note");
            };
            if !is_id(request_id) {
                bail!("Invalid request ID");
            }
            let id = format!("app-{source}-{request_id}");
            let title = write.title.clone().unwrap_or_else(|| note_title(body));
            let tags = write.tags.clone().unwrap_or_default();
            if let Some(existing) = db.get_note(&id)? {
                if existing.title != title || &existing.body != body || existing.tags != tags {
                    bail!("Request ID was already used for another note");
                }
                return Ok(view(&existing));
            }
            let saved = db.upsert_note(&NoteUpsert {
                id,
                title,
                body: body.clone(),
                tags,
                source_session_id: Some(source.to_string()),
                source_cwd: cwd.map(str::to_string),
            })?;
            Ok(view(&saved))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn notes_are_listed_read_and_written_once() {
        let db = AppDb::open_in_memory().unwrap();
        let write = parse(
            "notes.write",
            &input(json!({ "body": "# Plan\n\nSteps", "tags": ["#Work"] })),
        )
        .unwrap();
        let saved = run(&db, &write, "s1", Some("/src/app"), "r1").unwrap();
        assert_eq!(saved["id"], "app-s1-r1");
        assert_eq!(saved["title"], "Plan");
        assert_eq!(saved["tags"], json!(["work"]));
        assert_eq!(
            run(&db, &write, "s1", Some("/src/app"), "r1").unwrap()["id"],
            "app-s1-r1"
        );
        let other = parse("notes.write", &input(json!({ "body": "other" }))).unwrap();
        assert!(run(&db, &other, "s1", None, "r1").is_err());
        let edit = parse(
            "notes.write",
            &input(json!({ "id": "app-s1-r1", "title": "Plan v2" })),
        )
        .unwrap();
        assert_eq!(
            run(&db, &edit, "s1", None, "r2").unwrap()["body"],
            "# Plan\n\nSteps"
        );
        let list = run(
            &db,
            &parse("notes.list", &input(json!({}))).unwrap(),
            "s1",
            None,
            "r3",
        )
        .unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["notes"][0]["preview"], "# Plan\n\nSteps");
        assert!(parse("notes.write", &input(json!({ "tags": "x" }))).is_err());
        assert!(
            run(
                &db,
                &parse("notes.read", &input(json!({ "id": "nope" }))).unwrap(),
                "s1",
                None,
                "r"
            )
            .is_err()
        );
    }
}
