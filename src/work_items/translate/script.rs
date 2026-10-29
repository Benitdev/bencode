//! Whether a text is already in a language, judged from its letters alone:
//! "Always translate" skips what needs no turn. A guess on purpose, cheap
//! and predictable, never a language detector:
//!
//! - Code (fenced blocks and inline spans) and URLs are not read.
//! - **Japanese**: some kana or CJK ideograph, and no Vietnamese letter.
//!   Latin words (API, XSS, product names) are common in Japanese tickets.
//! - **Vietnamese**: some Vietnamese letter (ă â đ ê ô ơ ư ĩ ũ and the
//!   tone-marked vowels of Latin Extended Additional), and no kana or
//!   ideograph: "Cho phép クーポン番号 bắt đầu bằng 0" still needs a turn.
//! - **English**: some Latin letter, and neither of the above. Other
//!   Latin-script languages pass as English.
//! - A text with no letters at all (empty, numbers, code) is in every
//!   language: there is nothing to translate.

use super::Language;

/// What the letters of a text were.
#[derive(Default)]
struct Letters {
    cjk: bool,
    vietnamese: bool,
    latin: bool,
}

/// Hiragana, katakana (and its half-width forms) and CJK ideographs.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{FF66}'..='\u{FF9F}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}')
}

/// Letters only Vietnamese uses among the languages offered.
fn is_vietnamese(c: char) -> bool {
    matches!(
        c,
        'ă' | 'Ă' | 'â' | 'Â' | 'đ' | 'Đ' | 'ê' | 'Ê' | 'ô' | 'Ô' | 'ơ' | 'Ơ' | 'ư' | 'Ư'
        | 'ĩ' | 'Ĩ' | 'ũ' | 'Ũ'
        // ạ ả ấ … ỹ: the tone-marked vowels.
        | '\u{1EA0}'..='\u{1EF9}'
    )
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || (c.is_alphabetic() && matches!(c, '\u{00C0}'..='\u{024F}'))
}

/// `text` without its fenced code blocks, inline code and URLs.
fn prose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Inline code sits between backticks: every other piece.
        for (ix, piece) in line.split('`').enumerate() {
            if ix % 2 == 1 {
                continue;
            }
            for word in piece.split_whitespace() {
                if !(word.contains("://") || word.starts_with("www.")) {
                    out.push_str(word);
                    out.push(' ');
                }
            }
        }
        out.push('\n');
    }
    out
}

fn letters(text: &str) -> Letters {
    let mut found = Letters::default();
    for c in prose(text).chars() {
        if is_cjk(c) {
            found.cjk = true;
        } else if is_vietnamese(c) {
            found.vietnamese = true;
        } else if is_latin(c) {
            found.latin = true;
        }
    }
    found
}

/// `text` reads as `language` already (the module's rules).
pub fn is_in(language: Language, text: &str) -> bool {
    let found = letters(text);
    if !(found.cjk || found.vietnamese || found.latin) {
        return true;
    }
    match language {
        Language::Japanese => found.cjk && !found.vietnamese,
        Language::Vietnamese => found.vietnamese && !found.cjk,
        Language::English => !found.cjk && !found.vietnamese,
    }
}

/// The one language `text` reads as, if only one: what a translation was
/// made from. `None` for mixed text or text with no letters.
pub fn written_in(text: &str) -> Option<Language> {
    let mut matching = Language::ALL.into_iter().filter(|l| is_in(*l, text));
    let language = matching.next()?;
    matching.next().is_none().then_some(language)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VI: Language = Language::Vietnamese;
    const EN: Language = Language::English;
    const JA: Language = Language::Japanese;

    #[test]
    fn japanese_titles_need_a_turn_into_vietnamese() {
        let title = "クーポンコードを0始まりで入力できるようにする（購入手続き画面）";
        assert!(is_in(JA, title));
        assert!(!is_in(VI, title));
        assert!(!is_in(EN, title));
        // Latin words in a Japanese ticket keep it Japanese.
        let mixed = "[Security] 商品レビュー欄経由の格納型XSS";
        assert!(is_in(JA, mixed));
        assert!(!is_in(VI, mixed));
        // Kanji alone, half-width katakana alone.
        assert!(is_in(JA, "確認"));
        assert!(is_in(JA, "ｸｰﾎﾟﾝ"));
    }

    #[test]
    fn vietnamese_needs_its_own_letters_and_nothing_japanese() {
        assert!(is_in(VI, "Cho phép mã giảm giá bắt đầu bằng 0"));
        assert!(is_in(VI, "ĐÃ XONG"));
        assert!(!is_in(EN, "Cho phép mã giảm giá bắt đầu bằng 0"));
        // Half translated: still a turn.
        let mixed = "Cho phép クーポン番号 bắt đầu bằng 0";
        assert!(!is_in(VI, mixed));
        assert!(!is_in(JA, mixed));
        assert!(!is_in(EN, mixed));
        // Plain Latin is English, not Vietnamese without its marks.
        assert!(!is_in(VI, "Allow coupon codes to start with 0"));
    }

    #[test]
    fn plain_latin_reads_as_english() {
        assert!(is_in(EN, "Allow coupon codes to start with 0"));
        assert!(is_in(EN, "Mise à jour du café"), "other Latin scripts pass");
        assert!(!is_in(JA, "Allow coupon codes to start with 0"));
    }

    #[test]
    fn code_and_urls_are_not_read() {
        let body = "Tài liệu: https://example.com/ログイン\n\n```js\n// 日本語のコメント\nconst a = 1;\n```\n\nGọi `ログイン()` rồi kiểm tra.";
        assert!(is_in(VI, body));
        assert!(is_in(EN, "See `確認` at https://example.jp/確認"));
    }

    #[test]
    fn a_source_language_is_named_only_when_it_is_one() {
        assert_eq!(written_in("クーポンコードを0始まりで入力"), Some(JA));
        assert_eq!(written_in("Cho phép mã giảm giá"), Some(VI));
        assert_eq!(written_in("Allow coupon codes"), Some(EN));
        assert_eq!(written_in("Cho phép số クーポン番号"), None);
        assert_eq!(written_in("1234"), None);
    }

    #[test]
    fn nothing_to_read_needs_no_turn() {
        for language in Language::ALL {
            assert!(is_in(language, ""));
            assert!(is_in(language, "  123 — 4.5%  "));
            assert!(is_in(language, "```\nログイン\n```"));
        }
    }
}
