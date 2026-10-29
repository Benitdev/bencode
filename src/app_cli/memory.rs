//! The `soul.*` and `memory.*` actions (MonoCode `agentApp.ts`
//! `handleSoul`, `handleMemory`, `editAgentFile`, `keepMemoryInBudget`):
//! a Bot reads and changes its own files through these, never by editing
//! them. Every write names the version it was made over (`bots/files.rs`),
//! so the user's edits in the settings dialog are never overwritten; a
//! memory edit is reapplied to the latest file when one lands in between.
//! Blocking file work, for the background executor.

use std::path::Path;

use serde_json::{Map, Value, json};

use super::input::{optional, required};
use crate::bots::files::{self, MEMORY_FILE, SOUL_FILE, SaveError};
use crate::bots::memory;

/// MonoCode `memoryPath`: `MEMORY.md`, or one topic file.
fn memory_path(topic: Option<&Value>) -> Result<String, String> {
    match topic {
        None => Ok(MEMORY_FILE.to_string()),
        Some(value) => {
            let name = memory::topic_name(&required(Some(value), "topic", 80)?)?;
            Ok(format!("memory/{name}.md"))
        }
    }
}

/// `soul.read` / `soul.update`, for the Bot whose files are in `dir`.
pub fn soul(dir: &Path, action: &str, input: &Map<String, Value>) -> Result<Value, String> {
    if action == "soul.read" {
        let files = files::load(dir).map_err(|err| format!("{err:#}"))?;
        return Ok(json!({ "file": SOUL_FILE, "text": files.soul, "hash": files.soul_hash }));
    }
    let text = match input.get("text").and_then(Value::as_str) {
        Some(text) if text.chars().count() <= 240_000 => text,
        _ => return Err("text must be a string under 240000 characters".into()),
    };
    let expected = required(input.get("expectedHash"), "expectedHash", 128)?;
    match files::save(dir, SOUL_FILE, text, Some(&expected)) {
        Ok(hash) => Ok(json!({ "file": SOUL_FILE, "updated": true, "hash": hash })),
        Err(SaveError::Conflict) => Err("SOUL.md changed since you read it. Run soul.read and \
             reapply the user's requested changes to the current text before calling \
             soul.update again."
            .into()),
        Err(SaveError::Failed(err)) => Err(format!("{err:#}")),
    }
}

/// MonoCode `editAgentFile`: a topic file that does not exist yet starts
/// with its heading.
fn edit_memory<T>(
    dir: &Path,
    path: &str,
    change: impl Fn(&str) -> Result<(String, T), String>,
) -> Result<T, String> {
    let missing = files::read(dir, path)
        .map_err(|err| format!("{err:#}"))?
        .0
        .is_none();
    let fallback = path
        .strip_prefix("memory/")
        .and_then(|name| name.strip_suffix(".md"))
        .filter(|_| missing)
        .map(|name| format!("# {name}\n\n"));
    let result = std::cell::RefCell::new(None);
    files::edit(dir, path, |current| {
        let start = match &fallback {
            Some(heading) if current.is_empty() => heading.as_str(),
            _ => current,
        };
        let (text, value) = change(start)?;
        *result.borrow_mut() = Some(value);
        Ok(text)
    })?;
    result
        .into_inner()
        .ok_or_else(|| "The memory edit did not run".to_string())
}

/// MonoCode `keepMemoryInBudget`: the archive is written first, so a line
/// is never out of one file without already being in the other. A save the
/// user made in between leaves the trim for the next write.
fn keep_in_budget(dir: &Path, keep: &str, date: &str) -> Result<Vec<String>, String> {
    let (text, hash) = files::read(dir, MEMORY_FILE).map_err(|err| format!("{err:#}"))?;
    let (fitted, moved) =
        memory::fit_memory_budget(text.as_deref().unwrap_or(""), Some(keep), date);
    if moved.is_empty() {
        return Ok(moved);
    }
    edit_memory(dir, "memory/archive.md", |archive| {
        Ok((memory::archive_memory_entries(archive, &moved, date), ()))
    })?;
    match files::save(dir, MEMORY_FILE, &fitted, Some(&hash)) {
        Ok(_) => Ok(moved),
        Err(SaveError::Conflict) => Ok(Vec::new()),
        Err(SaveError::Failed(err)) => Err(format!("{err:#}")),
    }
}

/// The memory files a search reads: `MEMORY.md`, every topic, the archive.
fn search_files(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let loaded = files::load(dir).map_err(|err| format!("{err:#}"))?;
    let mut out = vec![(MEMORY_FILE.to_string(), loaded.memory)];
    for name in loaded
        .topics
        .iter()
        .map(|topic| format!("memory/{topic}.md"))
        .chain(["memory/archive.md".to_string()])
    {
        let (text, _) = files::read(dir, &name).map_err(|err| format!("{err:#}"))?;
        out.push((name, text.unwrap_or_default()));
    }
    Ok(out)
}

/// `memory.*`, for the Bot whose files are in `dir`; `today` dates the
/// entries.
pub fn memory(
    dir: &Path,
    action: &str,
    input: &Map<String, Value>,
    today: jiff::Zoned,
) -> Result<Value, String> {
    let path = memory_path(input.get("topic"))?;
    let date = today.date().strftime("%Y-%m-%d").to_string();
    let until = optional(input.get("until"), "until", 10)?;
    let report = |mut result: Value, keep: Option<&str>| -> Result<Value, String> {
        result["file"] = json!(path);
        if path == MEMORY_FILE
            && let Some(keep) = keep
        {
            let moved = keep_in_budget(dir, keep, &date)?;
            if !moved.is_empty() {
                result["movedToArchive"] = json!(moved.len());
                result["moved"] = json!(moved.iter().take(5).collect::<Vec<_>>());
            }
        }
        Ok(result)
    };
    match action {
        "memory.search" => {
            let query = optional(input.get("query"), "query", 500)?.unwrap_or_default();
            let since = optional(input.get("since"), "since", 20)?
                .map(|since| memory::since_date(&since, today.clone()))
                .transpose()?;
            let hits = memory::search_memory(&search_files(dir)?, &query, since.as_deref(), 20)?;
            if hits.is_empty() {
                Ok(
                    json!({ "hits": [], "note": "Nothing in memory matches. It may never have been saved." }),
                )
            } else {
                Ok(json!({ "hits": hits }))
            }
        }
        "memory.read" => {
            if path != MEMORY_FILE {
                let (text, _) = files::read(dir, &path).map_err(|err| format!("{err:#}"))?;
                let text = text.ok_or_else(|| "No such memory topic".to_string())?;
                return Ok(json!({ "file": path, "text": text }));
            }
            let loaded = files::load(dir).map_err(|err| format!("{err:#}"))?;
            let budget = memory::memory_within_budget(&loaded.memory);
            Ok(json!({
                "file": path,
                "text": loaded.memory,
                "lines": budget.lines,
                "notLoaded": budget.dropped_lines,
                "topics": loaded.topics,
            }))
        }
        "memory.add" => {
            let fact = required(input.get("fact"), "fact", 30_000)?;
            let entry = memory::memory_entry(&fact, &date, until.as_deref())?;
            let added = edit_memory(dir, &path, |text| {
                Ok(memory::add_memory_entry(text, &entry))
            })?;
            if added {
                report(json!({ "added": entry }), Some(&entry))
            } else {
                Ok(json!({ "file": path, "alreadyRemembered": true }))
            }
        }
        "memory.replace" => {
            let find = required(input.get("find"), "find", 2000)?;
            let fact = required(input.get("fact"), "fact", 30_000)?;
            let entry = memory::memory_entry(&fact, &date, until.as_deref())?;
            edit_memory(dir, &path, |text| {
                Ok((
                    memory::supersede_memory_entry(text, &find, &entry, &date)?,
                    (),
                ))
            })?;
            report(json!({ "superseded": find, "added": entry }), Some(&entry))
        }
        "memory.remove" => {
            let find = required(input.get("find"), "find", 2000)?;
            let removed = edit_memory(dir, &path, |text| memory::remove_memory_entry(text, &find))?;
            Ok(json!({ "file": path, "removed": removed }))
        }
        _ => Err(format!("Unknown app action: {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-app-memory-{}",
            crate::app_cli::random_hex(6)
        ));
        files::load(&dir).unwrap();
        dir
    }

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn today() -> jiff::Zoned {
        "2026-10-08T12:00:00[UTC]".parse().unwrap()
    }

    #[test]
    fn the_agent_keeps_its_memory_through_the_cli() {
        let dir = dir();
        let added = memory(
            &dir,
            "memory.add",
            &input(json!({ "fact": "likes tea" })),
            today(),
        )
        .unwrap();
        assert_eq!(added["added"], "- 2026-10-08 · likes tea");
        let again = memory(
            &dir,
            "memory.add",
            &input(json!({ "fact": "likes  tea" })),
            today(),
        )
        .unwrap();
        assert_eq!(again["alreadyRemembered"], true);
        memory(
            &dir,
            "memory.replace",
            &input(json!({ "find": "tea", "fact": "likes coffee" })),
            today(),
        )
        .unwrap();
        let read = memory(&dir, "memory.read", &input(json!({})), today()).unwrap();
        assert_eq!(
            read["text"],
            "- ~~2026-10-08 · likes tea~~ · superseded 2026-10-08\n- 2026-10-08 · likes coffee\n"
        );
        memory(
            &dir,
            "memory.add",
            &input(json!({ "fact": "tags are vX.Y.Z", "topic": "releases" })),
            today(),
        )
        .unwrap();
        let topic = memory(
            &dir,
            "memory.read",
            &input(json!({ "topic": "releases" })),
            today(),
        )
        .unwrap();
        assert_eq!(
            topic["text"],
            "# releases\n- 2026-10-08 · tags are vX.Y.Z\n"
        );
        let hits = memory(
            &dir,
            "memory.search",
            &input(json!({ "query": "coffee" })),
            today(),
        )
        .unwrap();
        assert_eq!(hits["hits"][0]["file"], "MEMORY.md");
        let removed = memory(
            &dir,
            "memory.remove",
            &input(json!({ "find": "coffee" })),
            today(),
        )
        .unwrap();
        assert_eq!(removed["removed"], "- 2026-10-08 · likes coffee");
        assert!(
            memory(
                &dir,
                "memory.read",
                &input(json!({ "topic": "nope" })),
                today()
            )
            .is_err()
        );
        assert!(
            memory(
                &dir,
                "memory.add",
                &input(json!({ "fact": "x", "topic": "../soul" })),
                today()
            )
            .is_err()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_memory_moves_its_oldest_to_the_archive() {
        let dir = dir();
        let full: String = (0..200)
            .map(|n| format!("- 2026-01-01 · fact {n}\n"))
            .collect();
        files::save(&dir, MEMORY_FILE, &full, None).unwrap();
        let added = memory(
            &dir,
            "memory.add",
            &input(json!({ "fact": "newest" })),
            today(),
        )
        .unwrap();
        assert_eq!(added["movedToArchive"], 1);
        let (archive, _) = files::read(&dir, "memory/archive.md").unwrap();
        assert_eq!(
            archive.unwrap(),
            "# archive\n- 2026-01-01 · fact 0 · moved 2026-10-08\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_soul_saves_only_over_what_was_read() {
        let dir = dir();
        let read = soul(&dir, "soul.read", &input(json!({}))).unwrap();
        let hash = read["hash"].as_str().unwrap().to_string();
        let saved = soul(
            &dir,
            "soul.update",
            &input(json!({ "text": "Be brief.", "expectedHash": hash })),
        )
        .unwrap();
        assert_eq!(saved["updated"], true);
        let stale = soul(
            &dir,
            "soul.update",
            &input(json!({ "text": "x", "expectedHash": hash })),
        );
        assert!(stale.unwrap_err().starts_with("SOUL.md changed"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
