//! Wiki and Document tools for `bencode --backlog-mcp`: list/get/create/update
//! wiki pages, document trees, document search/list, paginated document reading
//! with comments & replies, and document creation.

use serde_json::Value;

use super::{
    DEFAULT_LIMIT, bool_arg, format_bytes, i64_arg, resolve_project, short_day, slice_chars,
    string_arg,
};
use crate::backlog::{PAGE, call, call_owned, require_config, require_segment, text};

/// Default max characters returned per `backlog_get_document` call.
const DEFAULT_DOCUMENT_CHARS: usize = 32_000;

// ---------------------------------------------------------------------------
// Wiki
// ---------------------------------------------------------------------------

pub(super) fn list_wikis(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut query = vec![("projectIdOrKey", project.id.clone())];
    if let Some(keyword) = string_arg(args, "keyword") {
        query.push(("keyword", keyword));
    }
    let (pages_res, count_res) = std::thread::scope(|scope| {
        let pages = scope.spawn(|| call(&config, None, "/wikis", &query, &[]));
        let count = scope.spawn(|| {
            call(
                &config,
                None,
                "/wikis/count",
                &[("projectIdOrKey", project.id.clone())],
                &[],
            )
        });
        (
            pages.join().unwrap_or_else(|_| Err("Wiki list failed".into())),
            count.join().unwrap_or_else(|_| Err("Wiki count failed".into())),
        )
    });
    let pages_data = pages_res?;
    let rows = pages_data.as_array().map(Vec::as_slice).unwrap_or_default();
    let total = count_res
        .ok()
        .and_then(|v| v.get("count")?.as_i64())
        .unwrap_or(rows.len() as i64);
    if rows.is_empty() {
        return Ok(format!(
            "No matching wiki pages in {} (total in project: {total}).",
            project.key
        ));
    }
    let mut out = format!(
        "# Wiki pages in {} ({} shown / {} total)\n",
        project.key,
        rows.len(),
        total
    );
    for row in rows {
        let id = row.get("id").and_then(Value::as_i64).unwrap_or(0);
        let name = text(row, "name");
        let updated = short_day(&text(row, "updated"));
        let tags: Vec<String> = row
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|t| text(t, "name"))
            .filter(|t| !t.is_empty())
            .collect();
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        out.push_str(&format!("\n- #{id} {name}{tag_str} · updated {updated}"));
    }
    Ok(out)
}

pub(super) fn get_wiki(args: &Value) -> Result<String, String> {
    let wiki_id = i64_arg(args, "wikiId").ok_or("Give the numeric wikiId (from backlog_list_wikis)")?;
    let config = require_config()?;
    let data = call(&config, None, &format!("/wikis/{wiki_id}"), &[], &[])?;
    let name = text(&data, "name");
    let content = text(&data, "content");
    let author = data
        .get("createdUser")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let updater = data
        .get("updatedUser")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let created = short_day(&text(&data, "created"));
    let updated = short_day(&text(&data, "updated"));
    let tags: Vec<String> = data
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| text(t, "name"))
        .filter(|t| !t.is_empty())
        .collect();

    let mut out = format!("# {name} (wiki #{wiki_id})");
    if !tags.is_empty() {
        out.push_str(&format!("\nTags: {}", tags.join(", ")));
    }
    out.push_str(&format!(
        "\nCreated: {created} by {author} · Updated: {updated} by {updater}"
    ));
    if let Some(atts) = data.get("attachments").and_then(Value::as_array)
        && !atts.is_empty()
    {
        out.push_str("\n\n## Attachments\n");
        for att in atts {
            let id = att.get("id").and_then(Value::as_i64).unwrap_or(0);
            let fname = text(att, "name");
            let size = att.get("size").and_then(Value::as_u64).unwrap_or(0);
            out.push_str(&format!("\n- #{id} {fname} ({})", format_bytes(size)));
        }
    }
    out.push_str("\n\n## Content\n\n");
    out.push_str(if content.trim().is_empty() {
        "(empty)"
    } else {
        content.trim()
    });
    Ok(out)
}

pub(super) fn create_wiki(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let name = string_arg(args, "name").ok_or("Give the wiki page a name")?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or("Give the wiki page content")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut form = vec![
        ("projectId".to_string(), project.id.clone()),
        ("name".to_string(), name),
        ("content".to_string(), content.to_string()),
    ];
    if let Some(notify) = bool_arg(args, "mailNotify") {
        form.push(("mailNotify".into(), notify.to_string()));
    }
    let data = call_owned(&config, Some("POST"), "/wikis", &[], &form)?;
    let id = data.get("id").and_then(Value::as_i64).unwrap_or(0);
    Ok(format!(
        "Created wiki page #{id} ({}) in {}.",
        text(&data, "name"),
        project.key
    ))
}

pub(super) fn update_wiki(args: &Value) -> Result<String, String> {
    let wiki_id = i64_arg(args, "wikiId").ok_or("Give the numeric wikiId to update")?;
    let config = require_config()?;
    let mut form: Vec<(String, String)> = Vec::new();
    if let Some(name) = string_arg(args, "name") {
        form.push(("name".into(), name));
    }
    if let Some(content) = args.get("content").and_then(Value::as_str) {
        form.push(("content".into(), content.to_string()));
    }
    if let Some(notify) = bool_arg(args, "mailNotify") {
        form.push(("mailNotify".into(), notify.to_string()));
    }
    if form.is_empty() {
        return Err("Provide name and/or content to update".into());
    }
    let data = call_owned(
        &config,
        Some("PATCH"),
        &format!("/wikis/{wiki_id}"),
        &[],
        &form,
    )?;
    Ok(format!(
        "Updated wiki page #{wiki_id} ({}).",
        text(&data, "name")
    ))
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

pub(super) fn get_document_tree(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let include_trash = bool_arg(args, "includeTrash").unwrap_or(false);
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let data = call(
        &config,
        None,
        "/documents/tree",
        &[("projectIdOrKey", project.id.clone())],
        &[],
    )?;
    Ok(format!(
        "# Document tree for {}\n\n{}",
        project.key,
        format_document_tree(&data, include_trash)
    ))
}

pub(super) fn format_document_tree(data: &Value, include_trash: bool) -> String {
    let mut out = String::new();
    let render_branch = |out: &mut String, title: &str, tree: Option<&Value>| {
        let Some(children) = tree.and_then(|t| t.get("children")).and_then(Value::as_array) else {
            return;
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("## {title}\n"));
        if children.is_empty() {
            out.push_str("\n(empty)");
        } else {
            for node in children {
                write_tree_node(out, node, 0);
            }
        }
    };
    render_branch(&mut out, "Active", data.get("activeTree"));
    if include_trash {
        render_branch(&mut out, "Trash", data.get("trashTree"));
    }
    if out.is_empty() {
        "(no documents)".into()
    } else {
        out
    }
}

fn write_tree_node(out: &mut String, node: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    let id = text(node, "id");
    let title = text(node, "name");
    let title = if title.is_empty() { "(untitled)" } else { &title };
    let emoji = text(node, "emoji");
    if emoji.is_empty() {
        out.push_str(&format!("\n{indent}- {title} (`{id}`)"));
    } else {
        out.push_str(&format!("\n{indent}- {emoji} {title} (`{id}`)"));
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            write_tree_node(out, child, depth + 1);
        }
    }
}

pub(super) fn list_documents(args: &Value) -> Result<String, String> {
    let config = require_config()?;
    let mut query: Vec<(String, String)> = Vec::new();

    // Resolve one or multiple projects
    let mut project_ids: Vec<String> = Vec::new();
    if let Some(single) = string_arg(args, "project") {
        project_ids.push(resolve_project(&single)?.id);
    }
    if let Some(arr) = args.get("projects").and_then(Value::as_array) {
        for item in arr {
            let raw = match item {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            project_ids.push(resolve_project(&raw)?.id);
        }
    }
    for pid in &project_ids {
        query.push(("projectId[]".into(), pid.clone()));
    }
    if let Some(keyword) = string_arg(args, "keyword") {
        query.push(("keyword".into(), keyword));
    }
    if let Some(sort) = string_arg(args, "sort") {
        query.push(("sort".into(), sort));
    }
    if let Some(order) = string_arg(args, "order") {
        query.push(("order".into(), order));
    }
    // Backlog requires `offset` on GET /documents.
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n as usize)
        .clamp(1, PAGE);
    query.push(("offset".into(), offset.to_string()));
    query.push(("count".into(), limit.to_string()));

    let data = call_owned(&config, None, "/documents", &query, &[])?;
    let rows = data.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return Ok("No documents match.".into());
    }
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for doc in rows {
        let id = text(doc, "id");
        let title = text(doc, "title");
        let title = if title.is_empty() { "(untitled)" } else { &title };
        let emoji = text(doc, "emoji");
        let prefix = if emoji.is_empty() {
            String::new()
        } else {
            format!("{emoji} ")
        };
        let updated = short_day(&text(doc, "updated"));
        lines.push(format!("- {prefix}{title} (`{id}`) · updated {updated}"));
    }
    if rows.len() >= limit {
        lines.push(format!(
            "\n(Showing {} documents from offset {offset}; pass offset: {} for more.)",
            rows.len(),
            offset + rows.len() as u64
        ));
    }
    Ok(lines.join("\n"))
}

pub(super) fn get_document(args: &Value) -> Result<String, String> {
    let raw_id = string_arg(args, "documentId").ok_or("Give the documentId UUID")?;
    let doc_id = require_segment(&raw_id, "documentId")?;
    let start = args.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_DOCUMENT_CHARS, |n| (n as usize).max(1));
    let with_comments = bool_arg(args, "comments").unwrap_or(false);
    let config = require_config()?;

    let data = call(&config, None, &format!("/documents/{doc_id}"), &[], &[])?;
    let title = text(&data, "title");
    let title = if title.is_empty() { "(untitled)" } else { &title };
    let emoji = text(&data, "emoji");
    let heading = if emoji.is_empty() {
        format!("# {title}")
    } else {
        format!("# {emoji} {title}")
    };
    let created = short_day(&text(&data, "created"));
    let updated = short_day(&text(&data, "updated"));
    let author = data
        .get("createdUser")
        .map(|u| text(u, "name"))
        .unwrap_or_default();
    let tags: Vec<String> = data
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| text(t, "name"))
        .filter(|t| !t.is_empty())
        .collect();

    let plain = text(&data, "plain");
    let (slice, total_chars, next_start) = slice_chars(&plain, start, limit);

    let mut out = format!("{heading}\nDocument ID: `{doc_id}`");
    if !tags.is_empty() {
        out.push_str(&format!("\nTags: {}", tags.join(", ")));
    }
    out.push_str(&format!("\nCreated: {created} by {author} · Updated: {updated}"));
    if let Some(atts) = data.get("attachments").and_then(Value::as_array)
        && !atts.is_empty()
    {
        out.push_str("\n\n## Attachments\n");
        for att in atts {
            let id = att.get("id").and_then(Value::as_i64).unwrap_or(0);
            let name = text(att, "name");
            let size = att.get("size").and_then(Value::as_u64).unwrap_or(0);
            out.push_str(&format!("\n- #{id} {name} ({})", format_bytes(size)));
        }
    }
    out.push_str("\n\n## Content\n\n");
    if slice.is_empty() {
        out.push_str("(empty)");
    } else {
        out.push_str(&slice);
    }
    if let Some(next) = next_start {
        out.push_str(&format!(
            "\n\n---\n(Truncated at {next}/{total_chars} chars; call backlog_get_document with start: {next} to continue.)"
        ));
    }

    if with_comments {
        let comments_data = call(
            &config,
            None,
            &format!("/documents/{doc_id}/comments"),
            &[],
            &[],
        )?;
        let rows = comments_data
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        out.push_str("\n\n## Comments");
        if rows.is_empty() {
            out.push_str("\n\n(none)");
        }
        for c in rows {
            let c_author = c
                .get("createdUser")
                .map(|u| text(u, "name"))
                .unwrap_or_default();
            let c_when = short_day(&text(c, "created"));
            let c_body = {
                let p = text(c, "plain");
                if p.trim().is_empty() {
                    text(c, "content")
                } else {
                    p
                }
            };
            out.push_str(&format!("\n\n### {c_author} · {c_when}\n\n{}", c_body.trim()));
            if let Some(replies) = c.get("replies").and_then(Value::as_array) {
                for r in replies {
                    let r_author = r
                        .get("createdUser")
                        .map(|u| text(u, "name"))
                        .unwrap_or_default();
                    let r_when = short_day(&text(r, "created"));
                    let r_body = {
                        let p = text(r, "plain");
                        if p.trim().is_empty() {
                            text(r, "content")
                        } else {
                            p
                        }
                    };
                    out.push_str(&format!(
                        "\n\n> **{r_author} · {r_when}**: {}",
                        r_body.trim()
                    ));
                }
            }
        }
    }

    Ok(out)
}

pub(super) fn create_document(args: &Value) -> Result<String, String> {
    let raw = string_arg(args, "project").ok_or("Name the project: its key or id")?;
    let config = require_config()?;
    let project = resolve_project(&raw)?;
    let mut form = vec![("projectId".to_string(), project.id.clone())];
    if let Some(title) = string_arg(args, "title") {
        form.push(("title".into(), title));
    }
    if let Some(content) = args.get("content").and_then(Value::as_str) {
        form.push(("content".into(), content.to_string()));
    }
    if let Some(emoji) = string_arg(args, "emoji") {
        form.push(("emoji".into(), emoji));
    }
    if let Some(parent_id) = string_arg(args, "parentId") {
        form.push(("parentId".into(), parent_id));
    }
    if let Some(add_last) = bool_arg(args, "addLast") {
        form.push(("addLast".into(), add_last.to_string()));
    }
    let data = call_owned(&config, Some("POST"), "/documents", &[], &form)?;
    let id = text(&data, "id");
    let title = text(&data, "title");
    Ok(format!(
        "Created document `{id}` ({}) in {}.",
        if title.is_empty() { "(untitled)" } else { &title },
        project.key
    ))
}
