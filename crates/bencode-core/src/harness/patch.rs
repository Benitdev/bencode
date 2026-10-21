//! What a file edit changed, as the hunks of a unified diff, for the
//! transcript to show under the edit (Claude Code's `Update(…)` rows).
//! Only harnesses that report the change carry one: Claude's
//! `tool_use_result.structuredPatch`. Codex's `exec --json` names the
//! files only; Grok's ACP diffs are not read yet.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most lines a patch keeps; the rest is counted in `omitted`.
const MAX_LINES: usize = 400;
/// The most characters a kept line keeps (minified code).
const MAX_LINE_CHARS: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPatch {
    pub hunks: Vec<PatchHunk>,
    /// Lines added and removed, the omitted ones included.
    pub added: usize,
    pub removed: usize,
    /// The edit wrote a new file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub created: bool,
    /// Lines past `MAX_LINES`, not kept.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

/// A run of lines, each starting with ` `, `-` or `+`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl EditPatch {
    /// Claude's `tool_use_result` for an Edit, MultiEdit or Write: its
    /// `structuredPatch`, or a created file's `content` as added lines.
    pub fn from_claude(result: &Value) -> Option<Self> {
        let created = result.get("type").and_then(Value::as_str) == Some("create");
        let hunks: Vec<PatchHunk> = match result.get("structuredPatch") {
            Some(Value::Array(hunks)) if !hunks.is_empty() => {
                hunks.iter().filter_map(claude_hunk).collect()
            }
            _ if created => {
                let content = result.get("content").and_then(Value::as_str)?;
                vec![PatchHunk {
                    old_start: 0,
                    new_start: 1,
                    lines: content.lines().map(|line| format!("+{line}")).collect(),
                }]
            }
            _ => return None,
        };
        Self::capped(hunks, created)
    }

    /// Counts the lines, then keeps the first `MAX_LINES`; `None` when
    /// nothing changed.
    fn capped(hunks: Vec<PatchHunk>, created: bool) -> Option<Self> {
        let all = || hunks.iter().flat_map(|hunk| hunk.lines.iter());
        let added = all().filter(|line| line.starts_with('+')).count();
        let removed = all().filter(|line| line.starts_with('-')).count();
        if added + removed == 0 {
            return None;
        }
        let mut budget = MAX_LINES;
        let mut omitted = 0;
        let mut kept = Vec::new();
        for mut hunk in hunks {
            if budget == 0 {
                omitted += hunk.lines.len();
                continue;
            }
            if hunk.lines.len() > budget {
                omitted += hunk.lines.len() - budget;
                hunk.lines.truncate(budget);
            }
            budget -= hunk.lines.len();
            for line in &mut hunk.lines {
                if let Some((at, _)) = line.char_indices().nth(MAX_LINE_CHARS) {
                    line.truncate(at);
                }
            }
            kept.push(hunk);
        }
        Some(Self {
            hunks: kept,
            added,
            removed,
            created,
            omitted,
        })
    }
}

fn claude_hunk(hunk: &Value) -> Option<PatchHunk> {
    let number = |key: &str| hunk.get(key).and_then(Value::as_u64).map(|n| n as u32);
    Some(PatchHunk {
        old_start: number("oldStart")?,
        new_start: number("newStart")?,
        lines: hunk
            .get("lines")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            // `\ No newline at end of file` is not a line of the file.
            .filter(|line| !line.starts_with('\\'))
            .map(String::from)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_edit_keeps_its_hunks_and_counts() {
        // Recorded from Claude Code 2.x (`tool_use_result` of an Edit).
        let patch = EditPatch::from_claude(&json!({
            "filePath": "/r/a.txt", "oldString": "line two", "newString": "LINE 2\nline 2b",
            "structuredPatch": [{"oldStart": 1, "oldLines": 4, "newStart": 1, "newLines": 5,
                "lines": [" line one", "-line two", "+LINE 2", "+line 2b", " line three"]}],
            "userModified": false, "replaceAll": false
        }))
        .unwrap();
        assert_eq!((patch.added, patch.removed, patch.created), (2, 1, false));
        assert_eq!(patch.hunks[0].lines[1], "-line two");
        assert_eq!(patch.omitted, 0);
    }

    #[test]
    fn a_created_file_is_all_added() {
        let patch = EditPatch::from_claude(&json!({
            "type": "create", "filePath": "/r/b.txt", "content": "hi\nthere\n",
            "structuredPatch": [], "originalFile": null
        }))
        .unwrap();
        assert!(patch.created);
        assert_eq!(patch.added, 2);
        assert_eq!(patch.hunks[0].lines, ["+hi", "+there"]);
        assert_eq!(patch.hunks[0].new_start, 1);
    }

    #[test]
    fn other_results_carry_no_patch() {
        assert_eq!(EditPatch::from_claude(&json!({"type": "text"})), None);
        let unchanged =
            json!({"structuredPatch": [{"oldStart": 1, "newStart": 1, "lines": [" a"]}]});
        assert_eq!(EditPatch::from_claude(&unchanged), None);
    }

    #[test]
    fn a_long_patch_keeps_its_head_and_counts_the_rest() {
        let lines: Vec<String> = (0..MAX_LINES + 10).map(|i| format!("+{i}")).collect();
        let hunks = vec![
            PatchHunk {
                old_start: 0,
                new_start: 1,
                lines,
            },
            PatchHunk {
                old_start: 900,
                new_start: 900,
                lines: vec!["-x".into()],
            },
        ];
        let patch = EditPatch::capped(hunks, false).unwrap();
        assert_eq!(patch.hunks.len(), 1);
        assert_eq!(patch.hunks[0].lines.len(), MAX_LINES);
        assert_eq!(patch.omitted, 11);
        assert_eq!((patch.added, patch.removed), (MAX_LINES + 10, 1));
    }
}
