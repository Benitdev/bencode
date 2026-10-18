//! What Search finds: threads, project files and recent projects, ranked,
//! with the text that matched marked. Pure, so it is tested without the app.
//!
//! Threads match on words, not fuzzily: every word of the query must be in
//! the title, in the thread's project / branch / model, or somewhere in
//! what was said (user and assistant text, not tool output). A title hit
//! ranks above a place hit, which ranks above a transcript hit; newer first
//! within each. Files rank like Go to File (`quick_open::rank_files`).

use std::ops::Range;

use gpui::SharedString;

use crate::db::SessionRow;
use crate::ui::quick_open::{fuzzy_match, rank_files};

/// How many of each kind All shows; Tab moves to the kind to see them all.
const ALL_THREADS: usize = 6;
const ALL_FILES: usize = 5;
const ALL_PROJECTS: usize = 3;
const THREAD_LIMIT: usize = 200;
const FILE_LIMIT: usize = 60;
/// With no query: what was used last.
const RECENT_THREADS: usize = 8;
const RECENT_THREADS_SCOPED: usize = 50;
const RECENT_FILES: usize = 5;
const RECENT_PROJECTS: usize = 5;
/// Characters kept ahead of a transcript match, and the excerpt's length.
const SNIPPET_LEAD: usize = 32;
const SNIPPET_MAX: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    All,
    Conversations,
    Files,
    Projects,
}

impl SearchScope {
    /// The kinds a hit can be, in the order All lists them.
    pub const KINDS: [SearchScope; 3] = [
        SearchScope::Conversations,
        SearchScope::Files,
        SearchScope::Projects,
    ];

    pub fn includes(self, other: SearchScope) -> bool {
        self == SearchScope::All || self == other
    }

    fn slot(self) -> usize {
        match self {
            SearchScope::Files => 1,
            SearchScope::Projects => 2,
            SearchScope::Conversations | SearchScope::All => 0,
        }
    }
}

/// Text with the byte ranges to mark, sorted and apart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marked {
    pub text: SharedString,
    pub marks: Vec<Range<usize>>,
}

impl Marked {
    fn new(text: impl Into<SharedString>, marks: Vec<Range<usize>>) -> Self {
        Self {
            text: text.into(),
            marks: merge(marks),
        }
    }

    fn plain(text: impl Into<SharedString>) -> Self {
        Self::new(text, Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HitKind {
    Thread { harness: String, updated_at: i64 },
    File { changed: bool },
    Project { threads: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub kind: HitKind,
    /// A session id, a path relative to the project, or a project folder.
    pub target_id: String,
    pub title: Marked,
    pub detail: Marked,
}

impl SearchHit {
    pub fn scope(&self) -> SearchScope {
        match self.kind {
            HitKind::Thread { .. } => SearchScope::Conversations,
            HitKind::File { .. } => SearchScope::Files,
            HitKind::Project { .. } => SearchScope::Projects,
        }
    }
}

/// What a search looks through.
pub struct Sources<'a> {
    pub sessions: &'a [SessionRow],
    /// The open project's files, relative to it.
    pub files: &'a [SharedString],
    /// Files picked lately, newest first.
    pub recent_files: &'a [String],
    pub projects: &'a [String],
    /// Whether a file has uncommitted changes.
    pub changed: &'a dyn Fn(&str) -> bool,
    pub home: &'a str,
}

#[derive(Debug, Default)]
pub struct Found {
    /// In list order: threads, then files, then projects.
    pub hits: Vec<SearchHit>,
    /// How many of each kind matched, before All cut them short.
    pub totals: [usize; 3],
}

impl Found {
    pub fn total(&self, scope: SearchScope) -> usize {
        match scope {
            SearchScope::All => self.totals.iter().sum(),
            kind => self.totals[kind.slot()],
        }
    }
}

/// Everything in `scope` matching `query`; with no query, what was used last.
pub fn find(query: &str, scope: SearchScope, src: &Sources) -> Found {
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let all = scope == SearchScope::All;
    let mut found = Found::default();
    for kind in SearchScope::KINDS {
        if !scope.includes(kind) {
            continue;
        }
        let (hits, cap) = match kind {
            SearchScope::Conversations => (
                thread_hits(&tokens, all, src),
                if all { ALL_THREADS } else { THREAD_LIMIT },
            ),
            SearchScope::Files => (
                file_hits(query.trim(), src),
                if all { ALL_FILES } else { FILE_LIMIT },
            ),
            _ => (
                project_hits(query.trim(), &tokens, src),
                if all { ALL_PROJECTS } else { usize::MAX },
            ),
        };
        found.totals[kind.slot()] = hits.len();
        found.hits.extend(hits.into_iter().take(cap));
    }
    found
}

fn thread_hits(tokens: &[String], all: bool, src: &Sources) -> Vec<SearchHit> {
    if tokens.is_empty() {
        let mut recent: Vec<&SessionRow> = src.sessions.iter().filter(|s| !s.archived).collect();
        recent.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        let cap = if all {
            RECENT_THREADS
        } else {
            RECENT_THREADS_SCOPED
        };
        return recent
            .into_iter()
            .take(cap)
            .map(|s| thread_hit(s, Marked::plain(s.title.clone()), place(s)))
            .collect();
    }
    let mut ranked: Vec<(u8, i64, SearchHit)> = src
        .sessions
        .iter()
        .filter_map(|s| {
            let (tier, hit) = match_thread(tokens, s)?;
            Some((tier, s.updated_at, hit))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    ranked.into_iter().map(|(.., hit)| hit).collect()
}

/// A thread's rank (lower is better) and its hit, if every token is in
/// its title, its place, its model or what was said.
fn match_thread(tokens: &[String], s: &SessionRow) -> Option<(u8, SearchHit)> {
    if let Some(marks) = find_all(&s.title, tokens) {
        let tier = if marks[0].start == 0 {
            0
        } else if marks.iter().all(|m| word_start(&s.title, m.start)) {
            1
        } else {
            2
        };
        let hit = thread_hit(s, Marked::new(s.title.clone(), marks), place(s));
        return Some((tier, hit));
    }
    let said = || {
        s.blocks
            .iter()
            .filter(|b| matches!(b.role.as_str(), "user" | "assistant"))
            .filter_map(|b| b.text.as_deref())
    };
    let where_ = place(s);
    let (mut title_marks, mut place_marks) = (Vec::new(), Vec::new());
    // The first token found only in what was said: the excerpt shows it.
    let mut unsaid: Option<&str> = None;
    for token in tokens {
        if let Some(at) = find_folded(&s.title, token) {
            title_marks.push(at);
        } else if let Some(at) = find_folded(&where_.text, token) {
            place_marks.push(at);
        } else if contains_folded(&s.model, token) {
        } else if said().any(|text| contains_folded(text, token)) {
            unsaid.get_or_insert(token);
        } else {
            return None;
        }
    }
    let title = Marked::new(s.title.clone(), title_marks);
    let Some(token) = unsaid else {
        let where_ = Marked::new(where_.text, place_marks);
        return Some((3, thread_hit(s, title, where_)));
    };
    let excerpt = said().find_map(|text| Some(snippet(text, find_folded(text, token)?)))?;
    Some((4, thread_hit(s, title, excerpt)))
}

fn thread_hit(s: &SessionRow, title: Marked, detail: Marked) -> SearchHit {
    SearchHit {
        kind: HitKind::Thread {
            harness: s.harness.clone(),
            updated_at: s.updated_at,
        },
        target_id: s.id.clone(),
        title,
        detail,
    }
}

/// "project · branch".
fn place(s: &SessionRow) -> Marked {
    let mut text = folder_name(&s.cwd).to_string();
    if let Some(branch) = s.branch.as_deref().filter(|b| !b.is_empty()) {
        text.push_str(" · ");
        text.push_str(branch);
    }
    Marked::plain(text)
}

fn file_hits(query: &str, src: &Sources) -> Vec<SearchHit> {
    let mut ranked = rank_files(src.files, query, src.recent_files);
    if query.is_empty() {
        ranked.truncate(RECENT_FILES);
    }
    ranked
        .into_iter()
        .map(|(path, hit)| {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            let name_at = path.len() - name.len();
            let marks = char_marks(&path, &hit.positions);
            let shift = |at: Range<usize>, by: usize| at.start - by..at.end - by;
            let name_marks = marks
                .iter()
                .filter(|m| m.start >= name_at)
                .map(|m| shift(m.clone(), name_at))
                .collect();
            let dir_marks = marks
                .iter()
                .filter(|m| m.end <= dir.len())
                .cloned()
                .collect();
            SearchHit {
                kind: HitKind::File {
                    changed: (src.changed)(&path),
                },
                title: Marked::new(name.to_string(), name_marks),
                detail: Marked::new(dir.to_string(), dir_marks),
                target_id: path,
            }
        })
        .collect()
}

fn project_hits(query: &str, tokens: &[String], src: &Sources) -> Vec<SearchHit> {
    let hit = |path: &String, title: Marked, detail: Marked| SearchHit {
        kind: HitKind::Project {
            threads: src.sessions.iter().filter(|s| s.cwd == *path).count(),
        },
        target_id: path.clone(),
        title,
        detail,
    };
    if tokens.is_empty() {
        return src
            .projects
            .iter()
            .take(RECENT_PROJECTS)
            .map(|p| {
                let title = Marked::plain(folder_name(p).to_string());
                hit(p, title, Marked::plain(tildify(p, src.home)))
            })
            .collect();
    }
    // A fuzzy hit on the folder's name first, then the words anywhere in its path.
    let mut ranked: Vec<(i64, usize, SearchHit)> = src
        .projects
        .iter()
        .enumerate()
        .filter_map(|(recency, p)| {
            let name = folder_name(p);
            let shown = tildify(p, src.home);
            if let Some(found) = fuzzy_match(query, name) {
                let title = Marked::new(name.to_string(), char_marks(name, &found.positions));
                return Some((
                    found.score + 1000,
                    recency,
                    hit(p, title, Marked::plain(shown)),
                ));
            }
            let marks = find_all(&shown, tokens)?;
            let title = Marked::plain(name.to_string());
            Some((0, recency, hit(p, title, Marked::new(shown, marks))))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(.., hit)| hit).collect()
}

/// The line around `at` in `text`, from a little before it, its whitespace
/// folded, with the match marked.
fn snippet(text: &str, at: Range<usize>) -> Marked {
    let line_start = text[..at.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[at.end..].find('\n').map_or(text.len(), |i| at.end + i);
    let from = text[line_start..at.start]
        .char_indices()
        .rev()
        .nth(SNIPPET_LEAD - 1)
        .map_or(line_start, |(i, _)| line_start + i);
    let mut out = String::new();
    if from > line_start {
        out.push('…');
    }
    let (mut mark_start, mut mark_end) = (0, None);
    let mut blank = false;
    for (chars, (i, c)) in text[from..line_end].char_indices().enumerate() {
        let i = from + i;
        if i == at.start {
            mark_start = out.len();
        }
        if i == at.end {
            mark_end = Some(out.len());
        }
        if chars >= SNIPPET_MAX && i >= at.end {
            out.push('…');
            break;
        }
        if c.is_whitespace() {
            if !blank && !out.is_empty() {
                out.push(' ');
            }
            blank = true;
        } else {
            out.push(c);
            blank = false;
        }
    }
    let out = out.trim_end().to_string();
    let mark = mark_start..mark_end.unwrap_or(out.len()).min(out.len());
    Marked::new(out, vec![mark])
}

/// Where each token first is in `text`, or `None` if one is missing.
fn find_all(text: &str, tokens: &[String]) -> Option<Vec<Range<usize>>> {
    tokens.iter().map(|t| find_folded(text, t)).collect()
}

/// The first place `needle` (already lowercase) is in `haystack`, ignoring
/// case, as a byte range of `haystack`. Every keystroke runs this over each
/// transcript, so it lowercases as it compares instead of copying the text.
fn find_folded(haystack: &str, needle: &str) -> Option<Range<usize>> {
    if needle.is_empty() {
        return Some(0..0);
    }
    if haystack.is_ascii() && needle.is_ascii() {
        return haystack
            .as_bytes()
            .windows(needle.len())
            .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
            .map(|at| at..at + needle.len());
    }
    'start: for (start, _) in haystack.char_indices() {
        let mut want = needle.chars();
        for (i, c) in haystack[start..].char_indices() {
            for lower in c.to_lowercase() {
                match want.next() {
                    None => return Some(start..start + i + c.len_utf8()),
                    Some(w) if w == lower => {}
                    Some(_) => continue 'start,
                }
            }
            if want.as_str().is_empty() {
                return Some(start..start + i + c.len_utf8());
            }
        }
        return None;
    }
    None
}

fn contains_folded(haystack: &str, needle: &str) -> bool {
    find_folded(haystack, needle).is_some()
}

fn word_start(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric())
}

/// Character `positions` in `text` as byte ranges.
fn char_marks(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    text.char_indices()
        .enumerate()
        .filter(|(ix, _)| positions.contains(ix))
        .map(|(_, (at, c))| at..at + c.len_utf8())
        .collect()
}

/// Sorted, with touching and overlapping ranges joined.
fn merge(mut marks: Vec<Range<usize>>) -> Vec<Range<usize>> {
    marks.retain(|m| m.start < m.end);
    marks.sort_by_key(|m| m.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(marks.len());
    for m in marks {
        match out.last_mut() {
            Some(last) if m.start <= last.end => last.end = last.end.max(m.end),
            _ => out.push(m),
        }
    }
    out
}

fn folder_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(trimmed)
}

/// `path` with the home folder written `~`.
fn tildify(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Block;

    fn session(id: &str, title: &str, updated_at: i64) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: title.into(),
            cwd: "/home/me/bencode".into(),
            updated_at,
            ..Default::default()
        }
    }

    fn said(role: &str, text: &str) -> Block {
        Block {
            role: role.into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    fn sources<'a>(sessions: &'a [SessionRow], files: &'a [SharedString]) -> Sources<'a> {
        Sources {
            sessions,
            files,
            recent_files: &[],
            projects: &[],
            changed: &|_| false,
            home: "/home/me",
        }
    }

    fn ids(found: &Found) -> Vec<&str> {
        found.hits.iter().map(|h| h.target_id.as_str()).collect()
    }

    #[test]
    fn find_folded_matches_like_lowercasing() {
        for (haystack, needle) in [
            ("Fix Parser", "parser"),
            ("Fix Parser", "x p"),
            ("Fix Parser", "parsers"),
            ("ab", "abc"),
            ("Tiếng VIỆT", "việt"),
            ("Tiếng VIỆT", "tiếng v"),
            ("ÉCOLE", "école"),
            ("ÉCOLE", "ecole"),
            ("", ""),
        ] {
            assert_eq!(
                contains_folded(haystack, needle),
                haystack.to_lowercase().contains(needle),
                "{haystack:?} / {needle:?}"
            );
        }
        assert_eq!(find_folded("Tiếng VIỆT", "việt"), Some(8..14));
    }

    #[test]
    fn title_hits_rank_by_place_then_recency() {
        let sessions = [
            session("inside", "a parser fix", 30),
            session("old-start", "Parser bug", 10),
            session("new-start", "parser crash", 20),
            session("mid-word", "subparser", 40),
        ];
        let found = find(
            "parser",
            SearchScope::Conversations,
            &sources(&sessions, &[]),
        );
        assert_eq!(
            ids(&found),
            ["new-start", "old-start", "inside", "mid-word"]
        );
        assert_eq!(found.hits[2].title.marks, [2..8]);
    }

    #[test]
    fn transcript_hits_come_last_with_an_excerpt() {
        let mut talked = session("talked", "Untitled", 99);
        talked.blocks = vec![
            said("tool", "jq output"),
            said("user", "first line\nplease   run jq on the file"),
        ];
        let sessions = [talked, session("titled", "jq tips", 1)];
        let found = find("JQ", SearchScope::All, &sources(&sessions, &[]));
        assert_eq!(ids(&found), ["titled", "talked"]);
        let excerpt = &found.hits[1].detail;
        assert_eq!(excerpt.text.as_ref(), "please run jq on the file");
        assert_eq!(&excerpt.text[excerpt.marks[0].clone()], "jq");
    }

    #[test]
    fn every_word_must_match_somewhere() {
        let mut s = session("a", "Fix the parser", 1);
        s.blocks = vec![said("assistant", "the lexer was wrong")];
        let sessions = [s];
        let src = sources(&sessions, &[]);
        assert_eq!(find("parser lexer", SearchScope::All, &src).hits.len(), 1);
        assert!(
            find("parser tokens", SearchScope::All, &src)
                .hits
                .is_empty()
        );
        assert_eq!(find("bencode", SearchScope::All, &src).hits.len(), 1);
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let text = format!("{} needle {}", "x".repeat(100), "y".repeat(400));
        let at = find_folded(&text, "needle").unwrap();
        let excerpt = snippet(&text, at);
        assert!(excerpt.text.starts_with('…') && excerpt.text.ends_with('…'));
        assert_eq!(&excerpt.text[excerpt.marks[0].clone()], "needle");
        assert!(excerpt.text.chars().count() <= SNIPPET_MAX + 2);
    }

    #[test]
    fn all_caps_each_kind_but_counts_them() {
        let sessions: Vec<SessionRow> = (0..10)
            .map(|i| session(&format!("s{i}"), &format!("mod {i}"), i))
            .collect();
        let files: Vec<SharedString> = (0..9).map(|i| format!("src/mod{i}.rs").into()).collect();
        let src = sources(&sessions, &files);
        let all = find("mod", SearchScope::All, &src);
        assert_eq!(all.hits.len(), ALL_THREADS + ALL_FILES);
        assert_eq!(all.totals, [10, 9, 0]);
        assert_eq!(all.total(SearchScope::All), 19);
        let file = all
            .hits
            .iter()
            .find(|h| h.scope() == SearchScope::Files)
            .unwrap();
        assert_eq!(file.title.text.as_ref(), "mod0.rs");
        assert_eq!(file.title.marks, [0..3]);
        assert_eq!(file.detail.text.as_ref(), "src");
        assert_eq!(find("mod", SearchScope::Files, &src).hits.len(), 9);
    }

    #[test]
    fn no_query_lists_recent_threads_and_projects() {
        let mut archived = session("archived", "old", 50);
        archived.archived = true;
        let sessions = [session("a", "one", 1), session("b", "two", 2), archived];
        let projects = vec!["/home/me/bencode".to_string()];
        let src = Sources {
            projects: &projects,
            ..sources(&sessions, &[])
        };
        let found = find("  ", SearchScope::All, &src);
        assert_eq!(ids(&found), ["b", "a", "/home/me/bencode"]);
        assert_eq!(found.hits[2].detail.text.as_ref(), "~/bencode");
        assert_eq!(found.hits[2].kind, HitKind::Project { threads: 3 });
    }

    #[test]
    fn projects_match_their_name_first() {
        let projects = vec!["/work/bencode".to_string(), "/bench/other".to_string()];
        let src = Sources {
            projects: &projects,
            ..sources(&[], &[])
        };
        let found = find("ben", SearchScope::Projects, &src);
        assert_eq!(ids(&found), ["/work/bencode", "/bench/other"]);
        assert_eq!(found.hits[0].title.marks, [0..3]);
        assert_eq!(found.hits[1].detail.marks, [1..4]);
    }

    #[test]
    fn marks_are_merged_and_paths_tildified() {
        assert_eq!(merge(vec![4..6, 0..2, 1..3, 3..4]), [0..6]);
        assert_eq!(tildify("/home/me/x", "/home/me"), "~/x");
        assert_eq!(tildify("/home/meow", "/home/me"), "/home/meow");
        assert_eq!(folder_name("/a/b/"), "b");
    }
}
