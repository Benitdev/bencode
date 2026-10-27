//! MonoCode `session_changes` / `unpushed` (`monocode-core/src/git.rs`):
//! the git pill's view of one thread's own files, whether they are staged
//! or still only in the working tree, and the commits a git action's turn
//! made. Other threads' and the user's changes in the same checkout are
//! never counted.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::diffs::validate_sha;
use super::{git_succeeds, run_git_string};

/// Where a thread's own changes stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionChanges {
    /// Some of them are in the index.
    pub staged: bool,
    /// Some of them, or new files, are only in the working tree.
    pub unstaged: bool,
}

impl SessionChanges {
    pub fn uncommitted(self) -> bool {
        self.staged || self.unstaged
    }
}

/// A commit a git action's turn made, as its card lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCommit {
    pub sha: String,
    pub subject: String,
    #[serde(default)]
    pub files: Vec<String>,
}

/// `path` with its folder resolved (`/tmp` is `/private/tmp` on macOS),
/// but not a final symlink git tracks as a file, nor a deleted file.
fn resolved(path: &Path) -> PathBuf {
    path.parent()
        .and_then(|parent| std::fs::canonicalize(parent).ok())
        .zip(path.file_name())
        .map_or_else(|| path.to_path_buf(), |(parent, name)| parent.join(name))
}

/// `paths` (absolute, or under `cwd`) inside `cwd`'s checkout, as paths
/// from its top level; the top level too.
fn inside_checkout(cwd: &str, paths: &[String]) -> Option<(String, Vec<String>)> {
    let top = run_git_string(cwd, &["rev-parse", "--show-toplevel"]).ok()?;
    let top = top.trim();
    let root = std::fs::canonicalize(top).unwrap_or_else(|_| PathBuf::from(top));
    let inside: Vec<String> = paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| resolved(&Path::new(cwd).join(path)))
        .filter_map(|path| {
            path.strip_prefix(&root)
                .ok()
                .map(|rel| rel.to_string_lossy().into_owned())
        })
        .filter(|rel| !rel.is_empty())
        .collect();
    (!inside.is_empty()).then(|| (root.to_string_lossy().into_owned(), inside))
}

/// Where `paths` stand in `cwd`'s checkout. Nothing is changed when none of
/// them is in it, or `cwd` is not a checkout.
pub fn session_changes(cwd: &str, paths: &[String]) -> SessionChanges {
    if paths.is_empty() {
        return SessionChanges::default();
    }
    let Some((root, inside)) = inside_checkout(cwd, paths) else {
        return SessionChanges::default();
    };
    let mut args = vec![
        "--no-optional-locks",
        "--literal-pathspecs",
        "status",
        "--porcelain",
        "--untracked-files=all",
        "--",
    ];
    args.extend(inside.iter().map(String::as_str));
    match run_git_string(&root, &args) {
        Ok(out) => parse_porcelain(&out),
        Err(err) => {
            log::warn!("could not read a thread's git changes: {err:#}");
            SessionChanges::default()
        }
    }
}

/// `git status --porcelain` lines: an index column, then a tree column.
fn parse_porcelain(out: &str) -> SessionChanges {
    let mut changes = SessionChanges::default();
    for line in out.lines() {
        let mut columns = line.chars();
        let (Some(index), Some(tree)) = (columns.next(), columns.next()) else {
            continue;
        };
        changes.staged |= !matches!(index, ' ' | '?');
        changes.unstaged |= tree != ' ';
    }
    changes
}

/// The commit `cwd` has checked out, if any.
pub fn head(cwd: &str) -> Option<String> {
    run_git_string(cwd, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok()
        .map(|sha| sha.trim().to_string())
        .filter(|sha| !sha.is_empty())
}

/// The commits on HEAD since `base`, oldest first, with their files.
pub fn commits_since(cwd: &str, base: &str) -> Result<Vec<SessionCommit>> {
    validate_sha(base)?;
    let range = format!("{base}..HEAD");
    let out = run_git_string(
        cwd,
        &[
            "log",
            "--reverse",
            "--format=%x1e%H%x1f%s",
            "--name-only",
            &range,
            "--",
        ],
    )?;
    Ok(parse_log(&out))
}

fn parse_log(out: &str) -> Vec<SessionCommit> {
    out.split('\u{1e}')
        .filter_map(|record| {
            let mut lines = record.lines();
            let (sha, subject) = lines.next()?.split_once('\u{1f}')?;
            Some(SessionCommit {
                sha: sha.trim().to_string(),
                subject: subject.trim().to_string(),
                files: lines
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

/// `sha` is on the checked-out branch and no remote branch holds it yet.
pub fn unpushed(cwd: &str, sha: &str) -> bool {
    if validate_sha(sha).is_err() {
        return false;
    }
    git_succeeds(cwd, &["symbolic-ref", "-q", "HEAD"])
        && git_succeeds(cwd, &["merge-base", "--is-ancestor", sha, "HEAD"])
        && run_git_string(
            cwd,
            &["--no-optional-locks", "branch", "-r", "--contains", sha],
        )
        .is_ok_and(|out| out.trim().is_empty())
}

/// The branch's upstream (`origin/main`), once it has one.
pub fn upstream(cwd: &str) -> Option<String> {
    run_git_string(
        cwd,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .map(|name| name.trim().to_string())
    .filter(|name| !name.is_empty())
}

/// The web page of `origin`'s repository on GitHub.
pub fn github_web(cwd: &str) -> Option<String> {
    let url = run_git_string(cwd, &["remote", "get-url", "origin"]).ok()?;
    github_web_url(url.trim())
}

/// `https://github.com/owner/name` for a github.com remote, from HTTPS or
/// SSH addresses.
fn github_web_url(remote: &str) -> Option<String> {
    let path = [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
    ]
    .iter()
    .find_map(|prefix| remote.strip_prefix(prefix))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = path.split_once('/')?;
    let ok = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    };
    (ok(owner) && ok(name)).then(|| format!("https://github.com/{owner}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_columns_tell_staged_from_unstaged() {
        assert_eq!(parse_porcelain(""), SessionChanges::default());
        assert_eq!(
            parse_porcelain("M  a.rs\n"),
            SessionChanges {
                staged: true,
                unstaged: false
            }
        );
        assert_eq!(
            parse_porcelain(" M a.rs\n?? b.rs\n"),
            SessionChanges {
                staged: false,
                unstaged: true
            }
        );
        assert_eq!(
            parse_porcelain("MM a.rs\n"),
            SessionChanges {
                staged: true,
                unstaged: true
            }
        );
    }

    #[test]
    fn log_records_carry_their_files() {
        let out = "\u{1e}aaa\u{1f}Fix the sidebar\n\nsrc/a.rs\nsrc/b.rs\n\u{1e}bbb\u{1f}Docs\n\nREADME.md\n";
        let commits = parse_log(out);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "aaa");
        assert_eq!(commits[0].subject, "Fix the sidebar");
        assert_eq!(commits[0].files, ["src/a.rs", "src/b.rs"]);
        assert_eq!(commits[1].files, ["README.md"]);
    }

    #[test]
    fn github_remotes_have_a_web_page() {
        for remote in [
            "https://github.com/o/r.git",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r",
            "https://github.com/o/r/",
        ] {
            assert_eq!(
                github_web_url(remote).as_deref(),
                Some("https://github.com/o/r"),
                "{remote}"
            );
        }
        assert_eq!(github_web_url("https://gitlab.com/o/r.git"), None);
        assert_eq!(github_web_url("git@github.com:o/r;x"), None);
    }

    struct Repo(PathBuf);

    impl Repo {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "bencode-session-git-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&root).unwrap();
            let repo = Self(root.canonicalize().unwrap());
            repo.git(&["init", "-q"]);
            for name in ["own.rs", "other.rs"] {
                std::fs::write(repo.0.join(name), "base\n").unwrap();
            }
            repo.git(&["add", "."]);
            repo.git(&["commit", "-q", "-m", "Base"]);
            repo
        }

        fn cwd(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }

        fn git(&self, args: &[&str]) {
            let out = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(&self.0)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove the test repository: {err}");
            }
        }
    }

    #[test]
    fn only_the_threads_own_files_count() {
        let repo = Repo::new();
        let own = vec![repo.0.join("own.rs").to_string_lossy().into_owned()];
        std::fs::write(repo.0.join("other.rs"), "another thread\n").unwrap();
        assert!(!session_changes(&repo.cwd(), &own).uncommitted());

        std::fs::write(repo.0.join("own.rs"), "this thread\n").unwrap();
        assert_eq!(
            session_changes(&repo.cwd(), &own),
            SessionChanges {
                staged: false,
                unstaged: true
            }
        );
        // Relative paths are read from the thread's folder.
        assert!(session_changes(&repo.cwd(), &["own.rs".into()]).unstaged);
        repo.git(&["add", "own.rs"]);
        assert_eq!(
            session_changes(&repo.cwd(), &own),
            SessionChanges {
                staged: true,
                unstaged: false
            }
        );

        let base = head(&repo.cwd()).unwrap();
        repo.git(&["commit", "-q", "-m", "Own change", "--", "own.rs"]);
        assert!(!session_changes(&repo.cwd(), &own).uncommitted());
        let commits = commits_since(&repo.cwd(), &base).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "Own change");
        assert_eq!(commits[0].files, ["own.rs"]);
        // No remote holds it.
        assert!(unpushed(&repo.cwd(), &commits[0].sha));
        assert!(!unpushed(&repo.cwd(), "--all"));
        assert!(commits_since(&repo.cwd(), "HEAD~1; rm").is_err());
    }
}
