//! BenCode's own: removes what a CLI keeps of one session on disk, for a
//! thread deleted with "also delete the CLI's session" (MonoCode only
//! deletes its own row). Afterwards the CLI can no longer resume it.
//!
//! - Claude: `projects/<cwd slug>/<id>.jsonl` and its `<id>/` beside it,
//!   `session-env/<id>`, `file-history/<id>`, in every config folder
//!   (a thread moved between accounts left copies in each).
//! - Codex: `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl` and
//!   `archived_sessions/`, in `CODEX_HOME` and every account profile.
//! - Grok Build: `~/.grok/sessions/<encoded cwd>/<id>/`.
//! - Antigravity: `~/.gemini/antigravity-cli/{conversations/<id>.db,
//!   brain/<id>, annotations/<id>.pbtxt}`.
//! - OpenCode keeps sessions in its own SQLite database, which BenCode
//!   does not write: not supported.
//!
//! The CLIs' own indexes (Codex's state database, Grok's search index)
//! are left as they are; they skip a session whose files are gone.

use std::path::{Path, PathBuf};

use crate::harness::HarnessKind;
use crate::harness::accounts::{AccountProfile, profiles_on_disk};
use crate::harness::claude::session_files;

/// Whether BenCode can remove `harness`'s own copy of a session.
pub fn supported(harness: &str) -> bool {
    matches!(
        HarnessKind::from_id(harness),
        Some(
            HarnessKind::Claude | HarnessKind::Codex | HarnessKind::Grok | HarnessKind::Antigravity
        )
    )
}

/// The folders `harness` may keep a session in: its default home, then
/// each account profile's. Blocking (lists the profiles on disk).
pub fn homes(harness: &str) -> Vec<PathBuf> {
    homes_by_account(harness)
        .into_iter()
        .map(|(_, dir)| dir)
        .collect()
}

/// [`homes`], each with the account profile it belongs to (`None` for the
/// default home). Blocking.
pub fn homes_by_account(harness: &str) -> Vec<(Option<String>, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let profiles = |provider: &str| {
        profiles_on_disk()
            .into_iter()
            .filter(|(p, _)| p == provider)
            .filter_map(|(_, id)| {
                let dir = AccountProfile::resolve(provider, Some(&id))?.dir;
                Some((Some(id), dir))
            })
            .collect::<Vec<_>>()
    };
    let (default, provider) = match HarnessKind::from_id(harness) {
        Some(HarnessKind::Claude) => (session_files::config_dir(None), "claude"),
        Some(HarnessKind::Codex) => (
            std::env::var_os("CODEX_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .or_else(|| home.map(|h| h.join(".codex"))),
            "codex",
        ),
        Some(HarnessKind::Grok) => {
            return home.map(|h| (None, h.join(".grok"))).into_iter().collect();
        }
        Some(HarnessKind::Antigravity) => {
            return home
                .map(|h| (None, h.join(".gemini/antigravity-cli")))
                .into_iter()
                .collect();
        }
        _ => return Vec::new(),
    };
    default
        .map(|dir| (None, dir))
        .into_iter()
        .chain(profiles(provider))
        .collect()
}

/// Removes everything `harness` keeps of `session_id` under `homes`.
/// Returns how many files or folders went; none found is not an error.
/// Blocking.
pub fn purge(harness: &str, session_id: &str, homes: &[PathBuf]) -> Result<usize, String> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("not a session id: {session_id:?}"));
    }
    let kind = HarnessKind::from_id(harness).filter(|_| supported(harness));
    let Some(kind) = kind else {
        return Err(format!("{harness} sessions cannot be deleted by BenCode"));
    };
    let mut found = Vec::new();
    for home in homes {
        found.extend(session_paths(kind, home, session_id));
    }
    let mut removed = 0;
    for path in found {
        let gone = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match gone {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("{}: {err}", path.display())),
        }
    }
    Ok(removed)
}

/// What `kind` keeps of `id` under one home folder, as it is on disk now.
fn session_paths(kind: HarnessKind, home: &Path, id: &str) -> Vec<PathBuf> {
    let existing = |paths: Vec<PathBuf>| paths.into_iter().filter(|p| p.exists()).collect();
    match kind {
        HarnessKind::Claude => {
            let mut paths = Vec::new();
            for project in subdirs(&home.join("projects")) {
                paths.push(project.join(format!("{id}.jsonl")));
                paths.push(project.join(id));
            }
            paths.push(home.join("session-env").join(id));
            paths.push(home.join("file-history").join(id));
            existing(paths)
        }
        HarnessKind::Codex => {
            let mut paths = Vec::new();
            let marker = format!("-{id}.jsonl");
            for root in ["sessions", "archived_sessions"] {
                rollouts(&home.join(root), &marker, 3, &mut paths);
            }
            paths
        }
        HarnessKind::Grok => existing(
            subdirs(&home.join("sessions"))
                .into_iter()
                .map(|cwd| cwd.join(id))
                .collect(),
        ),
        HarnessKind::Antigravity => existing(vec![
            home.join("conversations").join(format!("{id}.db")),
            home.join("brain").join(id),
            home.join("annotations").join(format!("{id}.pbtxt")),
        ]),
        HarnessKind::OpenCode => Vec::new(),
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

/// Codex rollouts naming the session, `depth` folders deep at most
/// (`YYYY/MM/DD`); archived ones may be compressed (`.jsonl.zst`).
fn rollouts(dir: &Path, marker: &str, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 {
                rollouts(&entry.path(), marker, depth - 1, found);
            }
        } else if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("rollout-") && name.contains(marker))
        {
            found.push(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-session-purge-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}\n").unwrap();
    }

    #[test]
    fn claude_loses_the_session_in_every_profile_and_keeps_the_others() {
        let root = scratch("claude");
        let (a, b) = (root.join("a"), root.join("b"));
        touch(&a.join("projects/-Users-me-app/s-1.jsonl"));
        touch(&a.join("projects/-Users-me-app/s-1/subagents/agent-a.jsonl"));
        touch(&a.join("projects/-Users-me-app/s-2.jsonl"));
        touch(&a.join("session-env/s-1/env"));
        touch(&b.join("projects/-Users-me-wt/s-1.jsonl"));

        assert_eq!(purge("claude", "s-1", &[a.clone(), b.clone()]), Ok(4));
        assert!(!a.join("projects/-Users-me-app/s-1.jsonl").exists());
        assert!(!a.join("projects/-Users-me-app/s-1").exists());
        assert!(!a.join("session-env/s-1").exists());
        assert!(!b.join("projects/-Users-me-wt/s-1.jsonl").exists());
        assert!(a.join("projects/-Users-me-app/s-2.jsonl").is_file());
        // Gone already: nothing to do, not an error.
        assert_eq!(purge("claude", "s-1", &[a, b]), Ok(0));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn codex_finds_dated_and_archived_rollouts() {
        let root = scratch("codex");
        let id = "019f-aa";
        touch(&root.join(format!(
            "sessions/2026/08/03/rollout-2026-08-03T15-32-19-{id}.jsonl"
        )));
        touch(&root.join(format!(
            "archived_sessions/rollout-2026-07-01T00-00-00-{id}.jsonl.zst"
        )));
        touch(&root.join("sessions/2026/08/03/rollout-2026-08-03T15-32-19-019f-bb.jsonl"));

        assert_eq!(purge("codex", id, std::slice::from_ref(&root)), Ok(2));
        assert!(
            root.join("sessions/2026/08/03/rollout-2026-08-03T15-32-19-019f-bb.jsonl")
                .is_file()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn grok_and_antigravity_lose_their_session_folders() {
        let root = scratch("grok-agy");
        let grok = root.join("grok");
        touch(&grok.join("sessions/%2FUsers%2Fme/g-1/summary.json"));
        touch(&grok.join("sessions/%2FUsers%2Fme/g-2/summary.json"));
        assert_eq!(purge("grok", "g-1", std::slice::from_ref(&grok)), Ok(1));
        assert!(grok.join("sessions/%2FUsers%2Fme/g-2").is_dir());

        let agy = root.join("agy");
        touch(&agy.join("conversations/c-1.db"));
        touch(&agy.join("brain/c-1/scratch"));
        touch(&agy.join("annotations/c-1.pbtxt"));
        touch(&agy.join("presence/c-1.lock"));
        assert_eq!(purge("agy", "c-1", std::slice::from_ref(&agy)), Ok(3));
        assert!(agy.join("presence/c-1.lock").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_bad_id_or_an_unsupported_harness_is_an_error() {
        let root = scratch("bad");
        touch(&root.join("projects/p/x.jsonl"));
        assert!(purge("claude", "../x", std::slice::from_ref(&root)).is_err());
        assert!(purge("claude", "", std::slice::from_ref(&root)).is_err());
        assert!(purge("opencode", "ses_1", std::slice::from_ref(&root)).is_err());
        assert!(!supported("opencode"));
        assert!(root.join("projects/p/x.jsonl").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
