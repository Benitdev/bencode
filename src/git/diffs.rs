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
    let too_large = FileDiff {
        too_large: true,
        ..FileDiff::default()
    };
    let bytes = match full.to_str().filter(|p| crate::remote::is_remote(p)) {
        // On a machine: one read, capped one byte past the limit.
        Some(remote) => {
            let file = crate::remote::files::read(remote, MAX_UNTRACKED_READ_BYTES + 1)?;
            if file.truncated {
                return Ok(too_large);
            }
            file.bytes.0
        }
        None => {
            if std::fs::metadata(&full)?.len() > MAX_UNTRACKED_READ_BYTES {
                return Ok(too_large);
            }
            std::fs::read(&full)?
        }
    };
    let text = match String::from_utf8(bytes) {
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

/// One file of a patch that covers several (`gh pr diff`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    /// Where a renamed file came from.
    pub old_path: Option<String>,
    pub diff: FileDiff,
}

/// A path as a patch header writes it: `a/` or `b/` dropped, git's quotes
/// for unusual names taken off.
fn header_path(raw: &str) -> Option<String> {
    let raw = raw.trim_end_matches('\t').trim();
    if raw == "/dev/null" {
        return None;
    }
    let raw = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);
    let path = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    Some(path.to_string())
}

/// Splits a multi-file patch at each `diff --git`, naming each file by its
/// new path (its old one when deleted) and reading its lines as `patch`.
pub fn split_patch(text: &str) -> Vec<PatchFile> {
    struct Open<'a> {
        path: String,
        old_path: Option<String>,
        from: Option<String>,
        lines: Vec<&'a str>,
        in_hunk: bool,
    }
    fn close(open: Open<'_>) -> PatchFile {
        let path = if open.path.is_empty() {
            open.from.clone().unwrap_or_default()
        } else {
            open.path
        };
        PatchFile {
            path,
            old_path: open.old_path,
            diff: patch(&open.lines.join("\n")),
        }
    }
    let mut files = Vec::new();
    let mut open: Option<Open> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            files.extend(open.take().map(close));
            // `a/x b/x`: the new path is the second half, as long as the
            // two halves name the same file (they do unless renamed, and a
            // rename names both again below).
            let path = rest
                .rfind(" b/")
                .and_then(|at| header_path(&rest[at + 1..]))
                .unwrap_or_default();
            open = Some(Open {
                path,
                old_path: None,
                from: None,
                lines: vec![line],
                in_hunk: false,
            });
            continue;
        }
        let Some(file) = open.as_mut() else {
            continue;
        };
        file.lines.push(line);
        if file.in_hunk {
            continue;
        }
        if line.starts_with("@@") {
            file.in_hunk = true;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            file.old_path = Some(from.to_string());
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.path = to.to_string();
        } else if let Some(from) = line.strip_prefix("--- ") {
            file.from = header_path(from);
        } else if let Some(to) = line.strip_prefix("+++ ") {
            match header_path(to) {
                Some(to) => file.path = to,
                // Deleted: named by the side it had.
                None => file.path = file.from.clone().unwrap_or_default(),
            }
        }
    }
    files.extend(open.map(close));
    files
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
    fn a_pull_request_patch_splits_by_file() {
        let text = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
-old
+new
--- not a header
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
";
        let files = split_patch(text);
        let names: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["src/a.rs", "new.rs", "gone.txt", "logo.png"]);
        // The hunk's own `---` line stays content.
        assert_eq!(files[0].diff.lines.len(), 4);
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));
        assert!(files[1].diff.lines.is_empty() && !files[1].diff.binary);
        assert_eq!(files[2].diff.lines.len(), 2);
        assert!(files[3].diff.binary);
    }

    #[test]
    fn sha_must_be_hex() {
        assert!(validate_sha("abc123").is_ok());
        assert!(validate_sha("--output=x").is_err());
        assert!(validate_sha("").is_err());
    }
}
