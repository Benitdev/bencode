//! What every `bencode app` action's input is checked with (MonoCode
//! `agentApp.ts`'s `FIELDS`, `requiredString`, `optionalString`): an
//! unknown field is an error rather than ignored, and strings are trimmed
//! and capped.

use serde_json::{Map, Value};

/// MonoCode `FIELDS`: every action and the fields its input may hold.
pub const FIELDS: &[(&str, &[&str])] = &[
    ("models.list", &[]),
    ("sessions.list", &["project"]),
    ("sessions.read", &["sessionId", "before", "limit", "maxChars", "project"]),
    ("sessions.send", &["sessionId", "prompt", "project", "notifyOnComplete"]),
    ("sessions.draft", &["sessionId", "prompt", "project"]),
    ("sessions.stop", &["sessionId", "project"]),
    ("sessions.archive", &["sessionId", "project"]),
    ("sessions.delete", &["sessionId", "project"]),
    (
        "sessions.start",
        &[
            "prompt",
            "draft",
            "harness",
            "model",
            "modelSettings",
            "effort",
            "runtimeMode",
            "reveal",
            "workspaceMode",
            "worktreeBase",
            "worktreeCwd",
            "placement",
            "besideSessionId",
            "project",
            "notifyOnComplete",
        ],
    ),
    ("worktrees.list", &["project"]),
    ("worktrees.create", &["branch", "base", "existing", "project"]),
    ("folders.list", &["project"]),
    ("folders.move", &["sessionId", "folderId", "newFolderName", "project"]),
    ("notes.list", &["limit", "offset"]),
    ("notes.read", &["id"]),
    ("notes.write", &["id", "title", "body", "tags"]),
    ("artifacts.list", &["kind", "limit", "offset"]),
    ("artifacts.read", &["id"]),
    ("artifacts.write", &["id", "kind", "title", "body", "summary"]),
    ("soul.read", &[]),
    ("soul.update", &["text", "expectedHash"]),
    ("memory.read", &["topic"]),
    ("memory.search", &["query", "since"]),
    ("memory.add", &["fact", "topic", "until"]),
    ("memory.replace", &["find", "fact", "topic", "until"]),
    ("memory.remove", &["find", "topic"]),
    ("habits.list", &[]),
    ("habits.add", &["name", "instructions", "schedule"]),
    ("habits.update", &["id", "name", "instructions", "schedule", "enabled"]),
    ("habits.run", &["id"]),
    ("habits.remove", &["id"]),
    ("board.list", &[]),
    ("board.add", &["title", "prompt", "project", "sessionId"]),
    ("board.update", &["id", "title", "prompt", "done"]),
    ("board.remove", &["id"]),
    (
        "board.start",
        &[
            "id",
            "harness",
            "model",
            "modelSettings",
            "effort",
            "runtimeMode",
            "workspaceMode",
            "worktreeBase",
            "worktreeCwd",
            "notifyOnComplete",
        ],
    ),
    (
        "chat.card",
        &[
            "type",
            "repo",
            "number",
            "note",
            "sessionId",
            "options",
            "name",
            "instructions",
            "schedule",
        ],
    ),
];

/// Whether `action` is one the CLI knows.
pub fn is_action(action: &str) -> bool {
    FIELDS.iter().any(|(name, _)| *name == action)
}

/// The actions, for the CLI's error.
pub fn actions() -> Vec<&'static str> {
    FIELDS.iter().map(|(name, _)| *name).collect()
}

/// MonoCode `fields`: an unknown field is an error, not ignored.
pub fn check_fields(action: &str, input: &Map<String, Value>) -> Result<(), String> {
    let Some((_, allowed)) = FIELDS.iter().find(|(name, _)| *name == action) else {
        return Err(format!("Unknown app action: {action}"));
    };
    let unknown: Vec<&str> = input
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!("Unknown {action} fields: {}", unknown.join(", ")))
    }
}

/// MonoCode `requiredString`: trimmed, not empty, at most `max` characters.
pub fn required(value: Option<&Value>, name: &str, max: usize) -> Result<String, String> {
    match value.and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() && text.chars().count() <= max => {
            Ok(text.trim().to_string())
        }
        _ => Err(format!(
            "{name} must be a non-empty string under {max} characters"
        )),
    }
}

/// MonoCode `optionalString`.
pub fn optional(value: Option<&Value>, name: &str, max: usize) -> Result<Option<String>, String> {
    value.map(|_| required(value, name, max)).transpose()
}

/// A boolean field, or `default` when left out.
pub fn flag(value: Option<&Value>, name: &str, default: bool) -> Result<bool, String> {
    match value {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name} must be a boolean")),
    }
}

/// `limit` (1–100, 30 by default) and `offset` (0 by default), as the
/// list actions take them.
pub fn page(input: &Map<String, Value>) -> Result<(usize, usize), String> {
    let limit = match input.get("limit") {
        None => 30,
        Some(value) => match value.as_u64() {
            Some(n) if (1..=100).contains(&n) => n as usize,
            _ => return Err("limit must be an integer from 1 to 100".into()),
        },
    };
    let offset = match input.get("offset") {
        None => 0,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| "offset must be a non-negative integer".to_string())?
            as usize,
    };
    Ok((limit, offset))
}

/// MonoCode's `/^[A-Za-z0-9_-]{1,128}$/`, for request IDs.
pub fn is_id(id: &str) -> bool {
    id.len() <= 128 && id_chars(id)
}

/// MonoCode's `/^[A-Za-z0-9_-]+$/`, for the id of a saved record (which
/// may be longer: `artifact-<session>-<request>`).
pub fn id_chars(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// MonoCode `noteBody`.
pub fn body(value: &Value) -> Result<String, String> {
    match value.as_str() {
        Some(text) if text.chars().count() <= 240_000 => {
            Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
        }
        _ => Err("body must be a string under 240000 characters".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inputs_are_checked() {
        let input = json!({ "id": "a", "x": 1 });
        let err = check_fields("notes.read", input.as_object().unwrap()).unwrap_err();
        assert_eq!(err, "Unknown notes.read fields: x");
        assert!(check_fields("sessions.nope", &Map::new()).is_err());
        assert_eq!(required(Some(&json!(" a ")), "id", 5).unwrap(), "a");
        assert!(required(Some(&json!("")), "id", 5).is_err());
        assert!(required(Some(&json!(1)), "id", 5).is_err());
        assert_eq!(optional(None, "id", 5).unwrap(), None);
        assert!(flag(Some(&json!("yes")), "draft", false).is_err());
        let paged = json!({ "limit": 5, "offset": 2 });
        assert_eq!(page(paged.as_object().unwrap()).unwrap(), (5, 2));
        assert!(page(json!({ "limit": 0 }).as_object().unwrap()).is_err());
        assert!(is_id("app-1_x") && !is_id("a b") && !is_id(""));
    }
}
