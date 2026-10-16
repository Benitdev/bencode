//! Claude keeps a session's transcript in its config folder:
//! `projects/<cwd slug>/<session id>.jsonl`, with its subagents' beside it
//! in `<session id>/`. Each account profile is its own config folder, so a
//! thread moved to another account finds nothing to resume there. Copying
//! those files over lets the new account resume the same session, whole
//! (BenCode's own; MonoCode starts a new session briefed on a recap).

use std::path::{Path, PathBuf};

use crate::harness::accounts::AccountProfile;

/// The config folder Claude runs with for `profile`: the profile's own, or
/// for the default account what the CLI would pick by itself.
pub fn config_dir(profile: Option<&AccountProfile>) -> Option<PathBuf> {
    if let Some(profile) = profile {
        return Some(profile.dir.clone());
    }
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".claude"))
}

/// Copies session `session_id` from the config folder `from` into `to`,
/// under the same project folder, replacing an older copy. Blocking.
pub fn copy_session(from: &Path, to: &Path, session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("not a Claude session id: {session_id:?}"));
    }
    if from == to {
        return Ok(());
    }
    let file = format!("{session_id}.jsonl");
    let project = find_project(&from.join("projects"), &file)
        .ok_or_else(|| format!("session {session_id} not found in {}", from.display()))?;
    let name = project
        .file_name()
        .ok_or_else(|| format!("no project folder name in {}", project.display()))?;
    let target = to.join("projects").join(name);
    let failed = |what: &Path, err: std::io::Error| format!("{}: {err}", what.display());
    std::fs::create_dir_all(&target).map_err(|err| failed(&target, err))?;
    // Written beside it first, so a CLI never reads half a transcript.
    let partial = target.join(format!(".{file}.partial"));
    std::fs::copy(project.join(&file), &partial).map_err(|err| failed(&partial, err))?;
    std::fs::rename(&partial, target.join(&file)).map_err(|err| failed(&target, err))?;
    let extras = project.join(session_id);
    if extras.is_dir() {
        copy_dir(&extras, &target.join(session_id))?;
    }
    Ok(())
}

fn find_project(projects: &Path, file: &str) -> Option<PathBuf> {
    std::fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|dir| dir.join(file).is_file())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    let failed = |what: &Path, err: std::io::Error| format!("{}: {err}", what.display());
    std::fs::create_dir_all(to).map_err(|err| failed(to, err))?;
    for entry in std::fs::read_dir(from).map_err(|err| failed(from, err))?.flatten() {
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(|err| failed(&source, err))?;
        if kind.is_dir() {
            copy_dir(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target).map_err(|err| failed(&source, err))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-claude-session-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn copies_the_transcript_and_its_subagents_under_the_same_project() {
        let root = scratch("copy");
        let (from, to) = (root.join("a"), root.join("b"));
        let project = from.join("projects/-Users-me-app");
        std::fs::create_dir_all(project.join("s-1/subagents")).unwrap();
        std::fs::write(project.join("s-1.jsonl"), "{\"x\":1}\n").unwrap();
        std::fs::write(project.join("s-1/subagents/agent-a.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("other.jsonl"), "{}\n").unwrap();

        copy_session(&from, &to, "s-1").unwrap();
        let copied = to.join("projects/-Users-me-app");
        assert_eq!(
            std::fs::read_to_string(copied.join("s-1.jsonl")).unwrap(),
            "{\"x\":1}\n"
        );
        assert!(copied.join("s-1/subagents/agent-a.jsonl").is_file());
        assert!(!copied.join("other.jsonl").exists());
        assert!(!copied.join(".s-1.jsonl.partial").exists());

        // Moving back replaces the older copy with the newer transcript.
        std::fs::write(copied.join("s-1.jsonl"), "{\"x\":2}\n").unwrap();
        copy_session(&to, &from, "s-1").unwrap();
        assert_eq!(
            std::fs::read_to_string(project.join("s-1.jsonl")).unwrap(),
            "{\"x\":2}\n"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_session_or_a_bad_id_is_an_error() {
        let root = scratch("missing");
        std::fs::create_dir_all(root.join("a/projects/p")).unwrap();
        assert!(copy_session(&root.join("a"), &root.join("b"), "nope").is_err());
        assert!(copy_session(&root.join("a"), &root.join("b"), "../x").is_err());
        assert!(!root.join("b").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
