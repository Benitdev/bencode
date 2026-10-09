//! Search in files (MonoCode `src-tauri/src/search.rs`): `git grep` over the
//! project, else a scan of its files, with match case, whole word, regex and
//! include / exclude globs. Blocking: run it off the UI thread.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

/// MonoCode `MAX_MATCHES`.
const MAX_MATCHES: usize = 500;
/// MonoCode `MAX_FILE_BYTES`: larger files are skipped by the scan.
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// MonoCode `MAX_GIT_GREP_BYTES`: enough for 500 ordinary previews.
const MAX_GIT_GREP_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Comma-separated globs.
    pub include: String,
    pub exclude: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    /// Relative to the searched folder, with `/`.
    pub relative: String,
    /// From one.
    pub line: u32,
    /// From one, in bytes; 1 when a regex matched.
    pub column: u32,
    pub preview: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    /// More matched than were kept.
    pub truncated: bool,
}

/// Searches `root`; a set `cancel` ends it early with no matches.
pub fn search(
    root: &Path,
    options: &SearchOptions,
    cancel: &AtomicBool,
) -> Result<SearchResult, String> {
    let query = options.query.trim();
    if query.is_empty() || cancel.load(Ordering::Acquire) {
        return Ok(SearchResult::default());
    }
    if !root.is_dir() {
        return Err(format!("{}: Not a directory", root.display()));
    }
    match git_grep(root, options, query, MAX_GIT_GREP_BYTES, cancel) {
        Some(result) => Ok(result),
        None => Ok(scan_files(root, options, query, cancel)),
    }
}

/// `None` when git cannot search here (not a repository), so the scan runs.
fn git_grep(
    root: &Path,
    options: &SearchOptions,
    query: &str,
    max_bytes: usize,
    cancel: &AtomicBool,
) -> Option<SearchResult> {
    let mut args = vec!["--no-pager", "grep", "-z", "-n"];
    if !options.case_sensitive {
        args.push("-i");
    }
    if options.whole_word {
        args.push("-w");
    }
    args.push(if options.regex { "-E" } else { "-F" });
    args.extend(["-e", query]);
    // An include glob starting with `-` is a pathspec, not a flag.
    args.push("--");
    let specs = pathspecs(&options.include, &options.exclude);
    args.extend(specs.iter().map(String::as_str));

    let (mut raw, mut truncated) = match grep_output(root, &args, max_bytes, cancel) {
        Some(output) => output,
        None if cancel.load(Ordering::Acquire) => return Some(SearchResult::default()),
        None => return None,
    };
    if truncated {
        // The cap can cut a record; keep whole lines only.
        let end = raw.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        raw.truncate(end);
    }
    let mut matches = Vec::new();
    // Each record is `path\0line\0preview\n`.
    for record in raw.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        let mut fields = record.splitn(3, |b| *b == 0);
        let (Some(path), Some(line), Some(preview)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let preview = String::from_utf8_lossy(preview).into_owned();
        let column = if options.regex {
            1
        } else {
            find_on_line(&preview, &needle(query, options.case_sensitive), options).unwrap_or(1)
        };
        matches.push(SearchMatch {
            relative: String::from_utf8_lossy(path).replace('\\', "/"),
            line: std::str::from_utf8(line)
                .ok()
                .and_then(|l| l.parse().ok())
                .unwrap_or(1),
            column,
            preview,
        });
        if matches.len() >= MAX_MATCHES {
            truncated = true;
            break;
        }
    }
    Some(SearchResult { matches, truncated })
}

/// MonoCode `git_output_capped`: stdout up to `max_bytes` (and whether it
/// was cut). `None` when git failed or `cancel` was set; grep's "no match"
/// exit is a success.
fn grep_output(
    root: &Path,
    args: &[&str],
    max_bytes: usize,
    cancel: &AtomicBool,
) -> Option<(Vec<u8>, bool)> {
    let mut child = crate::git::git_command(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let stop = |child: &mut std::process::Child| {
        if let Err(err) = child.kill() {
            log::debug!("stopping git grep: {err}");
        }
        if let Err(err) = child.wait() {
            log::debug!("reaping git grep: {err}");
        }
    };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if cancel.load(Ordering::Acquire) {
            stop(&mut child);
            return None;
        }
        let read = match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => {
                stop(&mut child);
                return None;
            }
        };
        let room = max_bytes.saturating_sub(buf.len());
        if read > room {
            buf.extend_from_slice(&chunk[..room]);
            stop(&mut child);
            return Some((buf, true));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    let status = child.wait().ok()?;
    (status.success() || status.code() == Some(1)).then_some((buf, false))
}

/// MonoCode `scan_files`: outside a repository, a plain-text search of the
/// project's files (no regex).
fn scan_files(
    root: &Path,
    options: &SearchOptions,
    query: &str,
    cancel: &AtomicBool,
) -> SearchResult {
    if options.regex {
        return SearchResult::default();
    }
    let include = glob_tokens(&options.include);
    let exclude = glob_tokens(&options.exclude);
    let needle = needle(query, options.case_sensitive);
    let mut matches = Vec::new();
    for relative in crate::workspace::list_project_files(root) {
        if cancel.load(Ordering::Acquire) {
            return SearchResult::default();
        }
        if !matches_pathspec(&relative, &include, &exclude) {
            continue;
        }
        let path = root.join(&relative);
        let small =
            std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES);
        let Some(content) = small
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .filter(|bytes| !bytes.contains(&0))
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let Some(column) = find_on_line(line, &needle, options) else {
                continue;
            };
            matches.push(SearchMatch {
                relative: relative.clone(),
                line: index as u32 + 1,
                column,
                preview: line.to_string(),
            });
            if matches.len() >= MAX_MATCHES {
                return SearchResult {
                    matches,
                    truncated: true,
                };
            }
        }
    }
    SearchResult {
        matches,
        truncated: false,
    }
}

fn needle(query: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    }
}

/// The first match of `needle` on `line` (column from one), honouring
/// whole word.
fn find_on_line(line: &str, needle: &str, options: &SearchOptions) -> Option<u32> {
    let haystack = needle_case(line, options.case_sensitive);
    // Lowercasing can change byte lengths; then columns are not trusted.
    let same_bytes = haystack.len() == line.len();
    let mut start = 0;
    while let Some(index) = haystack.get(start..)?.find(needle) {
        let column = start + index;
        if options.whole_word && same_bytes && !is_word_boundary(line, column, needle.len()) {
            start = column + haystack[column..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        return Some(if same_bytes { column as u32 + 1 } else { 1 });
    }
    None
}

fn needle_case(text: &str, case_sensitive: bool) -> std::borrow::Cow<'_, str> {
    if case_sensitive {
        text.into()
    } else {
        text.to_lowercase().into()
    }
}

fn is_word_boundary(line: &str, start: usize, len: usize) -> bool {
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let before = line.get(..start).and_then(|s| s.chars().next_back());
    let after = line.get(start + len..).and_then(|s| s.chars().next());
    !before.is_some_and(word) && !after.is_some_and(word)
}

fn glob_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// Git pathspecs for the include globs and `:(exclude)` ones.
fn pathspecs(include: &str, exclude: &str) -> Vec<String> {
    glob_tokens(include)
        .into_iter()
        .chain(
            glob_tokens(exclude)
                .into_iter()
                .map(|glob| format!(":(exclude){glob}")),
        )
        .collect()
}

fn matches_pathspec(relative: &str, include: &[String], exclude: &[String]) -> bool {
    if !include.is_empty() && !include.iter().any(|glob| glob_match(glob, relative)) {
        return false;
    }
    !exclude.iter().any(|glob| glob_match(glob, relative))
}

/// MonoCode `glob_match`: `dir/*`, `*.ext`, `*part*`, or a path prefix.
fn glob_match(glob: &str, path: &str) -> bool {
    let glob = glob.trim_start_matches("./");
    if glob.contains('*') || glob.contains('?') {
        if let Some(prefix) = glob.strip_suffix('*') {
            let prefix = prefix.trim_end_matches('/');
            return path == prefix || path.starts_with(&format!("{prefix}/"));
        }
        if let Some(suffix) = glob.strip_prefix('*') {
            let suffix = suffix.trim_start_matches('/');
            return path.ends_with(suffix) || path.contains(suffix);
        }
        return path.contains(glob.trim_matches('*'));
    }
    path == glob || path.starts_with(&format!("{glob}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.0) {
                eprintln!("removing {}: {err}", self.0.display());
            }
        }
    }

    fn tmp(label: &str) -> Tmp {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "bencode-search-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = crate::git::git_command(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn options(query: &str) -> SearchOptions {
        SearchOptions {
            query: query.into(),
            case_sensitive: true,
            ..Default::default()
        }
    }

    fn live() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn git_grep_finds_lines_and_columns() {
        let dir = tmp("grep");
        git(&dir.0, &["init", "-q"]);
        std::fs::create_dir(dir.0.join("src")).unwrap();
        std::fs::write(dir.0.join("src/app.ts"), "let a = 1;\nconst needle = 2;\n").unwrap();
        std::fs::write(dir.0.join("notes.md"), "a Needle here\n").unwrap();
        git(&dir.0, &["add", "."]);

        let found = search(&dir.0, &options("needle"), &live()).unwrap();
        assert_eq!(
            found.matches,
            [SearchMatch {
                relative: "src/app.ts".into(),
                line: 2,
                column: 7,
                preview: "const needle = 2;".into(),
            }]
        );

        let any_case = SearchOptions {
            case_sensitive: false,
            ..options("needle")
        };
        assert_eq!(search(&dir.0, &any_case, &live()).unwrap().matches.len(), 2);

        let only_md = SearchOptions {
            include: "*.md".into(),
            ..any_case.clone()
        };
        let found = search(&dir.0, &only_md, &live()).unwrap();
        assert_eq!(found.matches.len(), 1);
        assert_eq!(
            (found.matches[0].relative.as_str(), found.matches[0].column),
            ("notes.md", 3)
        );

        let not_src = SearchOptions {
            exclude: "src".into(),
            ..any_case
        };
        assert_eq!(
            search(&dir.0, &not_src, &live()).unwrap().matches[0].relative,
            "notes.md"
        );
    }

    #[test]
    fn a_folder_outside_git_is_scanned() {
        let dir = tmp("scan");
        std::fs::write(dir.0.join("app.ts"), "const needle = 1;\nneedles\n").unwrap();
        let whole = SearchOptions {
            whole_word: true,
            ..options("needle")
        };
        let found = search(&dir.0, &whole, &live()).unwrap();
        assert_eq!(found.matches.len(), 1);
        assert_eq!((found.matches[0].line, found.matches[0].column), (1, 7));
        // The scan has no regex engine.
        let regex = SearchOptions {
            regex: true,
            ..options("need.e")
        };
        assert!(search(&dir.0, &regex, &live()).unwrap().matches.is_empty());
    }

    #[test]
    fn a_cancelled_search_returns_nothing() {
        let dir = tmp("cancel");
        std::fs::write(dir.0.join("app.ts"), "needle\n").unwrap();
        let found = search(&dir.0, &options("needle"), &AtomicBool::new(true)).unwrap();
        assert_eq!(found, SearchResult::default());
    }

    #[test]
    fn a_capped_grep_keeps_whole_records() {
        let dir = tmp("cap");
        git(&dir.0, &["init", "-q"]);
        std::fs::write(
            dir.0.join("a.txt"),
            "needle one\nneedle two\nneedle three\n",
        )
        .unwrap();
        git(&dir.0, &["add", "."]);
        // Room for one record and part of the next.
        let found = git_grep(&dir.0, &options("needle"), "needle", 30, &live()).unwrap();
        assert!(found.truncated);
        assert_eq!(found.matches.len(), 1);
        assert_eq!(found.matches[0].preview, "needle one");
    }

    #[test]
    fn globs_follow_monocode() {
        assert!(glob_match("src/*", "src/a/b.rs"));
        assert!(glob_match("*.rs", "src/a.rs"));
        assert!(glob_match("src", "src/a.rs"));
        assert!(!glob_match("src", "srcs/a.rs"));
        assert_eq!(pathspecs("a, b", "c"), ["a", "b", ":(exclude)c"]);
    }
}
