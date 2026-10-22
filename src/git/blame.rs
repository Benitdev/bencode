//! Who last changed each line of a file (`git blame --porcelain`), what
//! that commit said and changed there (`git show`), and the file's own
//! history (`git log --follow`): the editor's line blame and File History.

use std::collections::HashMap;

use anyhow::Result;

use super::diffs::validate_sha;
use super::run_git_string;

/// The sha `git blame` gives lines not committed yet.
const UNCOMMITTED: &str = "0000000000000000000000000000000000000000";
/// How many commits File History lists.
const HISTORY_LIMIT: usize = 500;
/// How many lines of a commit's change the blame card shows.
const HUNK_LINES: usize = 14;

/// A commit as blame names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub author_mail: String,
    /// Epoch seconds.
    pub time: i64,
    pub summary: String,
    /// The file's path in this commit.
    pub path: String,
}

impl BlameCommit {
    pub fn is_uncommitted(&self) -> bool {
        self.sha == UNCOMMITTED
    }

    pub fn short_sha(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
}

/// A file's blame: for each line (from zero), its commit and its line
/// (from one) in that commit's copy of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileBlame {
    commits: Vec<BlameCommit>,
    lines: Vec<(usize, usize)>,
}

impl FileBlame {
    /// Line `line`'s (from zero) commit and its line there.
    pub fn line(&self, line: usize) -> Option<(&BlameCommit, usize)> {
        let (commit, orig) = *self.lines.get(line)?;
        Some((self.commits.get(commit)?, orig))
    }
}

/// `git blame` of `path` as last saved.
pub fn blame_file(cwd: &str, path: &str) -> Result<FileBlame> {
    let text = run_git_string(cwd, &["blame", "--porcelain", "--", path])?;
    Ok(parse_porcelain(&text))
}

/// Reads `--porcelain`: a header (`sha orig final [count]`) per line,
/// the commit's fields the first time it appears, then the line after a
/// tab.
fn parse_porcelain(text: &str) -> FileBlame {
    let mut blame = FileBlame::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    // The current line's commit and its line in that commit.
    let mut current: Option<(usize, usize)> = None;
    for row in text.lines() {
        if row.starts_with('\t') {
            if let Some(line) = current.take() {
                blame.lines.push(line);
            }
            continue;
        }
        let Some((commit, _)) = current else {
            let mut parts = row.split(' ');
            let (Some(sha), Some(orig)) = (parts.next(), parts.next()) else {
                continue;
            };
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let at = *index.entry(sha.to_string()).or_insert_with(|| {
                blame.commits.push(BlameCommit {
                    sha: sha.to_string(),
                    ..BlameCommit::default()
                });
                blame.commits.len() - 1
            });
            current = Some((at, orig.parse().unwrap_or(1)));
            continue;
        };
        let (key, value) = row.split_once(' ').unwrap_or((row, ""));
        let commit = &mut blame.commits[commit];
        match key {
            "author" => commit.author = value.to_string(),
            "author-mail" => {
                commit.author_mail = value.trim_matches(['<', '>']).to_string();
            }
            "author-time" => commit.time = value.parse().unwrap_or(0),
            "summary" => commit.summary = value.to_string(),
            "filename" if commit.path.is_empty() => commit.path = value.to_string(),
            _ => {}
        }
    }
    blame
}

/// One line of a commit's change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HunkLine {
    Added(String),
    Removed(String),
}

/// What a commit says, and its change to one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitDetail {
    pub message: String,
    /// Each hunk: its first line on the new side (from one), how many
    /// lines it has there, and its lines.
    hunks: Vec<(usize, usize, Vec<HunkLine>)>,
}

impl CommitDetail {
    /// The commit's change around its line `line` (from one), at most
    /// `HUNK_LINES` long; whether more was cut.
    pub fn hunk_at(&self, line: usize) -> Option<(&[HunkLine], bool)> {
        let (_, _, lines) = self
            .hunks
            .iter()
            .find(|(start, count, _)| (*start..start + (*count).max(1)).contains(&line))?;
        Some((
            &lines[..lines.len().min(HUNK_LINES)],
            lines.len() > HUNK_LINES,
        ))
    }
}

/// `sha`'s message and its change to `path` (the file's path in that
/// commit, from the repository's top, as blame names it), without context
/// lines.
pub fn commit_detail(cwd: &str, sha: &str, path: &str) -> Result<CommitDetail> {
    validate_sha(sha)?;
    let text = run_git_string(
        cwd,
        &[
            "show",
            "--no-show-signature",
            "--no-color",
            "--no-ext-diff",
            "--format=%B%x00",
            "-U0",
            sha,
            "--",
            &format!(":(top){path}"),
        ],
    )?;
    Ok(parse_detail(&text))
}

fn parse_detail(text: &str) -> CommitDetail {
    let (message, patch) = text.split_once('\0').unwrap_or((text, ""));
    let mut hunks: Vec<(usize, usize, Vec<HunkLine>)> = Vec::new();
    for row in patch.lines() {
        if let Some(header) = row.strip_prefix("@@ ") {
            // `@@ -a,b +c,d @@`: the new side's start and count.
            let new = header
                .split(' ')
                .find_map(|part| part.strip_prefix('+'))
                .unwrap_or("0");
            let (start, count) = new.split_once(',').unwrap_or((new, "1"));
            hunks.push((
                start.parse().unwrap_or(0),
                count.parse().unwrap_or(1),
                Vec::new(),
            ));
            continue;
        }
        // The card shows `HUNK_LINES` and whether more were cut: a commit
        // that wrote the whole file keeps no more than that.
        let Some((_, _, lines)) = hunks.last_mut().filter(|(_, _, l)| l.len() <= HUNK_LINES) else {
            continue;
        };
        if let Some(added) = row.strip_prefix('+') {
            lines.push(HunkLine::Added(added.to_string()));
        } else if let Some(removed) = row.strip_prefix('-') {
            lines.push(HunkLine::Removed(removed.to_string()));
        }
    }
    CommitDetail {
        message: message.trim().to_string(),
        hunks,
    }
}

/// A commit that changed a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub author_mail: String,
    /// Epoch seconds.
    pub time: i64,
    pub subject: String,
}

/// The commits that changed `path`, newest first, across renames.
pub fn file_history(cwd: &str, path: &str) -> Result<Vec<FileCommit>> {
    let limit = HISTORY_LIMIT.to_string();
    let text = run_git_string(
        cwd,
        &[
            "log",
            "--follow",
            "--no-show-signature",
            "--max-count",
            &limit,
            "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s%x1e",
            "--",
            path,
        ],
    )?;
    Ok(parse_history(&text))
}

fn parse_history(text: &str) -> Vec<FileCommit> {
    text.split('\u{1e}')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let mut fields = record.split('\0');
            Some(FileCommit {
                sha: fields.next().filter(|s| !s.is_empty())?.to_string(),
                short_sha: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                author_mail: fields.next()?.to_string(),
                time: fields.next()?.parse().unwrap_or(0),
                subject: fields.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// The repository a folder is in: its top (a path as the folder is named,
/// so one on a machine stays `remote://…`) and the email it commits as,
/// to call its lines "You".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    pub root: String,
    pub email: Option<String>,
}

pub fn repo_of(dir: &str) -> Repo {
    let root = match run_git_string(dir, &["rev-parse", "--show-cdup"]) {
        Ok(up) => climb(dir, up.trim()),
        Err(err) => {
            log::debug!("no repository above {dir}: {err:#}");
            dir.to_string()
        }
    };
    let email = run_git_string(dir, &["config", "user.email"])
        .ok()
        .map(|email| email.trim().to_string())
        .filter(|email| !email.is_empty());
    Repo { root, email }
}

/// `dir` with `up` (`../../`, from `--show-cdup`) taken off its end.
fn climb(dir: &str, up: &str) -> String {
    let steps = up.split('/').filter(|step| *step == "..").count();
    let mut root = dir.trim_end_matches('/');
    for _ in 0..steps {
        root = root.rsplit_once('/').map_or(root, |(parent, _)| parent);
    }
    root.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "43ac07a1111111111111111111111111111111aa";

    #[test]
    fn porcelain_names_each_lines_commit() {
        let text = format!(
            "{A} 3 1 2\nauthor Ben\nauthor-mail <ben@x.io>\nauthor-time 1790000000\n\
             summary docs: spec\nprevious 1234 old.md\nfilename spec.md\n\tfirst\n\
             {A} 4 2\nfilename spec.md\n\tsecond\n\
             {UNCOMMITTED} 3 3 1\nauthor Not Committed Yet\nauthor-mail <not.committed.yet>\n\
             author-time 1790000500\nsummary Version of spec.md from spec.md\nfilename spec.md\n\tthird\n"
        );
        let blame = parse_porcelain(&text);
        let (commit, orig) = blame.line(1).unwrap();
        assert_eq!((commit.short_sha(), orig), ("43ac07a", 4));
        assert_eq!(commit.author_mail, "ben@x.io");
        assert_eq!(commit.summary, "docs: spec");
        assert_eq!(commit.path, "spec.md");
        assert!(blame.line(2).unwrap().0.is_uncommitted());
        assert!(blame.line(3).is_none());
    }

    #[test]
    fn detail_keeps_the_message_and_finds_the_lines_hunk() {
        let text = "docs: spec\n\nWhy.\n\0\ndiff --git a/s.md b/s.md\n--- a/s.md\n+++ b/s.md\n\
                    @@ -3 +3,2 @@\n-old\n+new\n+more\n@@ -9,0 +11 @@ ctx\n+tail\n";
        let detail = parse_detail(text);
        assert_eq!(detail.message, "docs: spec\n\nWhy.");
        let (lines, cut) = detail.hunk_at(4).unwrap();
        assert!(!cut);
        assert_eq!(
            lines,
            [
                HunkLine::Removed("old".into()),
                HunkLine::Added("new".into()),
                HunkLine::Added("more".into())
            ]
        );
        assert_eq!(
            detail.hunk_at(11).unwrap().0,
            [HunkLine::Added("tail".into())]
        );
        assert!(detail.hunk_at(7).is_none());
    }

    #[test]
    fn detail_keeps_only_what_the_card_shows() {
        let added: String = (0..500).map(|n| format!("+line {n}\n")).collect();
        let text = format!("feat: a file\n\0\n@@ -0,0 +1,500 @@\n{added}");
        let detail = parse_detail(&text);
        let (lines, cut) = detail.hunk_at(250).unwrap();
        assert_eq!(lines.len(), HUNK_LINES);
        assert!(cut);
        assert_eq!(detail.hunks[0].2.len(), HUNK_LINES + 1);
    }

    #[test]
    fn roots_climb_from_the_folder() {
        assert_eq!(climb("/r/src/app", "../../"), "/r");
        assert_eq!(climb("/r", ""), "/r");
        assert_eq!(climb("remote://box/r/src", "../"), "remote://box/r");
    }

    #[test]
    fn history_reads_each_record() {
        let text = format!("{A}\x0043ac07a\x00Ben\x00ben@x.io\x001790000000\x00docs: spec\x1e\n");
        let history = parse_history(&text);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].short_sha, "43ac07a");
        assert_eq!(history[0].time, 1_790_000_000);
        assert_eq!(history[0].subject, "docs: spec");
    }
}
