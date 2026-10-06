//! One `git status` read shared by a workspace refresh: the state
//! fingerprint, the staged/unstaged split and the combined changes all come
//! from the same `--porcelain=v2 -b` output, which also carries the branch,
//! its upstream counts and whether `HEAD` exists.

use std::hash::{Hash, Hasher};
use std::path::Path;

use super::{
    DEFAULT_BRANCH, GitDetailedStatus, GitFileChange, GitFileStatus, StatusEntry, combined_status,
    current_branch, diff_numstat, empty_tree, index_side_status, lossy, make_change, run_git,
    worktree_side_status,
};

/// `git status --porcelain=v2 -b -z -uall`, parsed once.
pub struct StatusPass {
    raw: Vec<u8>,
    entries: Vec<StatusEntry>,
    branch: BranchHeader,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BranchHeader {
    /// `# branch.head`: `None` when detached.
    head: Option<String>,
    /// `# branch.oid` was `(initial)`: no commit yet.
    unborn: bool,
    /// `# branch.ab +ahead -behind`, present only with an upstream.
    ahead: usize,
    behind: usize,
}

impl StatusPass {
    /// `None` outside a repository or for a missing folder.
    pub fn read(cwd: &str) -> Option<Self> {
        if !Path::new(cwd).exists() {
            return None;
        }
        let raw = match run_git(cwd, &["status", "--porcelain=v2", "-b", "-z", "-uall"]) {
            Ok(raw) => raw,
            Err(err) => {
                // Expected for folders that are not git repositories.
                log::debug!("no git status for {cwd}: {err:#}");
                return None;
            }
        };
        let (branch, entries) = parse_porcelain_v2(&raw);
        Some(Self {
            raw,
            entries,
            branch,
        })
    }

    /// See `super::state_fingerprint`.
    pub fn fingerprint(&self, cwd: &str) -> u64 {
        let lines = run_git(cwd, &["diff", "--no-ext-diff", "--shortstat"]).unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.raw.hash(&mut hasher);
        lines.hash(&mut hasher);
        hasher.finish()
    }

    /// `HEAD`, or the empty tree in a repository with no commits yet.
    fn diff_base(&self, cwd: &str) -> String {
        if self.branch.unborn {
            empty_tree(cwd)
        } else {
            "HEAD".to_string()
        }
    }

    fn branch_name(&self, cwd: &str) -> String {
        match &self.branch.head {
            Some(name) if !name.is_empty() => name.clone(),
            // Detached: the short hash, as `current_branch` shows it.
            _ if self.branch.unborn => DEFAULT_BRANCH.to_string(),
            _ => current_branch(cwd),
        }
    }

    fn visible(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| !e.is_ignored())
    }

    /// The Changes panel's combined HEAD → work tree list.
    pub fn changes(&self, cwd: &str) -> Vec<GitFileChange> {
        let stats = diff_numstat(cwd, &[self.diff_base(cwd).as_str()]);
        self.visible()
            .map(|e| make_change(&e.path, combined_status(e), cwd, &stats))
            .collect()
    }

    /// Branch, upstream counts and the staged / unstaged split.
    pub fn detailed_status(&self, cwd: &str) -> GitDetailedStatus {
        let base = self.diff_base(cwd);
        let staged_stats = diff_numstat(cwd, &["--cached", base.as_str()]);
        let unstaged_stats = diff_numstat(cwd, &[]);
        let staged = self
            .visible()
            .filter(|e| !e.is_unmerged())
            .filter_map(|e| {
                index_side_status(e.index).map(|s| make_change(&e.path, s, cwd, &staged_stats))
            })
            .collect();
        let unstaged = self
            .visible()
            .filter_map(|e| {
                let status = if e.is_unmerged() {
                    Some(GitFileStatus::Modified)
                } else {
                    worktree_side_status(e.worktree)
                };
                status.map(|s| make_change(&e.path, s, cwd, &unstaged_stats))
            })
            .collect();
        GitDetailedStatus {
            branch: self.branch_name(cwd),
            ahead: self.branch.ahead,
            behind: self.branch.behind,
            staged,
            unstaged,
        }
    }
}

/// What a workspace refresh reads from `git status`.
#[derive(Default)]
pub struct LocalState {
    pub fingerprint: Option<u64>,
    pub status: GitDetailedStatus,
    pub changes: Vec<GitFileChange>,
}

/// One status read, then its line counts side by side (each is its own
/// `git diff` process).
pub fn read_local_state(cwd: &str) -> LocalState {
    let Some(pass) = StatusPass::read(cwd) else {
        return LocalState {
            status: super::get_detailed_status(cwd),
            ..Default::default()
        };
    };
    std::thread::scope(|scope| {
        let fingerprint = scope.spawn(|| pass.fingerprint(cwd));
        let status = scope.spawn(|| pass.detailed_status(cwd));
        let changes = pass.changes(cwd);
        LocalState {
            fingerprint: fingerprint.join().ok(),
            status: status.join().unwrap_or_else(|_| {
                log::error!("reading git status for {cwd} panicked");
                GitDetailedStatus::default()
            }),
            changes,
        }
    })
}

/// v2's `.` (unchanged) is v1's space, so the v1 helpers apply unchanged.
fn status_char(byte: u8) -> char {
    if byte == b'.' { ' ' } else { byte as char }
}

/// Parses `git status --porcelain=v2 -b -z`. Ordinary entries are
/// `1 XY sub mH mI mW hH hI path`, renames `2 XY … Xscore path\0orig`,
/// conflicts `u XY sub m1 m2 m3 mW h1 h2 h3 path`, then `? path`, `! path`.
fn parse_porcelain_v2(raw: &[u8]) -> (BranchHeader, Vec<StatusEntry>) {
    let mut branch = BranchHeader::default();
    let mut entries = Vec::new();
    let mut records = raw.split(|b| *b == 0);
    while let Some(rec) = records.next() {
        let Some((&kind, rest)) = rec.split_first() else {
            continue;
        };
        match kind {
            b'#' => read_header(&lossy(rest), &mut branch),
            b'?' | b'!' => {
                let mark = kind as char;
                if let Some(path) = rest.strip_prefix(b" ") {
                    entries.push(StatusEntry {
                        index: mark,
                        worktree: mark,
                        path: lossy(path),
                        orig_path: None,
                    });
                }
            }
            b'1' | b'2' | b'u' => {
                // Tokens before the path: 8 for `1`, 9 for `2`, 10 for `u`.
                let fields = match kind {
                    b'1' => 8,
                    b'2' => 9,
                    _ => 10,
                };
                let mut parts = rec.splitn(fields + 1, |b| *b == b' ');
                let xy = parts.nth(1).unwrap_or_default();
                let Some(path) = parts.nth(fields - 2) else {
                    continue;
                };
                if xy.len() != 2 {
                    continue;
                }
                let orig_path = (kind == b'2').then(|| records.next().map(lossy)).flatten();
                entries.push(StatusEntry {
                    index: status_char(xy[0]),
                    worktree: status_char(xy[1]),
                    path: lossy(path),
                    orig_path,
                });
            }
            _ => {}
        }
    }
    (branch, entries)
}

fn read_header(line: &str, branch: &mut BranchHeader) {
    let line = line.trim_start();
    if let Some(oid) = line.strip_prefix("branch.oid ") {
        branch.unborn = oid == "(initial)";
    } else if let Some(head) = line.strip_prefix("branch.head ") {
        branch.head = (head != "(detached)").then(|| head.to_string());
    } else if let Some(ab) = line.strip_prefix("branch.ab ") {
        let mut counts = ab.split_whitespace();
        let mut count = |sign: char| {
            counts
                .next()
                .and_then(|c| c.strip_prefix(sign))
                .and_then(|c| c.parse().ok())
                .unwrap_or(0)
        };
        branch.ahead = count('+');
        branch.behind = count('-');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_header_and_every_entry_kind() {
        let raw = b"# branch.oid 0123abcd\0# branch.head main\0# branch.upstream origin/main\0\
# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaaa bbbb mod file.txt\0\
2 R. N... 100644 100644 100644 aaaa bbbb R100 new name.txt\0old name.txt\0\
u UU N... 100644 100644 100644 100644 aaaa bbbb cccc both.txt\0\
? new.txt\0! ignored.log\0";

        let (branch, entries) = parse_porcelain_v2(raw);

        assert_eq!(
            branch,
            BranchHeader {
                head: Some("main".into()),
                unborn: false,
                ahead: 2,
                behind: 1,
            }
        );
        let summary: Vec<(char, char, &str, Option<&str>)> = entries
            .iter()
            .map(|e| (e.index, e.worktree, e.path.as_str(), e.orig_path.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                (' ', 'M', "mod file.txt", None),
                ('R', ' ', "new name.txt", Some("old name.txt")),
                ('U', 'U', "both.txt", None),
                ('?', '?', "new.txt", None),
                ('!', '!', "ignored.log", None),
            ]
        );
        assert!(entries[2].is_unmerged());
        assert!(entries[3].is_untracked());
        assert!(entries[4].is_ignored());
    }

    #[test]
    fn detached_and_unborn_heads() {
        let (detached, _) = parse_porcelain_v2(b"# branch.oid 0123\0# branch.head (detached)\0");
        assert_eq!(detached.head, None);
        assert!(!detached.unborn);

        let (unborn, _) = parse_porcelain_v2(b"# branch.oid (initial)\0# branch.head main\0");
        assert!(unborn.unborn);
        assert_eq!((unborn.ahead, unborn.behind), (0, 0));
    }
}
