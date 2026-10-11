//! Diffs of one side of a change and of past commits. MonoCode shows the
//! staged side as HEAD↔index and the unstaged side as index↔work tree, and
//! opens a commit as its file list plus per-file diffs (`CommitDiff.tsx`).

use std::path::Path;

use anyhow::{Result, bail};

use super::{
    DiffLineKind, GitFileChange, GitFileStatus, MAX_UNTRACKED_READ_BYTES, is_in_index,
    parse_numstat_z, parse_unified_diff, run_git, run_git_string,
};

/// Which change a diff describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// HEAD↔index.
    Staged,
    /// Index↔work tree; untracked files show as all-new.
    Unstaged,
    /// What a commit changed.
    Commit(String),
}

/// One file's diff with every unchanged line as context, so the review can
/// fold and unfold it (MonoCode diffs the whole original and current text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub lines: Vec<DiffLineKind>,
    pub binary: bool,
    pub too_large: bool,
}

/// Patches bigger than this are not shown (MonoCode `tooLarge`).
const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
/// Context wide enough to take in any whole file.
const FULL_CONTEXT: &str = "-U2147483647";

/// Rejects anything but a hex object id, so it can never be read as an option.
pub(super) fn validate_sha(sha: &str) -> Result<()> {
    if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("not a commit id: {sha}");
    }
    Ok(())
}

pub(super) fn patch(text: &str) -> FileDiff {
    if text.len() > MAX_PATCH_BYTES {
        return FileDiff {
            too_large: true,
            ..FileDiff::default()
        };
    }
    let lines = parse_unified_diff(text);
    let binary = lines.is_empty()
        && text
            .lines()
            .any(|l| l.starts_with("Binary files ") || l == "GIT binary patch");
    FileDiff {
        lines,
        binary,
        too_large: false,
    }
}

/// An untracked file, all of it added.
fn untracked(cwd: &str, path: &str) -> Result<FileDiff> {
    let full = Path::new(cwd).join(path);
    if std::fs::metadata(&full)?.len() > MAX_UNTRACKED_READ_BYTES {
        return Ok(FileDiff {
            too_large: true,
            ..FileDiff::default()
        });
    }
    let text = match String::from_utf8(std::fs::read(&full)?) {
        Ok(text) if !text.contains('\0') => text,
        _ => {
            return Ok(FileDiff {
                binary: true,
                ..FileDiff::default()
            });
        }
    };
    let header = DiffLineKind::Header(format!("@@ -0,0 +1,{} @@", text.lines().count()));
    let lines = std::iter::once(header)
        .chain(text.lines().map(|l| DiffLineKind::Addition(l.to_string())))
        .collect();
    Ok(FileDiff {
        lines,
        ..FileDiff::default()
    })
}

/// Diff of `path` as described by `source`, with full context.
pub fn file_diff(cwd: &str, path: &str, source: &DiffSource) -> Result<FileDiff> {
    const FLAGS: [&str; 3] = ["--no-ext-diff", "--no-color", FULL_CONTEXT];
    match source {
        DiffSource::Staged => {
            let args = [&["diff", "--cached"][..], &FLAGS, &["--", path]].concat();
            Ok(patch(&run_git_string(cwd, &args)?))
        }
        DiffSource::Unstaged => {
            let args = [&["diff"][..], &FLAGS, &["--", path]].concat();
            let diff = patch(&run_git_string(cwd, &args)?);
            if diff.lines.is_empty() && !diff.binary && !is_in_index(cwd, path) {
                untracked(cwd, path)
            } else {
                Ok(diff)
            }
        }
        DiffSource::Commit(sha) => {
            validate_sha(sha)?;
            let head = ["show", "--format=", "-M"];
            let args = [&head[..], &FLAGS, &[sha.as_str(), "--", path]].concat();
            Ok(patch(&run_git_string(cwd, &args)?))
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
    fn binary_and_oversized_patches_are_flagged() {
        let binary = patch("diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n");
        assert!(binary.binary && binary.lines.is_empty());
        let text = patch("@@ -1 +1 @@\n-a\n+b\n");
        assert!(!text.binary);
        assert_eq!(text.lines.len(), 3);
        assert!(patch(&"x".repeat(MAX_PATCH_BYTES + 1)).too_large);
    }

    #[test]
    fn sha_must_be_hex() {
        assert!(validate_sha("abc123").is_ok());
        assert!(validate_sha("--output=x").is_err());
        assert!(validate_sha("").is_err());
    }
}
