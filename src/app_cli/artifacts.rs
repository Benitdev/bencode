//! The `artifacts.*` actions (MonoCode `agentApp.ts` `handleArtifacts`,
//! `artifacts/artifacts.ts`): what each call takes, saving a document or a
//! visual, and the card a chat keeps for it on the turn that wrote it
//! (`artifactCards` on the user block, MonoCode `recordArtifactCard`).
//! Pure apart from the database, so it is tested without GPUI.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::input::{body, check_fields, id_chars, is_id, optional, page, required};
use crate::db::{AppDb, Artifact, ArtifactKind, ArtifactUpsert, Block};

/// The block field a chat keeps its cards in.
const CARDS: &str = "artifactCards";

/// MonoCode `ArtifactCard`: what a chat shows of an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactCard {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// One `artifacts.*` call, checked.
#[derive(Debug, PartialEq)]
pub enum Call {
    List {
        kind: Option<ArtifactKind>,
        limit: usize,
        offset: usize,
    },
    Read {
        id: String,
    },
    Write(Write),
}

/// `artifacts.write`: a new artifact, or the one `id` names revised.
#[derive(Debug, PartialEq)]
pub struct Write {
    pub id: Option<String>,
    pub kind: ArtifactKind,
    pub title: Option<String>,
    pub body: Option<String>,
    pub summary: Option<String>,
}

/// MonoCode `artifactKind`: a document when left out.
fn kind(value: Option<&Value>) -> Result<ArtifactKind, String> {
    match value {
        None => Ok(ArtifactKind::Document),
        Some(value) => value
            .as_str()
            .and_then(ArtifactKind::parse)
            .ok_or_else(|| "Unsupported artifact kind; use \"document\" or \"html\"".into()),
    }
}

/// Checks one call's input.
pub fn parse(action: &str, input: &Value) -> Result<Call, String> {
    let input = input
        .as_object()
        .ok_or_else(|| "Input must be a JSON object".to_string())?;
    check_fields(action, input)?;
    match action {
        "artifacts.list" => {
            let (limit, offset) = page(input)?;
            Ok(Call::List {
                kind: input.get("kind").map(|v| kind(Some(v))).transpose()?,
                limit,
                offset,
            })
        }
        "artifacts.read" => Ok(Call::Read {
            id: required(input.get("id"), "id", 256)?,
        }),
        _ => {
            let id = optional(input.get("id"), "id", 256)?;
            if id.as_deref().is_some_and(|id| !id_chars(id)) {
                return Err("Invalid artifact ID".into());
            }
            Ok(Call::Write(Write {
                id,
                kind: kind(input.get("kind"))?,
                title: optional(input.get("title"), "title", 200)?,
                body: input.get("body").map(body).transpose()?,
                summary: optional(input.get("summary"), "summary", 280)?,
            }))
        }
    }
}

/// `artifacts.list`'s answer: titles only, a page of them.
pub fn listing(
    all: Vec<Artifact>,
    kind: Option<ArtifactKind>,
    limit: usize,
    offset: usize,
) -> Value {
    let matching: Vec<Artifact> = all
        .into_iter()
        .filter(|artifact| kind.is_none_or(|kind| artifact.kind == kind))
        .collect();
    let page: Vec<Value> = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|a| json!({ "id": a.id, "kind": a.kind, "title": a.title, "updatedAt": a.updated_at }))
        .collect();
    json!({ "total": matching.len(), "offset": offset, "artifacts": page })
}

/// Saves what `write` asks, for the Mono whose conversation is
/// `session_id`. A new artifact's id comes from the request's, so a retry
/// with the same request ID saves nothing twice.
pub fn save(db: &AppDb, session_id: &str, request_id: &str, write: &Write) -> Result<Artifact> {
    if let Some(id) = &write.id {
        if write.title.is_none() && write.body.is_none() {
            bail!("Supply title or body to update an artifact");
        }
        let current = db
            .get_artifact(id)?
            .ok_or_else(|| anyhow!("Artifact was not found"))?;
        if current.kind != write.kind {
            bail!("An artifact's kind cannot be changed");
        }
        return db.upsert_artifact(&ArtifactUpsert {
            id: id.clone(),
            kind: write.kind,
            title: write.title.clone().unwrap_or(current.title),
            body: write.body.clone().unwrap_or(current.body),
            source_session_id: None,
            source_cwd: None,
        });
    }
    let Some(body) = &write.body else {
        bail!("body is required to create an artifact");
    };
    if !is_id(request_id) {
        bail!("Invalid request ID");
    }
    let id = format!("artifact-{session_id}-{request_id}");
    let title = write
        .title
        .clone()
        .unwrap_or_else(|| crate::app::notes::note_title(body));
    if let Some(existing) = db.get_artifact(&id)? {
        if existing.kind != write.kind || existing.title != title || &existing.body != body {
            bail!("Request ID was already used for another artifact");
        }
        return Ok(existing);
    }
    // A Mono works from the home folder, no one project.
    db.upsert_artifact(&ArtifactUpsert {
        id,
        kind: write.kind,
        title,
        body: body.clone(),
        source_session_id: Some(session_id.to_string()),
        source_cwd: None,
    })
}

/// The cards on one block.
pub fn block_cards(block: &Block) -> Vec<ArtifactCard> {
    block
        .extra
        .get(CARDS)
        .and_then(|cards| serde_json::from_value(cards.clone()).ok())
        .unwrap_or_default()
}

/// Whether a block holds any cards (cheap: no parsing).
pub fn has_cards(block: &Block) -> bool {
    block
        .extra
        .get(CARDS)
        .and_then(Value::as_array)
        .is_some_and(|cards| !cards.is_empty())
}

/// MonoCode `artifactCards`: every card in a chat, once each, in the order
/// they were first written.
pub fn chat_cards(blocks: &[Block]) -> Vec<ArtifactCard> {
    let mut cards: Vec<ArtifactCard> = Vec::new();
    for block in blocks.iter().filter(|block| has_cards(block)) {
        for card in block_cards(block) {
            match cards.iter_mut().find(|known| known.id == card.id) {
                Some(known) => *known = card,
                None => cards.push(card),
            }
        }
    }
    cards
}

/// MonoCode `recordArtifactCard`: the card goes on the turn's message (the
/// last one the user sent); a revision or a retry replaces its card there.
/// False when the chat has no message to put it on.
pub fn record_card(blocks: &mut [Block], card: &ArtifactCard) -> bool {
    let Some(anchor) = blocks.iter_mut().rev().find(|block| block.role == "user") else {
        return false;
    };
    let mut cards = block_cards(anchor);
    cards.retain(|known| known.id != card.id);
    cards.push(card.clone());
    match serde_json::to_value(&cards) {
        Ok(value) => {
            anchor.extra.insert(CARDS.into(), value);
            true
        }
        Err(err) => {
            log::error!("artifact card not recorded: {err}");
            false
        }
    }
}

/// MonoCode `removeArtifactCard`: takes the card for `id` out of every
/// block, keeping the rest. True when one was there.
pub fn remove_card(blocks: &mut [Block], id: &str) -> bool {
    let mut changed = false;
    for block in blocks.iter_mut().filter(|block| has_cards(block)) {
        let cards = block_cards(block);
        if !cards.iter().any(|card| card.id == id) {
            continue;
        }
        changed = true;
        let rest: Vec<ArtifactCard> = cards.into_iter().filter(|card| card.id != id).collect();
        if rest.is_empty() {
            block.extra.remove(CARDS);
        } else if let Ok(value) = serde_json::to_value(&rest) {
            block.extra.insert(CARDS.into(), value);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_checked_like_monocodes() {
        assert_eq!(
            parse("artifacts.list", &json!({})).unwrap(),
            Call::List {
                kind: None,
                limit: 30,
                offset: 0
            }
        );
        assert!(parse("artifacts.list", &json!({ "limit": 0 })).is_err());
        assert!(parse("artifacts.list", &json!({ "kind": "code" })).is_err());
        assert!(parse("artifacts.read", &json!({ "id": " " })).is_err());
        assert!(parse("artifacts.read", &json!({ "id": "a", "x": 1 })).is_err());
        let Call::Write(write) = parse(
            "artifacts.write",
            &json!({ "kind": "html", "title": " Chart ", "body": "<p>\r\n</p>" }),
        )
        .unwrap() else {
            panic!("a write");
        };
        assert_eq!(write.kind, ArtifactKind::Html);
        assert_eq!(write.title.as_deref(), Some("Chart"));
        assert_eq!(write.body.as_deref(), Some("<p>\n</p>"));
        assert!(parse("artifacts.write", &json!({ "id": "../x", "body": "b" })).is_err());
    }

    fn write(body: &str) -> Write {
        Write {
            id: None,
            kind: ArtifactKind::Document,
            title: None,
            body: Some(body.into()),
            summary: None,
        }
    }

    #[test]
    fn a_retry_saves_nothing_twice() {
        let db = AppDb::open_in_memory().unwrap();
        let saved = save(&db, "mono", "r1", &write("# Plan\n\nSteps")).unwrap();
        assert_eq!(saved.id, "artifact-mono-r1");
        assert_eq!(saved.title, "Plan");
        assert_eq!(
            save(&db, "mono", "r1", &write("# Plan\n\nSteps")).unwrap(),
            saved
        );
        assert!(save(&db, "mono", "r1", &write("other")).is_err());
        assert_eq!(db.list_artifacts().unwrap().len(), 1);
    }

    #[test]
    fn a_revision_keeps_its_kind_and_title() {
        let db = AppDb::open_in_memory().unwrap();
        let saved = save(&db, "mono", "r1", &write("# Plan")).unwrap();
        let revised = save(
            &db,
            "mono",
            "r2",
            &Write {
                id: Some(saved.id.clone()),
                ..write("# Plan v2")
            },
        )
        .unwrap();
        assert_eq!(
            (revised.title.as_str(), revised.body.as_str()),
            ("Plan", "# Plan v2")
        );
        let as_html = Write {
            id: Some(saved.id.clone()),
            kind: ArtifactKind::Html,
            ..write("<p>")
        };
        assert!(save(&db, "mono", "r3", &as_html).is_err());
        let missing = Write {
            id: Some("nope".into()),
            ..write("x")
        };
        assert!(save(&db, "mono", "r4", &missing).is_err());
    }

    #[test]
    fn listing_pages_titles() {
        let db = AppDb::open_in_memory().unwrap();
        for ix in 0..3 {
            save(
                &db,
                "mono",
                &format!("r{ix}"),
                &write(&format!("# Doc {ix}")),
            )
            .unwrap();
        }
        let page = listing(db.list_artifacts().unwrap(), None, 2, 1);
        assert_eq!(page["total"], 3);
        assert_eq!(page["artifacts"].as_array().unwrap().len(), 2);
        assert!(page["artifacts"][0].get("body").is_none());
        let none = listing(
            db.list_artifacts().unwrap(),
            Some(ArtifactKind::Html),
            30,
            0,
        );
        assert_eq!(none["total"], 0);
    }

    fn card(id: &str, title: &str) -> ArtifactCard {
        ArtifactCard {
            id: id.into(),
            kind: ArtifactKind::Document,
            title: title.into(),
            summary: None,
        }
    }

    #[test]
    fn cards_land_on_the_turns_message_once() {
        let mut blocks = vec![
            Block::new("u1", "user", "first"),
            Block::new("a1", "assistant", "done"),
            Block::new("u2", "user", "second"),
            Block::new("a2", "assistant", "working"),
        ];
        assert!(record_card(&mut blocks, &card("d", "Draft")));
        assert!(record_card(&mut blocks, &card("e", "Other")));
        assert!(record_card(&mut blocks, &card("d", "Final")));
        assert!(block_cards(&blocks[0]).is_empty());
        assert_eq!(
            block_cards(&blocks[2]),
            vec![card("e", "Other"), card("d", "Final")]
        );
        let json = serde_json::to_value(&blocks[2]).unwrap();
        assert_eq!(
            json["artifactCards"][1],
            json!({ "id": "d", "kind": "document", "title": "Final" })
        );
        assert_eq!(chat_cards(&blocks).len(), 2);
        assert!(remove_card(&mut blocks, "d"));
        assert!(!remove_card(&mut blocks, "d"));
        assert_eq!(chat_cards(&blocks), vec![card("e", "Other")]);
        assert!(remove_card(&mut blocks, "e"));
        assert!(!blocks[2].extra.contains_key("artifactCards"));
        assert!(!record_card(
            &mut [Block::new("a", "assistant", "x")],
            &card("d", "D")
        ));
    }
}
