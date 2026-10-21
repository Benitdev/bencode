//! Go to Definition without a language server: the name at the caret,
//! searched in the project's files of the same language family as a line
//! that declares it (`fn name`, `def name`, `class Name`, `func (r *T)
//! Name`, `const name =`, `int name(`, `#define NAME`, …), then each match
//! checked and ranked. A guess, not an answer: names in several files come
//! back as several candidates for the user to pick from.

use std::ops::Range;

use crate::search::{SearchMatch, SearchOptions};

/// Words that declare the name after them.
const DECLARERS: &str = "fn|struct|enum|union|trait|type|mod|const|static|let|var|val|def|class|\
                         interface|func|function|module|namespace|record|object|typedef|protocol|\
                         extension|actor|impl|alias|data|newtype|defmodule|defp|defmacro|sub|\
                         macro_rules!|package";

/// A line's first word that makes it a use, never a declaration.
const USES: &[&str] = &[
    "return", "if", "else", "while", "for", "switch", "case", "await", "yield", "throw", "new",
    "print", "echo", "assert", "raise", "not", "and", "or", "do", "in", "is", "elif", "when",
    "match", "puts", "delete", "typeof", "void", "go", "defer",
];

/// How far the best candidate must rank above the next to open alone
/// (a body over its prototype, this file over another).
const CLEAR_LEAD: i32 = 2;

/// A candidate definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Relative to the searched folder, with `/`.
    pub relative: String,
    /// From one.
    pub line: u32,
    /// From one, in bytes.
    pub column: u32,
    /// The name's bytes.
    pub len: usize,
    /// The line, trimmed.
    pub preview: String,
    pub score: i32,
}

fn is_name_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

/// The name around byte `offset`: letters, digits, `_` and `$`, not
/// starting with a digit.
pub fn name_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_name_char(*ch))
        .last()
        .map_or(offset, |(at, _)| at);
    let end = text[offset..]
        .char_indices()
        .find(|(_, ch)| !is_name_char(*ch))
        .map_or(text.len(), |(at, _)| offset + at);
    let name = &text[start..end];
    let first = name.chars().next()?;
    (!first.is_ascii_digit()).then_some(start..end)
}

/// The globs for the files that share `path`'s language family; empty
/// (every file) for a language not listed.
fn family_globs(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("rs") => "*.rs",
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte") => {
            "*.ts,*.tsx,*.js,*.jsx,*.mjs,*.cjs,*.mts,*.cts,*.vue,*.svelte"
        }
        Some("py" | "pyi") => "*.py,*.pyi",
        Some("go") => "*.go",
        Some("c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "m" | "mm") => {
            "*.c,*.h,*.cc,*.cpp,*.cxx,*.hpp,*.hh,*.hxx,*.m,*.mm"
        }
        Some("swift") => "*.swift",
        Some("java" | "kt" | "kts") => "*.java,*.kt,*.kts",
        Some("cs") => "*.cs",
        Some("rb") => "*.rb",
        Some("php") => "*.php",
        Some("lua") => "*.lua",
        Some("dart") => "*.dart",
        Some("zig") => "*.zig",
        Some("ex" | "exs") => "*.ex,*.exs",
        Some("scala") => "*.scala",
        _ => "",
    }
}

/// The search for lines that may declare `name` (an identifier), in the
/// files of `from`'s language family, untracked ones included.
pub fn search_options(name: &str, from: &str) -> SearchOptions {
    // In a POSIX extended regex only `$` of a name's characters is special.
    let n = name.replace('$', "\\$");
    let end = "([^[:alnum:]_$]|$)";
    let s = "[[:space:]]";
    let patterns = [
        // `fn name`, `pub(crate) async fn name`, `class Name`, `def name`.
        format!("(^|[^[:alnum:]_$])({DECLARERS}){s}+(mut{s}+|async{s}+|\\*{s}*)?{n}{end}"),
        // A Go method: `func (r *T) Name(`.
        format!("func{s}*\\([^)]*\\){s}*{n}{end}"),
        // `name: function`, `name = (a) =>`, `name = async x =>`.
        format!(
            "(^|[^[:alnum:]_$.]){n}{s}*[:=]{s}*(async{s}*)?(function|\\([^)]*\\){s}*=>|[[:alnum:]_$]+{s}*=>)"
        ),
        // A method in a class body: `  async name(args) {`.
        format!(
            "^{s}*((async|static|public|private|protected|override|get|set){s}+)*{n}{s}*(<[^>]*>)?\\([^)]*\\)[^;]*\\{{"
        ),
        // A C-like function or method at the start of a line or after a
        // modifier: `int name(`, `public static void name(`.
        format!("^[[:alpha:]_][[:alnum:]_:<>,*&[:space:]]*[[:space:]*&]{n}{s}*\\("),
        format!(
            "^{s}+(public|private|protected|static|final|abstract|override|virtual|internal|inline|extern)[^=;(]*[[:space:]*&]{n}{s}*\\("
        ),
        format!("#{s}*define{s}+{n}{end}"),
    ];
    SearchOptions {
        query: patterns.join("|"),
        case_sensitive: true,
        whole_word: false,
        regex: true,
        include: family_globs(from).to_string(),
        exclude: "*.min.js,*.map".to_string(),
        untracked: true,
    }
}

/// Where `name` first stands as a whole word in `line`, past any
/// declaring word.
fn find_word(line: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = line[from..].find(name) {
        let at = from + found;
        let before = line[..at].chars().next_back();
        let after = line[at + name.len()..].chars().next();
        if !before.is_some_and(is_name_char) && !after.is_some_and(is_name_char) {
            return Some(at);
        }
        from = at + name.len();
    }
    None
}

fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "/*", "* ", "--", ";"]
        .iter()
        .any(|mark| line.starts_with(mark))
        || line == "*"
        || (line.starts_with('#') && !line[1..].trim_start().starts_with("define"))
}

/// Checks and ranks the search's matches for `name`, best first. `from`
/// and `from_line` (from one) are where the jump began: a match there is
/// the name's own declaration, kept last.
pub fn rank(name: &str, from: &str, from_line: u32, matches: Vec<SearchMatch>) -> Vec<Definition> {
    let from_dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut found: Vec<Definition> = matches
        .into_iter()
        .filter_map(|found| {
            let line = found.preview.trim_end();
            if is_comment(line) {
                return None;
            }
            let first = line
                .trim_start()
                .split(|ch: char| !is_name_char(ch))
                .next()
                .unwrap_or_default();
            if USES.contains(&first) {
                return None;
            }
            let at = find_word(line, name)?;
            let mut score = 10;
            let here = found.relative == from;
            if here {
                score += 3;
            } else if found.relative.rsplit_once('/').map_or("", |(dir, _)| dir) == from_dir {
                score += 2;
            }
            let path = found.relative.to_ascii_lowercase();
            if ["test", "spec", "mock", "fixture", "example"]
                .iter()
                .any(|word| path.contains(word))
            {
                score -= 3;
            }
            // A body beats a prototype or a type declaration file.
            if line.ends_with(';') && line.contains('(') || path.ends_with(".d.ts") {
                score -= 2;
            }
            if here && found.line == from_line {
                score -= 20;
            }
            Some(Definition {
                relative: found.relative,
                line: found.line,
                column: at as u32 + 1,
                len: name.len(),
                preview: line.trim().to_string(),
                score,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.relative.cmp(&b.relative))
            .then(a.line.cmp(&b.line))
    });
    found
}

/// The candidate to open without asking: the only one, or one ranked
/// clearly above the rest. `None` asks the user to pick.
pub fn clear_winner(found: &[Definition]) -> Option<&Definition> {
    match found {
        [only] => Some(only),
        [best, next, ..] if best.score >= next.score + CLEAR_LEAD => Some(best),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(relative: &str, line: u32, preview: &str) -> SearchMatch {
        SearchMatch {
            relative: relative.into(),
            line,
            column: 1,
            preview: preview.into(),
        }
    }

    #[test]
    fn names_are_found_around_the_caret() {
        let text = "let x = foo_bar(1);";
        assert_eq!(&text[name_at(text, 10).unwrap()], "foo_bar");
        assert_eq!(&text[name_at(text, 8).unwrap()], "foo_bar", "at its start");
        assert_eq!(&text[name_at(text, 15).unwrap()], "foo_bar", "just past it");
        assert!(name_at(text, 17).is_none(), "on a digit");
        assert!(name_at("a + b", 2).is_none());
        assert_eq!(&"$el.x"[name_at("$el.x", 1).unwrap()], "$el");
    }

    #[test]
    fn options_search_the_language_family() {
        let options = search_options("answer", "src/a.c");
        assert!(options.regex && options.case_sensitive && options.untracked);
        assert!(options.include.contains("*.h"));
        assert_eq!(search_options("x", "README").include, "");
        assert!(search_options("$x", "a.js").query.contains("\\$x"));
    }

    #[test]
    fn uses_and_comments_are_dropped_and_bodies_come_first() {
        let found = rank(
            "answer",
            "a.c",
            6,
            vec![
                matched("b.h", 5, "int answer(void);"),
                matched("b.c", 3, "int answer(void) { return 42; }"),
                matched("a.c", 6, "    return twice(answer());"),
                matched("a.c", 9, "// int answer(void) is in b.c"),
                matched("tests/answer_test.c", 2, "int answer(void) { return 0; }"),
            ],
        );
        let places: Vec<(&str, u32)> = found
            .iter()
            .map(|d| (d.relative.as_str(), d.line))
            .collect();
        assert_eq!(places, [("b.c", 3), ("b.h", 5), ("tests/answer_test.c", 2)]);
        assert_eq!((found[0].column, found[0].len), (5, 6));
    }

    #[test]
    fn the_jumps_own_line_comes_last() {
        let found = rank(
            "twice",
            "a.c",
            3,
            vec![
                matched("a.c", 3, "static int twice(int x) { return x * 2; }"),
                matched("lib/x.c", 8, "int twice(int y) {"),
            ],
        );
        assert_eq!(found.last().unwrap().line, 3);
    }

    /// The demo project: `answer` has a body in b.c and a prototype in b.h,
    /// `twice` is in this file and in lib/, `helper` is in two files of lib/.
    #[test]
    fn a_clear_winner_opens_alone_and_a_tie_asks() {
        let winner = |name: &str, found: Vec<SearchMatch>| {
            let ranked = rank(name, "a.c", 7, found);
            clear_winner(&ranked).map(|d| (d.relative.clone(), d.line))
        };
        let answer = vec![
            matched("b.h", 5, "int answer(void);"),
            matched("b.c", 3, "int answer(void) { return 42; }"),
        ];
        assert_eq!(winner("answer", answer), Some(("b.c".into(), 3)));
        let twice = vec![
            matched("a.c", 3, "static int twice(int x) { return x * 2; }"),
            matched("lib/twice.c", 1, "int twice(int y) {"),
        ];
        assert_eq!(winner("twice", twice), Some(("a.c".into(), 3)));
        let helper = vec![
            matched("lib/x.c", 1, "int helper(void) { return 1; }"),
            matched("lib/y.c", 1, "int helper(void) { return 2; }"),
        ];
        assert_eq!(winner("helper", helper), None);
    }

    /// The pattern in `git grep -E` itself, on files of several languages.
    #[test]
    fn git_grep_finds_declarations_in_many_languages() {
        let dir = std::env::temp_dir().join(format!("bencode-defs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = [
            (
                "a.rs",
                "pub(crate) async fn answer() {}\nlet y = answer();\n",
            ),
            (
                "b.ts",
                "export const answer = (x: number) => x;\nanswer(1);\n",
            ),
            (
                "c.ts",
                "class A {\n  async answer(n: number): Promise<void> {\n  }\n}\n",
            ),
            ("d.py", "def answer():\n    return answer\n"),
            (
                "e.go",
                "func (s *Server) answer() int {\n\treturn s.answer()\n}\n",
            ),
            (
                "f.c",
                "#define ANSWER 42\nint answer(void) {\n  return answer();\n}\n",
            ),
        ];
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(status.status.success(), "{status:?}");
        };
        git(&["init", "-q"]);
        let lines = |name: &str, from: &str| -> Vec<(String, u32)> {
            let found = crate::search::search(
                &dir,
                &search_options(name, from),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let mut lines: Vec<(String, u32)> = found
                .matches
                .into_iter()
                .map(|m| (m.relative, m.line))
                .collect();
            lines.sort();
            lines
        };
        let line = |file: &str, n: u32| (file.to_string(), n);
        assert_eq!(lines("answer", "x.rs"), [line("a.rs", 1)]);
        assert_eq!(lines("answer", "x.ts"), [line("b.ts", 1), line("c.ts", 2)]);
        assert_eq!(lines("answer", "x.py"), [line("d.py", 1)]);
        assert_eq!(lines("answer", "x.go"), [line("e.go", 1)]);
        assert_eq!(lines("answer", "x.c"), [line("f.c", 2)]);
        assert_eq!(lines("ANSWER", "x.c"), [line("f.c", 1)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
