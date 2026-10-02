//! Diff lines with their real old/new line numbers, computed once off the UI thread.

use super::DiffLineKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: DiffLineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

/// Start lines from a hunk header such as `@@ -12,7 +12,9 @@ fn x`.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let start = |part: Option<&str>, sign: char| -> Option<u32> {
        part?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((start(parts.next(), '-')?, start(parts.next(), '+')?))
}

/// Numbers each line by walking the hunks: deletions advance the old side,
/// additions the new side, context both.
pub fn number_rows(lines: Vec<DiffLineKind>) -> Vec<DiffRow> {
    let (mut old, mut new) = (0u32, 0u32);
    lines
        .into_iter()
        .map(|kind| {
            let (o, n) = match &kind {
                DiffLineKind::Header(header) => {
                    if let Some((o, n)) = hunk_starts(header) {
                        (old, new) = (o, n);
                    }
                    (None, None)
                }
                DiffLineKind::Deletion(_) => (Some(old), None),
                DiffLineKind::Addition(_) => (None, Some(new)),
                DiffLineKind::Context(_) => (Some(old), Some(new)),
            };
            old += u32::from(o.is_some());
            new += u32::from(n.is_some());
            DiffRow { kind, old: o, new: n }
        })
        .collect()
}

/// The rows as unified-diff text, for copying.
pub fn unified_text(rows: &[DiffRow]) -> String {
    rows.iter()
        .map(|row| match &row.kind {
            DiffLineKind::Header(text) => format!("{text}\n"),
            DiffLineKind::Addition(text) => format!("+{text}\n"),
            DiffLineKind::Deletion(text) => format!("-{text}\n"),
            DiffLineKind::Context(text) => format!(" {text}\n"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<DiffRow> {
        number_rows(vec![
            DiffLineKind::Header("@@ -10,3 +20,4 @@ fn main".into()),
            DiffLineKind::Context("a".into()),
            DiffLineKind::Deletion("b".into()),
            DiffLineKind::Addition("c".into()),
            DiffLineKind::Addition("d".into()),
            DiffLineKind::Context("e".into()),
        ])
    }

    #[test]
    fn numbers_follow_hunk_header() {
        let numbers: Vec<_> = rows().iter().map(|r| (r.old, r.new)).collect();
        assert_eq!(
            numbers,
            [(None, None), (Some(10), Some(20)), (Some(11), None), (None, Some(21)), (None, Some(22)), (Some(12), Some(23))]
        );
    }

    #[test]
    fn unified_text_round_trips_prefixes() {
        assert_eq!(unified_text(&rows()), "@@ -10,3 +20,4 @@ fn main\n a\n-b\n+c\n+d\n e\n");
    }

    #[test]
    fn malformed_header_keeps_counting() {
        assert_eq!(hunk_starts("@@ New untracked file: x @@"), None);
        assert_eq!(hunk_starts("@@ -0,0 +1,5 @@ (new file)"), Some((0, 1)));
    }
}
