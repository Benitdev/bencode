//! `@file` labels (MonoCode `fileMentions.ts`): each file or folder gets the
//! shortest label that names it alone — its bare name when that is unique,
//! else its path — so the composer reads `@agent.rs` rather than
//! `@src/app/agent.rs`. Known labels are painted in the prompt, and on send
//! the turn spells out where each short label points.

use std::collections::HashMap;
use std::ops::Range;

/// MonoCode `MAX_QUERY`.
const MAX_QUERY: usize = 120;
const TRAILING_PUNCTUATION: [char; 11] = [',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '/'];

/// What a label points at: a project-relative path, folder or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionTarget {
    pub relative: String,
    pub dir: bool,
}

/// Every writable label, and each path's preferred one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MentionIndex {
    labels: HashMap<String, MentionTarget>,
    label_of: HashMap<String, String>,
}

/// MonoCode `isTokenSafe`: one word, no `@` or backslash, no control marks.
fn token_safe(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_QUERY
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c == '@' || c == '\\' || c.is_control())
}

/// MonoCode `encodeMentionToken`: whitespace runs become `-`.
fn encode(relative: &str) -> Option<String> {
    let encoded = relative.split_whitespace().collect::<Vec<_>>().join("-");
    token_safe(&encoded).then_some(encoded)
}

fn name_of(relative: &str) -> &str {
    relative.rsplit('/').next().unwrap_or(relative)
}

impl MentionIndex {
    /// MonoCode `buildMentionIndex` over files, folders and note paths.
    pub fn build(files: &[impl AsRef<str>], dirs: &[impl AsRef<str>], notes: &[String]) -> Self {
        let entries: Vec<MentionTarget> = files
            .iter()
            .map(|f| (f.as_ref(), false))
            .chain(dirs.iter().map(|d| (d.as_ref(), true)))
            .filter(|(rel, _)| token_safe(rel) || encode(rel).is_some())
            .map(|(rel, dir)| MentionTarget {
                relative: rel.to_string(),
                dir,
            })
            .collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in &entries {
            *counts.entry(name_of(&entry.relative)).or_default() += 1;
        }
        let mut index = Self::default();
        let claim = |index: &mut Self, label: String, target: &MentionTarget, preferred: bool| {
            if !token_safe(&label) || index.labels.contains_key(&label) {
                return;
            }
            if preferred || !index.label_of.contains_key(&target.relative) {
                index
                    .label_of
                    .insert(target.relative.clone(), label.clone());
            }
            index.labels.insert(label, target.clone());
        };
        for entry in &entries {
            let name = name_of(&entry.relative);
            if !entry.dir && counts.get(name) == Some(&1) && token_safe(name) {
                claim(&mut index, name.to_string(), entry, true);
            }
            if token_safe(&entry.relative) {
                claim(&mut index, entry.relative.clone(), entry, false);
            }
        }
        for entry in &entries {
            if index.label_of.contains_key(&entry.relative) {
                continue;
            }
            let Some(encoded) = encode(&entry.relative) else {
                continue;
            };
            let mut label = encoded.clone();
            let mut n = 2;
            while index.labels.contains_key(&label) && n < 1000 {
                label = format!("{encoded}~{n}");
                n += 1;
            }
            claim(&mut index, label, entry, true);
        }
        for note in notes {
            let target = MentionTarget {
                relative: note.clone(),
                dir: false,
            };
            claim(&mut index, note.clone(), &target, true);
        }
        index
    }

    /// MonoCode `mentionLabel`: the label to write for `relative`.
    pub fn label_for<'a>(&'a self, relative: &'a str) -> &'a str {
        self.label_of.get(relative).map_or(relative, String::as_str)
    }

    /// MonoCode `resolveLabel`: `@App.tsx,` still names `App.tsx`.
    fn resolve(&self, raw: &str) -> Option<(&str, &MentionTarget)> {
        let mut value = raw;
        loop {
            if let Some((label, target)) = self.labels.get_key_value(value) {
                return Some((label.as_str(), target));
            }
            let last = value.chars().last()?;
            if !TRAILING_PUNCTUATION.contains(&last) {
                return None;
            }
            value = &value[..value.len() - last.len_utf8()];
        }
    }

    /// MonoCode `scanMentions`: every known `@label` in `text`, with a
    /// trailing ` (line 12)` / ` (lines 3-9)` taken in, as byte ranges.
    pub fn scan(&self, text: &str) -> Vec<(Range<usize>, &MentionTarget, &str)> {
        if self.labels.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        let mut at = 0;
        while let Some(found) = text[at..].find('@') {
            let start = at + found;
            at = start + 1;
            let starts_word = text[..start].chars().last().is_none_or(char::is_whitespace);
            if !starts_word || in_blockquote(text, start) {
                continue;
            }
            let end = text[start + 1..]
                .find(char::is_whitespace)
                .map_or(text.len(), |e| start + 1 + e);
            let Some((label, target)) = self.resolve(&text[start + 1..end]) else {
                continue;
            };
            let mention_end = start + 1 + label.len();
            hits.push((
                mention_end_with_lines(text, start..mention_end),
                target,
                label,
            ));
            at = mention_end;
        }
        hits
    }
}

/// A mention on a `>` quoted line is quoted text, not a reference.
fn in_blockquote(text: &str, at: usize) -> bool {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..at].trim_start().starts_with('>')
}

/// MonoCode `LINE_LOCATION_RE`: ` (line 12)` or ` (lines 3-9)` after a mention.
fn mention_end_with_lines(text: &str, range: Range<usize>) -> Range<usize> {
    let rest = &text[range.end..];
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
    let close = |s: &str, n: usize| (n > 0 && s[n..].starts_with(')')).then_some(n + 1);
    let extra = if let Some(after) = rest.strip_prefix(" (line ") {
        close(after, digits(after)).map(|n| " (line ".len() + n)
    } else if let Some(after) = rest.strip_prefix(" (lines ") {
        let first = digits(after);
        let dash = after[first..]
            .chars()
            .next()
            .filter(|c| first > 0 && (*c == '-' || *c == '–'));
        dash.and_then(|d| {
            let tail = &after[first + d.len_utf8()..];
            close(tail, digits(tail)).map(|n| " (lines ".len() + first + d.len_utf8() + n)
        })
    } else {
        None
    };
    range.start..range.end + extra.unwrap_or(0)
}

/// MonoCode `applyFileMentionsToTurn`: short labels get a line saying which
/// path they mean, so the agent need not guess which `mod.rs` was meant.
pub fn spell_out_mentions(text: &str, index: &MentionIndex) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let lines: Vec<String> = index
        .scan(text)
        .into_iter()
        .filter(|(_, target, label)| {
            let fresh = !seen.contains(&target.relative.as_str());
            seen.push(&target.relative);
            fresh && *label != target.relative
        })
        .map(|(_, target, label)| format!("- @{label} → {}", target.relative))
        .collect();
    if lines.is_empty() {
        return text.to_string();
    }
    format!(
        "{text}\n\n---\nReferenced with @ above:\n{}",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> MentionIndex {
        let files = [
            "src/app/agent.rs",
            "src/ui/mod.rs",
            "src/app/mod.rs",
            "docs/My Plan.md",
        ];
        let dirs = ["src", "src/app", "src/ui", "docs"];
        MentionIndex::build(&files, &dirs, &["note/plan".to_string()])
    }

    #[test]
    fn unique_names_are_short_and_clashes_keep_their_path() {
        let index = index();
        assert_eq!(index.label_for("src/app/agent.rs"), "agent.rs");
        assert_eq!(index.label_for("src/ui/mod.rs"), "src/ui/mod.rs");
        assert_eq!(index.label_for("src/app"), "src/app");
        assert_eq!(index.label_for("docs/My Plan.md"), "docs/My-Plan.md");
    }

    #[test]
    fn scan_finds_known_labels_and_line_suffixes() {
        let index = index();
        let text = "look at @agent.rs, and @src/ui/mod.rs (lines 3-9) not @nope or a@agent.rs";
        let hits = index.scan(text);
        assert_eq!(hits.len(), 2);
        assert_eq!(&text[hits[0].0.clone()], "@agent.rs");
        assert_eq!(&text[hits[1].0.clone()], "@src/ui/mod.rs (lines 3-9)");
        assert!(index.scan("> quoted @agent.rs").is_empty());
        assert_eq!(index.scan("see @note/plan").len(), 1);
    }

    #[test]
    fn sent_turns_spell_out_short_labels() {
        let index = index();
        let sent = spell_out_mentions("fix @agent.rs and @src/ui/mod.rs, again @agent.rs", &index);
        assert_eq!(
            sent,
            "fix @agent.rs and @src/ui/mod.rs, again @agent.rs\n\n---\nReferenced with @ above:\n- @agent.rs → src/app/agent.rs"
        );
        assert_eq!(spell_out_mentions("no mentions", &index), "no mentions");
    }
}
