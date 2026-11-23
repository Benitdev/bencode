//! Atlassian Document Format (ADF) conversion (MonoCode `src-tauri/src/jira.rs`):
//! Jira REST API v3 sends issue descriptions and comments as ADF JSON trees
//! (or plain text on older servers), while BenCode's Inbox renders Markdown.
//! Writing back turns plain text into a v1 ADF document.

use serde_json::{Value, json};

/// Descriptions and comments arrive as ADF (v3) or plain text; the Inbox renders Markdown.
pub fn rich_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(doc @ Value::Object(_)) => adf_to_markdown(doc),
        _ => String::new(),
    }
}

pub fn adf_to_markdown(doc: &Value) -> String {
    let mut blocks = Vec::new();
    for node in children(doc) {
        let block = adf_block(node, "");
        if !block.trim().is_empty() {
            blocks.push(block);
        }
    }
    blocks.join("\n\n").trim().to_string()
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn attr<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.pointer(&format!("/attrs/{name}"))
}

fn attr_str<'a>(node: &'a Value, name: &str) -> &'a str {
    attr(node, name).and_then(Value::as_str).unwrap_or_default()
}

fn adf_block(node: &Value, indent: &str) -> String {
    match node_type(node) {
        "paragraph" => adf_inline(children(node)),
        "heading" => {
            let level = attr(node, "level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), adf_inline(children(node)))
        }
        "bulletList" => adf_list(node, indent, None),
        "orderedList" => {
            let start = attr(node, "order").and_then(Value::as_u64).unwrap_or(1);
            adf_list(node, indent, Some(start))
        }
        "taskList" => adf_task_list(node, indent),
        "codeBlock" => {
            let code: String = children(node)
                .iter()
                .filter_map(|child| child.get("text").and_then(Value::as_str))
                .collect();
            format!("```{}\n{}\n```", attr_str(node, "language"), code)
        }
        "blockquote" | "panel" => adf_blocks(children(node), indent)
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "expand" | "nestedExpand" => {
            let title = attr_str(node, "title").trim();
            let body = adf_blocks(children(node), indent);
            match (title.is_empty(), body.is_empty()) {
                (true, _) => body,
                (false, true) => format!("**{title}**"),
                (false, false) => format!("**{title}**\n\n{body}"),
            }
        }
        "rule" => "---".into(),
        "table" => adf_table(node),
        "mediaSingle" | "mediaGroup" | "media" => String::new(),
        _ => {
            if children(node).is_empty() {
                adf_inline(std::slice::from_ref(node))
            } else {
                adf_blocks(children(node), indent)
            }
        }
    }
}

fn adf_blocks(nodes: &[Value], indent: &str) -> String {
    nodes
        .iter()
        .map(|node| adf_block(node, indent))
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn adf_list(node: &Value, indent: &str, start: Option<u64>) -> String {
    let mut lines = Vec::new();
    for (index, item) in children(node).iter().enumerate() {
        let marker = match start {
            Some(start) => format!("{}. ", start + index as u64),
            None => "- ".into(),
        };
        let nested = format!("{indent}{}", " ".repeat(marker.len()));
        let mut first = true;
        for child in children(item) {
            let text = adf_block(child, &nested);
            if text.trim().is_empty() {
                continue;
            }
            if matches!(node_type(child), "bulletList" | "orderedList" | "taskList") {
                lines.push(text);
            } else if first {
                lines.push(format!(
                    "{indent}{marker}{}",
                    text.replace('\n', &format!("\n{nested}"))
                ));
                first = false;
            } else {
                lines.push(format!(
                    "{nested}{}",
                    text.replace('\n', &format!("\n{nested}"))
                ));
            }
        }
    }
    lines.join("\n")
}

fn adf_task_list(node: &Value, indent: &str) -> String {
    let mut lines = Vec::new();
    let nested = format!("{indent}  ");
    for child in children(node) {
        match node_type(child) {
            "taskItem" => {
                let done = attr_str(child, "state").eq_ignore_ascii_case("DONE");
                let mark = if done { "x" } else { " " };
                let text = adf_inline(children(child));
                if !text.trim().is_empty() {
                    lines.push(format!(
                        "{indent}- [{mark}] {}",
                        text.replace('\n', &format!("\n{nested}"))
                    ));
                }
            }
            "taskList" => {
                let sub = adf_task_list(child, &nested);
                if !sub.trim().is_empty() {
                    lines.push(sub);
                }
            }
            _ => {}
        }
    }
    lines.join("\n")
}

fn adf_table(node: &Value) -> String {
    let rows: Vec<Vec<String>> = children(node)
        .iter()
        .map(|row| {
            children(row)
                .iter()
                .map(|cell| {
                    adf_blocks(children(cell), "")
                        .replace('\n', " ")
                        .replace('|', "\\|")
                })
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect();
    let Some(width) = rows.iter().map(Vec::len).max() else {
        return String::new();
    };
    let line = |cells: &[String]| {
        let mut padded = cells.to_vec();
        padded.resize(width, String::new());
        format!("| {} |", padded.join(" | "))
    };
    let mut lines = vec![line(&rows[0]), format!("|{}", " --- |".repeat(width))];
    lines.extend(rows[1..].iter().map(|row| line(row)));
    lines.join("\n")
}

fn adf_inline(nodes: &[Value]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node_type(node) {
            "text" => out.push_str(&adf_marked_text(node)),
            "hardBreak" => out.push('\n'),
            "mention" => {
                let text = attr_str(node, "text");
                if text.starts_with('@') {
                    out.push_str(text);
                } else if !text.is_empty() {
                    out.push('@');
                    out.push_str(text);
                }
            }
            "emoji" => {
                let text = attr_str(node, "text");
                out.push_str(if text.is_empty() {
                    attr_str(node, "shortName")
                } else {
                    text
                });
            }
            "inlineCard" | "blockCard" | "embedCard" => {
                let url = attr_str(node, "url");
                if !url.is_empty() {
                    out.push_str(&format!("<{url}>"));
                }
            }
            "status" => {
                let label = attr_str(node, "text");
                if !label.is_empty() {
                    out.push_str(&format!("`{label}`"));
                }
            }
            "date" => {
                if let Some(day) = format_adf_date(attr_str(node, "timestamp")) {
                    out.push_str(&day);
                }
            }
            _ => out.push_str(&adf_inline(children(node))),
        }
    }
    out
}

fn format_adf_date(timestamp: &str) -> Option<String> {
    let ms: i64 = timestamp.trim().parse().ok()?;
    let ts = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(ts.strftime("%Y-%m-%d").to_string())
}

fn adf_marked_text(node: &Value) -> String {
    let mut text = node
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return text;
    }
    let marks = node
        .get("marks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if marks.iter().any(|mark| node_type(mark) == "code") {
        return format!("`{text}`");
    }
    for mark in marks {
        text = match node_type(mark) {
            "strong" => format!("**{text}**"),
            "em" => format!("_{text}_"),
            "strike" => format!("~~{text}~~"),
            "link" => {
                let href = attr_str(mark, "href");
                if href.is_empty() {
                    text
                } else {
                    format!("[{text}]({href})")
                }
            }
            _ => text,
        };
    }
    text
}

/// Posts plain text as ADF: blank lines split paragraphs, single newlines become hard breaks.
pub fn text_to_adf(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| {
            let mut content = Vec::new();
            for (index, line) in paragraph.lines().enumerate() {
                if index > 0 {
                    content.push(json!({ "type": "hardBreak" }));
                }
                if !line.is_empty() {
                    content.push(json!({ "type": "text", "text": line }));
                }
            }
            json!({ "type": "paragraph", "content": content })
        })
        .collect();
    json!({ "type": "doc", "version": 1, "content": paragraphs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adf_converts_to_markdown() {
        let doc = json!({
            "type": "doc",
            "version": 1,
            "content": [
                { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Repro" }] },
                { "type": "paragraph", "content": [
                    { "type": "text", "text": "Ping " },
                    { "type": "mention", "attrs": { "text": "@Maya" } },
                    { "type": "text", "text": " see " },
                    { "type": "text", "text": "docs", "marks": [{ "type": "link", "attrs": { "href": "https://example.com" } }] },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "now", "marks": [{ "type": "strong" }] },
                    { "type": "text", "text": " run " },
                    { "type": "text", "text": "make", "marks": [{ "type": "code" }, { "type": "strong" }] }
                ]},
                { "type": "bulletList", "content": [
                    { "type": "listItem", "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                        { "type": "orderedList", "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "nested" }] }] }
                        ]}
                    ]},
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                ]},
                { "type": "taskList", "content": [
                    { "type": "taskItem", "attrs": { "state": "DONE" }, "content": [{ "type": "text", "text": "shipped" }] },
                    { "type": "taskItem", "attrs": { "state": "TODO" }, "content": [{ "type": "text", "text": "verify" }] }
                ]},
                { "type": "codeBlock", "attrs": { "language": "sh" }, "content": [{ "type": "text", "text": "npm test" }] },
                { "type": "blockquote", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "quoted" }] }] },
                { "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "id": "x" } }] }
            ]
        });
        assert_eq!(
            adf_to_markdown(&doc),
            "## Repro\n\nPing @Maya see [docs](https://example.com)\n**now** run `make`\n\n- one\n  1. nested\n- two\n\n- [x] shipped\n- [ ] verify\n\n```sh\nnpm test\n```\n\n> quoted"
        );
    }

    #[test]
    fn adf_renders_tables() {
        let cell = |text: &str| json!({ "type": "tableCell", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }] });
        let doc = json!({
            "type": "doc",
            "content": [{ "type": "table", "content": [
                { "type": "tableRow", "content": [cell("a"), cell("b")] },
                { "type": "tableRow", "content": [cell("1"), cell("2|3")] }
            ]}]
        });
        assert_eq!(
            adf_to_markdown(&doc),
            "| a | b |\n| --- | --- |\n| 1 | 2\\|3 |"
        );
    }

    #[test]
    fn text_to_adf_splits_paragraphs_and_lines() {
        assert_eq!(
            text_to_adf("First line\nsecond\n\nNext"),
            json!({
                "type": "doc",
                "version": 1,
                "content": [
                    { "type": "paragraph", "content": [
                        { "type": "text", "text": "First line" },
                        { "type": "hardBreak" },
                        { "type": "text", "text": "second" }
                    ]},
                    { "type": "paragraph", "content": [{ "type": "text", "text": "Next" }] }
                ]
            })
        );
    }

    #[test]
    fn rich_text_handles_plain_strings_and_dates() {
        assert_eq!(rich_text(Some(&json!("  plain  "))), "plain");
        assert_eq!(rich_text(None), "");
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Due " },
                    { "type": "date", "attrs": { "timestamp": "1735689600000" } }
                ]
            }]
        });
        assert_eq!(rich_text(Some(&doc)), "Due 2025-01-01");
    }
}
