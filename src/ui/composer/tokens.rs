//! The `/command` or `@mention` being typed at the caret (MonoCode
//! `slashTokenAt`, `mentionTokenAt`, `replaceSlashToken`,
//! `replaceMentionToken`). Offsets are byte offsets into the prompt.

/// MonoCode `MAX_QUERY` for `@` queries.
const MAX_MENTION_QUERY: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Where the trigger character sits.
    pub start: usize,
    /// The end of the word the caret is in.
    pub end: usize,
    /// What is typed between the trigger and the caret.
    pub query: String,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\n' | b'\t' | b'\r')
}

/// MonoCode `isMarkdownBlockquotePosition`: `at` is on a `> ` line.
pub fn in_blockquote(text: &str, at: usize) -> bool {
    let at = at.min(text.len());
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..at];
    let indent = line.len() - line.trim_start_matches(' ').len();
    indent <= 3 && line[indent..].starts_with('>')
}

/// The word around `cursor` when it starts with `trigger`.
fn word_at(text: &str, cursor: usize, trigger: u8) -> Option<(usize, usize, &str)> {
    let bytes = text.as_bytes();
    let cursor = cursor.min(text.len());
    let mut start = cursor;
    while start > 0 && !is_space(bytes[start - 1]) {
        start -= 1;
    }
    if bytes.get(start) != Some(&trigger) || in_blockquote(text, start) {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len() && !is_space(bytes[end]) {
        end += 1;
    }
    Some((start, end, &text[start + 1..cursor.max(start + 1)]))
}

/// MonoCode `slashTokenAt` for harnesses without native commands.
pub fn slash_token_at(text: &str, cursor: usize) -> Option<Token> {
    let (start, end, typed) = word_at(text, cursor, b'/')?;
    if start > 0 && text.as_bytes()[start - 1] == b':' {
        return None;
    }
    // `^(?:[a-z0-9-]+(?::[a-z0-9-]*)?)?$`
    let word = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    let ok = match typed.split_once(':') {
        None => word(typed),
        Some((name, rest)) => !name.is_empty() && word(name) && word(rest),
    };
    ok.then(|| Token {
        start,
        end,
        query: typed.to_string(),
    })
}

/// MonoCode `mentionTokenAt`.
pub fn mention_token_at(text: &str, cursor: usize) -> Option<Token> {
    let (start, end, typed) = word_at(text, cursor, b'@')?;
    (!typed.contains('@') && typed.len() <= MAX_MENTION_QUERY).then(|| Token {
        start,
        end,
        query: typed.to_string(),
    })
}

/// MonoCode `replaceSlashToken` / `replaceMentionToken`: the token becomes
/// `replacement` followed by one space; the text after it is kept. Returns
/// the text and the caret just past that space.
pub fn replace_token(text: &str, token: &Token, replacement: &str) -> (String, usize) {
    let rest = &text[token.end.min(text.len())..];
    let spacer = if rest.starts_with(' ') { "" } else { " " };
    let next = format!("{}{replacement}{spacer}{rest}", &text[..token.start]);
    let caret = (token.start + replacement.len() + 1).min(next.len());
    (next, caret)
}

/// The token taken out, with one whitespace after it (MonoCode's `/mcp`
/// and New skill). Returns the text and the caret where the token was.
pub fn remove_token(text: &str, token: &Token) -> (String, usize) {
    let rest = &text[token.end.min(text.len())..];
    let rest = rest
        .strip_prefix(|c: char| c.is_whitespace())
        .unwrap_or(rest);
    (format!("{}{rest}", &text[..token.start]), token.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_tokens_follow_monocode_rules() {
        let text = "fix /rev now";
        let token = slash_token_at(text, 8).unwrap();
        assert_eq!(
            (token.start, token.end, token.query.as_str()),
            (4, 8, "rev")
        );
        // The caret mid-word queries what is before it.
        assert_eq!(slash_token_at(text, 6).unwrap().query, "r");
        assert_eq!(slash_token_at("/", 1).unwrap().query, "");
        assert!(slash_token_at("/Users", 6).is_none(), "uppercase is a path");
        assert!(slash_token_at("/foo.rs", 7).is_none());
        assert!(slash_token_at("a/b", 3).is_none(), "mid-word slash");
        assert!(slash_token_at("http:/x", 7).is_none());
        assert!(slash_token_at("> /plan", 7).is_none(), "blockquote");
        assert_eq!(slash_token_at("/ns:sk", 6).unwrap().query, "ns:sk");
        assert!(slash_token_at("fix it", 6).is_none());
    }

    #[test]
    fn mention_tokens_anywhere_in_the_text() {
        let text = "see @src/ma and more";
        let token = mention_token_at(text, 11).unwrap();
        assert_eq!(
            (token.start, token.end, token.query.as_str()),
            (4, 11, "src/ma")
        );
        assert!(mention_token_at("a@b", 3).is_none());
        assert!(mention_token_at("xin chào @fi", 14).is_some());
    }

    #[test]
    fn replacing_keeps_the_rest_of_the_text() {
        let text = "fix /rev now";
        let token = slash_token_at(text, 8).unwrap();
        let (next, caret) = replace_token(text, &token, "/review");
        assert_eq!(next, "fix /review now");
        assert_eq!(caret, 12);
        let (next, caret) = replace_token("/re", &slash_token_at("/re", 3).unwrap(), "/review");
        assert_eq!((next.as_str(), caret), ("/review ", 8));
        let (next, caret) = remove_token(text, &token);
        assert_eq!((next.as_str(), caret), ("fix now", 4));
    }
}
