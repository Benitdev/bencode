//! MonoCode `features/monos/model/monoMemory.ts` and `monoFiles.ts`'s
//! budget: a Mono's `MEMORY.md` as text in and text out. Every entry has
//! the same shape, one dated bullet (`- 2026-10-04 · fact · until
//! 2026-11-01`), superseded rather than deleted, and never a secret. Lines
//! without a date are the user's and are left as they wrote them.
//!
//! MonoCode's agent writes these through its `app` CLI (`memory.add`,
//! `memory.replace`, …); BenCode has no such CLI, so only the user's edits
//! (Settings › Memory) go through here. The secret patterns are MonoCode's,
//! matched by hand (BenCode carries no regex engine).

/// MonoCode `MEMORY_MAX_LINES` / `MEMORY_MAX_BYTES`: only this much of
/// `MEMORY.md` loads into a turn, whichever cuts first.
pub const MEMORY_MAX_LINES: usize = 200;
pub const MEMORY_MAX_BYTES: usize = 24 * 1024;
/// MonoCode `MEMORY_ENTRY_MAX`: one entry is at most this long, so a single
/// note cannot crowd out the rest.
pub const MEMORY_ENTRY_MAX: usize = 1000;

/// " · ", the separator between an entry's parts.
const SEP: &str = " · ";

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(ix, b)| match ix {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// MonoCode `DATED`: `- 2026-10-04 · ` (or struck, `- ~~2026-10-04 · `),
/// the entry's date.
fn dated(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("- ")?;
    let rest = rest.strip_prefix("~~").unwrap_or(rest);
    let date = rest.get(..10)?;
    (is_date(date) && rest[10..].starts_with(SEP)).then_some(date)
}

/// MonoCode `UNTIL`: ` · until 2026-11-01` at the end of the line; the line
/// without it, and the date.
fn split_until(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_end();
    let cut = trimmed.len().checked_sub(10)?;
    let date = trimmed.get(cut..)?;
    let head = trimmed[..cut].strip_suffix(" · until ")?;
    is_date(date).then_some((head, date))
}

fn without_until(line: &str) -> &str {
    split_until(line).map_or(line, |(head, _)| head)
}

/// MonoCode `memoryDate`: the local calendar date entries are dated with.
pub fn memory_date() -> String {
    jiff::Zoned::now().date().strftime("%Y-%m-%d").to_string()
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// How many leading bytes of `text` pass `allowed`.
fn run(text: &str, allowed: impl Fn(u8) -> bool) -> usize {
    text.bytes().take_while(|b| allowed(*b)).count()
}

fn base64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// The length of a credential starting at `text`, by MonoCode's `SECRETS`.
fn secret_at(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let at_least = |n: usize, min: usize| (n >= min).then_some(n);
    // `\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}`
    if ["sk-", "pk-", "rk-"].iter().any(|p| text.starts_with(p)) {
        if let Some(n) = at_least(run(&text[3..], base64url), 16) {
            return Some(3 + n);
        }
    }
    // `\bgh[pousr]_[A-Za-z0-9]{20,}`
    if text.starts_with("gh") && bytes.len() > 4 && b"pousr".contains(&bytes[2]) && bytes[3] == b'_'
    {
        if let Some(n) = at_least(run(&text[4..], |b| b.is_ascii_alphanumeric()), 20) {
            return Some(4 + n);
        }
    }
    // `\bgithub_pat_[A-Za-z0-9_]{20,}`
    if let Some(rest) = text.strip_prefix("github_pat_")
        && let Some(n) = at_least(run(rest, word_byte), 20)
    {
        return Some(11 + n);
    }
    // `\bxox[abprs]-[A-Za-z0-9-]{10,}`
    if text.starts_with("xox") && bytes.len() > 5 && b"abprs".contains(&bytes[3]) && bytes[4] == b'-'
    {
        let n = run(&text[5..], |b| b.is_ascii_alphanumeric() || b == b'-');
        if n >= 10 {
            return Some(5 + n);
        }
    }
    // `\bAKIA[0-9A-Z]{16}\b`
    if let Some(rest) = text.strip_prefix("AKIA")
        && run(rest, |b| b.is_ascii_digit() || b.is_ascii_uppercase()) >= 16
        && !rest.as_bytes().get(16).is_some_and(|b| word_byte(*b))
    {
        return Some(20);
    }
    // `\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}`
    if let Some(rest) = text.strip_prefix("eyJ") {
        let first = run(rest, base64url);
        let after = &rest[first..];
        if first >= 10 && after.starts_with('.') {
            let second = run(&after[1..], base64url);
            let after = &after[1 + second..];
            if second >= 10 && after.starts_with('.') {
                let third = run(&after[1..], base64url);
                if third >= 10 {
                    return Some(3 + first + 1 + second + 1 + third);
                }
            }
        }
    }
    // `\b(?:password|passwd|secret|token|api[_-]?key)\s*[:=]\s*\S+`, any case.
    let lower = text.get(..text.len().min(8))?.to_ascii_lowercase();
    let word = ["password", "passwd", "secret", "token", "api_key", "api-key", "apikey"]
        .iter()
        .find(|word| lower.starts_with(**word))?;
    let mut at = word.len();
    at += text[at..].len() - text[at..].trim_start().len();
    if !text[at..].starts_with([':', '=']) {
        return None;
    }
    at += 1;
    at += text[at..].len() - text[at..].trim_start().len();
    let value: usize = text[at..]
        .chars()
        .take_while(|c| !c.is_whitespace())
        .map(char::len_utf8)
        .sum();
    (value > 0).then_some(at + value)
}

/// A private key block from its BEGIN line to its END line, if `text`
/// starts with one.
fn private_key_at(text: &str) -> Option<usize> {
    let label = |rest: &str| -> Option<usize> {
        let end = rest.find("-----")?;
        let name = &rest[..end];
        (name.ends_with("PRIVATE KEY") && name.bytes().all(|b| b.is_ascii_uppercase() || b == b' '))
            .then_some(end + 5)
    };
    let rest = text.strip_prefix("-----BEGIN ")?;
    let head = 11 + label(rest)?;
    let close = text[head..].find("-----END ")? + head;
    let tail = label(&text[close + 9..])?;
    Some(close + 9 + tail)
}

/// MonoCode `redactSecrets`: anything that looks like a credential,
/// replaced before it reaches disk.
pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut ix = 0;
    let mut prev_word = false;
    while ix < text.len() {
        let rest = &text[ix..];
        let found = private_key_at(rest).or_else(|| {
            let starts_word = rest.as_bytes()[0].is_ascii_alphabetic();
            (!prev_word && starts_word)
                .then(|| secret_at(rest))
                .flatten()
        });
        if let Some(len) = found {
            let chars = rest[..len].chars().count();
            out.push_str(&format!("«redacted {chars} chars»"));
            ix += len;
            prev_word = false;
            continue;
        }
        let c = rest.chars().next().unwrap_or_default();
        prev_word = c.is_ascii_alphanumeric() || c == '_';
        out.push(c);
        ix += c.len_utf8();
    }
    out
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// MonoCode `memoryEntry`: a fact as one dated bullet.
pub fn memory_entry(fact: &str, date: &str, until: Option<&str>) -> Result<String, String> {
    let text = squash(&redact_secrets(fact));
    if text.is_empty() {
        return Err("fact must not be empty".to_string());
    }
    if text.chars().count() > MEMORY_ENTRY_MAX {
        return Err(format!("fact must be under {MEMORY_ENTRY_MAX} characters"));
    }
    if until.is_some_and(|until| !is_date(until)) {
        return Err("until must be a date like 2026-10-31".to_string());
    }
    Ok(match until {
        Some(until) => format!("- {date}{SEP}{text}{SEP}until {until}"),
        None => format!("- {date}{SEP}{text}"),
    })
}

fn lines(text: &str) -> Vec<&str> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        text.trim_end().split('\n').collect()
    }
}

fn join(list: &[&str]) -> String {
    if list.is_empty() {
        String::new()
    } else {
        format!("{}\n", list.join("\n"))
    }
}

/// MonoCode `factOf`: the fact a line holds, without its date,
/// strike-through or until.
fn fact_of(line: &str) -> String {
    let mut text = line;
    if let Some(date) = dated(text) {
        let at = text.find(date).unwrap_or(0) + date.len() + SEP.len();
        text = &text[at..];
    } else if let Some(rest) = text.strip_prefix("- ") {
        text = rest;
    }
    let text = without_until(text).replace("~~", "");
    strip_marker(&text, " · superseded ").trim().to_string()
}

/// `text` without a trailing ` · superseded 2026-10-04` (or ` · moved …`
/// with `marker` so), and anything after it.
fn strip_marker<'a>(text: &'a str, marker: &str) -> &'a str {
    let mut from = 0;
    while let Some(at) = text[from..].find(marker) {
        let at = from + at;
        if text
            .get(at + marker.len()..at + marker.len() + 10)
            .is_some_and(is_date)
        {
            return &text[..at];
        }
        from = at + marker.len();
    }
    text
}

/// MonoCode `addMemoryEntry`: appends the entry unless the same fact is
/// already there, live. Whether it was added.
pub fn add_memory_entry(text: &str, entry: &str) -> (String, bool) {
    let mut list = lines(text);
    let fact = fact_of(entry);
    if list
        .iter()
        .any(|line| !line.contains("~~") && fact_of(line) == fact)
    {
        return (join(&list), false);
    }
    list.push(entry);
    (join(&list), true)
}

/// MonoCode `MemoryLine`: one line of `MEMORY.md` as the Memory page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLine {
    /// Its line in the file, for editing or removing it.
    pub index: usize,
    /// The line as it is in the file, to find it again after a reload.
    pub line: String,
    /// The fact itself, without its bullet, date or markers.
    pub text: String,
    pub date: Option<String>,
    pub until: Option<String>,
    /// Superseded by a newer entry; kept for the record.
    pub struck: bool,
}

/// MonoCode `memoryLines`: `MEMORY.md` as a list of facts. Blank lines and
/// headings are left out; any other line is a fact, dated or not, so a
/// hand-written file lists too.
pub fn memory_lines(text: &str) -> Vec<MemoryLine> {
    let mut out = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.trim();
        let heading = line
            .find(|c: char| c != '#')
            .is_some_and(|hashes| (1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace));
        if line.is_empty() || heading {
            continue;
        }
        let struck = line.starts_with("- ~~");
        let date = dated(line).map(str::to_string);
        let until = split_until(line).map(|(_, until)| until.to_string());
        let mut fact = line;
        if let Some(rest) = fact
            .strip_prefix("- ")
            .or_else(|| fact.strip_prefix("* "))
        {
            fact = rest.trim_start();
        }
        let fact = fact.replace("~~", "");
        let fact = match fact.get(..10).filter(|d| is_date(d)) {
            Some(_) if fact[10..].starts_with(SEP) => fact[10 + SEP.len()..].to_string(),
            _ => fact,
        };
        let fact = without_until(&fact);
        let fact = strip_marker(strip_marker(fact, " · superseded "), " · moved ").trim();
        if fact.is_empty() {
            continue;
        }
        out.push(MemoryLine {
            index,
            line: raw.to_string(),
            text: fact.to_string(),
            date,
            until,
            struck,
        });
    }
    out
}

/// MonoCode `factIndex`: where the fact first listed as `original` is now,
/// after other writers may have added or moved lines.
pub fn fact_index(memory: &str, original: &str) -> Result<usize, String> {
    let found: Vec<usize> = memory
        .split('\n')
        .enumerate()
        .filter(|(_, line)| *line == original)
        .map(|(ix, _)| ix)
        .collect();
    match found.as_slice() {
        [only] => Ok(*only),
        _ => Err("This memory changed or is ambiguous. Check it before trying again.".to_string()),
    }
}

/// MonoCode `withoutLine`: the file without one of its lines.
pub fn without_line(text: &str, index: usize) -> String {
    let mut list: Vec<&str> = text.split('\n').collect();
    if index < list.len() {
        list.remove(index);
    }
    list.join("\n")
}

/// MonoCode `withLineEdited`: the file with one fact reworded in place. A
/// dated entry is redated to `date` and keeps its until; a line the user
/// wrote stays undated, with its bullet.
pub fn with_line_edited(text: &str, index: usize, fact: &str, date: &str) -> Result<String, String> {
    let mut list: Vec<String> = text.split('\n').map(str::to_string).collect();
    let old = list.get(index).cloned().unwrap_or_default();
    let line = if dated(old.trim()).is_some() {
        let until = split_until(&old).map(|(_, until)| until);
        memory_entry(fact, date, until)?
    } else {
        let clean = squash(&redact_secrets(fact));
        if clean.is_empty() {
            return Err("fact must not be empty".to_string());
        }
        let trimmed = old.trim_start();
        let indent = &old[..old.len() - trimmed.len()];
        let bullet = ["- ", "* "]
            .iter()
            .find(|b| trimmed.starts_with(**b))
            .map(|b| {
                let after = &trimmed[b.len()..];
                &trimmed[..b.len() + after.len() - after.trim_start().len()]
            })
            .map_or(String::new(), |bullet| format!("{indent}{bullet}"));
        format!("{bullet}{clean}")
    };
    match list.get_mut(index) {
        Some(slot) => *slot = line,
        None => list.push(line),
    }
    Ok(list.join("\n"))
}

/// MonoCode `MemoryBudget`: what of `MEMORY.md` loads into a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryBudget {
    pub text: String,
    pub lines: usize,
    pub bytes: usize,
    /// Lines past the budget, which do not load.
    pub dropped_lines: usize,
}

impl MemoryBudget {
    /// MonoCode `MemoryGauge`'s share: the larger of the line and byte use.
    pub fn share(&self) -> f32 {
        (self.lines as f32 / MEMORY_MAX_LINES as f32)
            .max(self.bytes as f32 / MEMORY_MAX_BYTES as f32)
    }

    /// The gauge's words: "12 / 200 lines", or what does not load.
    pub fn label(&self) -> String {
        if self.dropped_lines > 0 {
            format!("{} lines not loading", self.dropped_lines)
        } else {
            format!("{} / {MEMORY_MAX_LINES} lines", self.lines)
        }
    }
}

/// MonoCode `memoryWithinBudget`: the part of `MEMORY.md` that fits, whole
/// lines only.
pub fn memory_within_budget(memory: &str) -> MemoryBudget {
    let all = lines(memory);
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0;
    for line in &all {
        let size = line.len() + 1;
        if kept.len() >= MEMORY_MAX_LINES || bytes + size > MEMORY_MAX_BYTES {
            break;
        }
        kept.push(line);
        bytes += size;
    }
    MemoryBudget {
        text: kept.join("\n"),
        lines: all.len(),
        bytes: memory.len(),
        dropped_lines: all.len() - kept.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_one_dated_bullet() {
        assert_eq!(
            memory_entry("  likes   short replies ", "2026-10-04", None).unwrap(),
            "- 2026-10-04 · likes short replies"
        );
        assert_eq!(
            memory_entry("on leave", "2026-10-04", Some("2026-11-01")).unwrap(),
            "- 2026-10-04 · on leave · until 2026-11-01"
        );
        assert!(memory_entry("  ", "2026-10-04", None).is_err());
        assert!(memory_entry("x", "2026-10-04", Some("soon")).is_err());
        assert!(memory_entry(&"a".repeat(1001), "2026-10-04", None).is_err());
    }

    #[test]
    fn secrets_never_reach_the_file() {
        let key = format!("sk-{}", "a".repeat(20));
        assert_eq!(
            redact_secrets(&format!("use {key} here")),
            "use «redacted 23 chars» here"
        );
        assert_eq!(
            redact_secrets(&format!("gh{}_{}", "p", "A".repeat(24))),
            "«redacted 28 chars»"
        );
        assert_eq!(redact_secrets("AKIAABCDEFGHIJKLMNOP"), "«redacted 20 chars»");
        assert_eq!(redact_secrets("password: hunter2 ok"), "«redacted 17 chars» ok");
        assert_eq!(redact_secrets("API_KEY=abc"), "«redacted 11 chars»");
        let jwt = format!("eyJ{0}.{0}.{0}", "x".repeat(12));
        assert_eq!(redact_secrets(&jwt), format!("«redacted {} chars»", jwt.len()));
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----";
        assert_eq!(redact_secrets(&format!("k {pem} done")), "k «redacted 65 chars» done");
        // Inside a word, or too short, it is not a key.
        for plain in ["task-runner", "risk-aaaaaaaaaaaaaaaaaaaa", "tokens are fine", "sk-short"] {
            assert_eq!(redact_secrets(plain), plain);
        }
    }

    #[test]
    fn the_same_fact_is_not_added_twice() {
        let memory = "- 2026-10-01 · likes tea\n";
        let (text, added) = add_memory_entry(memory, "- 2026-10-04 · likes tea");
        assert!(!added);
        assert_eq!(text, memory);
        let (text, added) = add_memory_entry(memory, "- 2026-10-04 · likes coffee");
        assert!(added);
        assert_eq!(text, "- 2026-10-01 · likes tea\n- 2026-10-04 · likes coffee\n");
        let (text, added) = add_memory_entry("", "- 2026-10-04 · first");
        assert!(added);
        assert_eq!(text, "- 2026-10-04 · first\n");
    }

    #[test]
    fn the_file_lists_as_facts() {
        let memory = "# Memory\n\n- 2026-10-01 · likes tea\n- ~~2026-09-01 · old desk~~ · superseded 2026-10-01\nhand written\n- 2026-10-02 · away · until 2026-11-01\n";
        let facts = memory_lines(memory);
        let texts: Vec<&str> = facts.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, ["likes tea", "old desk", "hand written", "away"]);
        assert_eq!(facts[0].index, 2);
        assert_eq!(facts[0].date.as_deref(), Some("2026-10-01"));
        assert!(facts[1].struck);
        assert_eq!(facts[1].date.as_deref(), Some("2026-09-01"));
        assert_eq!(facts[2].date, None);
        assert_eq!(facts[3].until.as_deref(), Some("2026-11-01"));
    }

    #[test]
    fn a_fact_is_reworded_in_place() {
        let memory = "- 2026-10-01 · away · until 2026-11-01\n- mine\n* also mine";
        let edited = with_line_edited(memory, 0, "on leave", "2026-10-05").unwrap();
        assert_eq!(
            edited,
            "- 2026-10-05 · on leave · until 2026-11-01\n- mine\n* also mine"
        );
        let edited = with_line_edited(memory, 1, "still mine", "2026-10-05").unwrap();
        assert!(edited.contains("\n- still mine\n"));
        let edited = with_line_edited(memory, 2, "and this", "2026-10-05").unwrap();
        assert!(edited.ends_with("\n* and this"));
        assert!(with_line_edited(memory, 1, " ", "2026-10-05").is_err());
        assert_eq!(without_line(memory, 1), "- 2026-10-01 · away · until 2026-11-01\n* also mine");
    }

    #[test]
    fn a_fact_is_found_again_only_when_it_is_unique() {
        assert_eq!(fact_index("a\nb\nc", "b"), Ok(1));
        assert!(fact_index("a\nb\nb", "b").is_err());
        assert!(fact_index("a", "b").is_err());
    }

    #[test]
    fn only_the_budget_loads() {
        let budget = memory_within_budget("");
        assert_eq!((budget.lines, budget.dropped_lines), (0, 0));
        assert_eq!(budget.label(), "0 / 200 lines");
        let long: String = (0..210).map(|n| format!("- {n}\n")).collect();
        let budget = memory_within_budget(&long);
        assert_eq!(budget.lines, 210);
        assert_eq!(budget.dropped_lines, 10);
        assert_eq!(budget.text.lines().count(), 200);
        assert_eq!(budget.label(), "10 lines not loading");
        let wide = format!("{}\n{}\n", "a".repeat(20_000), "b".repeat(5_000));
        let budget = memory_within_budget(&wide);
        assert_eq!(budget.dropped_lines, 1);
        assert!(budget.share() > 1.0);
    }
}
