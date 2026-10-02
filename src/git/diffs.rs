//! Diffs of one side of a change and of past commits. MonoCode shows the
//! staged side as HEAD↔index and the unstaged side as index↔work tree, and
//! opens a commit as its file list plus per-file diffs (`CommitDiff.tsx`).

use anyhow::{Result, bail};

use super::{
    DiffLineKind, GitFileChange, GitFileStatus, get_file_diff, get_untracked_diff, is_in_index,
    parse_numstat_z, parse_unified_diff, run_git, run_git_string,
};

/// Which change a diff describes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DiffSource {
    /// HEAD↔work tree, both sides together (the Changes view's file list).
    #[default]
    WorkingTree,
    /// HEAD↔index.
    Staged,
    /// Index↔work tree; untracked files show as all-new.
    Unstaged,
    /// What a commit changed.
    Commit(String),
}

/// Rejects anything but a hex object id, so it can never be read as an option.
fn validate_sha(sha: &str) -> Result<()> {
    if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("not a commit id: {sha}");
    }
    Ok(())
}

fn unified(cwd: &str, args: &[&str]) -> Vec<DiffLineKind> {
    match run_git_string(cwd, args) {
        Ok(text) => parse_unified_diff(&text),
        Err(err) => {
            log::warn!("git {} failed: {err:#}", args.join(" "));
            Vec::new()
        }
    }
}

/// Diff of `path` as described by `source`.
pub fn diff_for(cwd: &str, path: &str, source: &DiffSource) -> Vec<DiffLineKind> {
    const FLAGS: [&str; 3] = ["--no-ext-diff", "--no-color", "-U3"];
    match source {
        DiffSource::WorkingTree => get_file_diff(cwd, path),
        DiffSource::Staged => {
            let args = [&["diff", "--cached"][..], &FLAGS, &["--", path]].concat();
            unified(cwd, &args)
        }
        DiffSource::Unstaged => {
            let args = [&["diff"][..], &FLAGS, &["--", path]].concat();
            let rows = unified(cwd, &args);
            if rows.is_empty() && !is_in_index(cwd, path) {
                get_untracked_diff(cwd, path)
            } else {
                rows
            }
        }
        DiffSource::Commit(sha) => {
            if let Err(err) = validate_sha(sha) {
                log::warn!("{err:#}");
                return Vec::new();
            }
            let head = ["show", "--format=", "-M"];
            let args = [&head[..], &FLAGS, &[sha.as_str(), "--", path]].concat();
            unified(cwd, &args)
        }
    }
}

/// Parses `git diff-tree --name-status -z`: `S\0path\0`, or for renames and
/// copies `R100\0old\0new\0`. Keyed by the new path.
fn parse_name_status_z(raw: &[u8]) -> Vec<(GitFileStatus, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut out = Vec::new();
    while let Some(code) = fields.next() {
        let status = match code.chars().next() {
            Some('A') => GitFileStatus::Added,
            Some('D') => GitFileStatus::Deleted,
            Some('R' | 'C') => GitFileStatus::Renamed,
            _ => GitFileStatus::Modified,
        };
        if status == GitFileStatus::Renamed {
            fields.next(); // old path
        }
        if let Some(path) = fields.next() {
            out.push((status, path.to_string()));
        }
    }
    out
}

/// Files a commit changed, with line counts. The root commit diffs against
/// the empty tree.
pub fn commit_files(cwd: &str, sha: &str) -> Result<Vec<GitFileChange>> {
    validate_sha(sha)?;
    let base = ["diff-tree", "--no-commit-id", "-r", "-z", "--root", "-M"];
    let names = run_git(cwd, &[&base[..], &["--name-status", sha]].concat())?;
    let stats = parse_numstat_z(&run_git(cwd, &[&base[..], &["--numstat", sha]].concat())?);
    Ok(parse_name_status_z(&names)
        .into_iter()
        .map(|(status, path)| {
            let (additions, deletions) = stats.get(&path).copied().unwrap_or((0, 0));
            GitFileChange {
                path,
                status,
                additions,
                deletions,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_reads_renames_by_new_path() {
        let raw = b"M\0src/a.rs\0R100\0old.rs\0new.rs\0A\0b.rs\0";
        let parsed = parse_name_status_z(raw);
        assert_eq!(
            parsed,
            vec![
                (GitFileStatus::Modified, "src/a.rs".to_string()),
                (GitFileStatus::Renamed, "new.rs".to_string()),
                (GitFileStatus::Added, "b.rs".to_string()),
            ]
        );
    }

    #[test]
    fn sha_must_be_hex() {
        assert!(validate_sha("abc123").is_ok());
        assert!(validate_sha("--output=x").is_err());
        assert!(validate_sha("").is_err());
    }
}
