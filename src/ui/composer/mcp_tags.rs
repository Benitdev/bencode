//! MonoCode `mcpPicker.ts`: the `@mcp/name` tags `/mcp` puts in the prompt,
//! which servers the picker offers, and the "MCP context" line a tagged turn
//! is sent with.

use std::collections::HashMap;
use std::ops::Range;

use crate::mcp::McpConnection;

/// MonoCode `MCP_PROVIDER_LABELS`.
pub fn provider_label(provider: &str) -> &str {
    match provider {
        "claude" => "Claude Code",
        "claude_desktop" => "Claude Desktop",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        "bencode" => "BenCode",
        other => other,
    }
}

/// A server picked for this message and the token naming it in the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTag {
    pub server: McpConnection,
    pub token: String,
}

impl McpTag {
    fn names(&self, server: &McpConnection) -> bool {
        let own = &self.server;
        own.provider == server.provider
            && own.name == server.name
            && own.scope == server.scope
            && own.config_path == server.config_path
    }
}

/// The tag already made for `server`, if any.
pub fn tag_for<'a>(tags: &'a [McpTag], server: &McpConnection) -> Option<&'a McpTag> {
    tags.iter().find(|tag| tag.names(server))
}

/// MonoCode `newMcpTag`: `@mcp/name`, made longer only when another server
/// already holds that token.
pub fn new_mcp_tag(server: &McpConnection, existing: &[McpTag]) -> McpTag {
    let used = |token: &str| existing.iter().any(|tag| tag.token == token);
    let candidates = [
        format!("@mcp/{}", server.name),
        format!("@mcp/{}/{}", server.provider, server.name),
        format!("@mcp/{}/{}/{}", server.provider, server.scope, server.name),
    ];
    let token = match candidates.iter().find(|c| !used(c)) {
        Some(free) => free.clone(),
        None => (2..)
            .map(|n| format!("{}-{n}", candidates[2]))
            .find(|c| !used(c))
            .expect("an unused suffix exists"),
    };
    McpTag {
        server: server.clone(),
        token,
    }
}

fn opens_tag(c: char) -> bool {
    c.is_whitespace() || "([{".contains(c)
}

fn closes_tag(c: char) -> bool {
    c.is_whitespace() || ")]}.!?;,".contains(c)
}

/// MonoCode `mcpTagParts`: where the tags stand in `text` as whole words,
/// with the index of each one's tag. The longer token wins a tie.
pub fn mcp_tag_ranges(text: &str, tags: &[McpTag]) -> Vec<(Range<usize>, usize)> {
    let mut hits: Vec<(Range<usize>, usize)> = tags
        .iter()
        .enumerate()
        .flat_map(|(ix, tag)| {
            text.match_indices(tag.token.as_str())
                .map(move |(start, token)| (start..start + token.len(), ix))
        })
        .filter(|(range, _)| {
            text[..range.start]
                .chars()
                .next_back()
                .is_none_or(opens_tag)
                && text[range.end..].chars().next().is_none_or(closes_tag)
        })
        .collect();
    hits.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    let mut end = 0;
    hits.retain(|(range, _)| {
        let keep = range.start >= end;
        if keep {
            end = range.end;
        }
        keep
    });
    hits
}

/// MonoCode `taggedMcpServers`: the servers whose tags are still in `text`.
pub fn tagged_servers<'a>(text: &str, tags: &'a [McpTag]) -> Vec<&'a McpConnection> {
    let present: Vec<usize> = mcp_tag_ranges(text, tags)
        .into_iter()
        .map(|(_, ix)| ix)
        .collect();
    tags.iter()
        .enumerate()
        .filter(|(ix, _)| present.contains(ix))
        .map(|(_, tag)| &tag.server)
        .collect()
}

/// MonoCode `mcpContextText`: the turn as sent, naming the tagged servers.
pub fn mcp_context_text(servers: &[&McpConnection], text: &str) -> String {
    if servers.is_empty() {
        return text.to_string();
    }
    let names = servers
        .iter()
        .map(|server| {
            let quoted = serde_json::to_string(&server.name).unwrap_or_default();
            format!("{quoted} ({})", server.provider)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let plural = if servers.len() == 1 { "" } else { "s" };
    format!(
        "MCP context: Use the configured server{plural} {names} when relevant to this request.\n\n{text}"
    )
}

/// Whether the thread can use a server now; the picker sorts by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Availability {
    Available,
    Authentication,
    Unavailable,
}

/// A picker row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerServer {
    pub server: McpConnection,
    pub availability: Availability,
    pub detail: &'static str,
}

impl PickerServer {
    /// The status text on the row's right.
    pub fn status(&self) -> &'static str {
        match self.availability {
            Availability::Available => "Available",
            Availability::Authentication => "Needs authentication",
            Availability::Unavailable => self.detail,
        }
    }
}

fn mentions_any(status: &str, words: &[&str]) -> bool {
    let status = status.to_lowercase();
    words.iter().any(|word| status.contains(word))
}

/// MonoCode `mcpPickerServers`: the servers matching `query`, usable ones
/// first. Only servers of the thread's own harness can be used; Claude's
/// `claude mcp list` health (`claude_status`) can still rule one out.
pub fn picker_servers(
    connections: &[McpConnection],
    harness: &str,
    claude_status: &HashMap<String, String>,
    query: &str,
) -> Vec<PickerServer> {
    let search = query.trim().to_lowercase();
    let mut rows: Vec<PickerServer> = connections
        .iter()
        .filter(|server| {
            format!(
                "{} {} {}",
                server.name,
                provider_label(&server.provider),
                server.scope
            )
            .to_lowercase()
            .contains(&search)
        })
        .map(|server| {
            let matches = server.provider == harness;
            let status = (server.provider == "claude")
                .then(|| claude_status.get(&server.name))
                .flatten()
                .filter(|_| matches);
            let authentication = status.is_some_and(|s| {
                mentions_any(s, &["auth", "sign in", "sign-in", "login", "log in"])
            });
            let failed = status.is_some_and(|s| {
                mentions_any(
                    s,
                    &["failed", "error", "offline", "unreachable", "disconnected"],
                )
            });
            let (availability, detail) = if !matches {
                (Availability::Unavailable, "Different provider")
            } else if !server.enabled {
                (
                    Availability::Unavailable,
                    "Disabled in provider configuration",
                )
            } else if failed {
                (Availability::Unavailable, "Connection unavailable")
            } else if authentication {
                (Availability::Authentication, "Configured for this provider")
            } else {
                (Availability::Available, "Configured for this provider")
            };
            PickerServer {
                server: server.clone(),
                availability,
                detail,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.availability
            .cmp(&b.availability)
            .then_with(|| a.server.name.cmp(&b.server.name))
            .then_with(|| a.server.provider.cmp(&b.server.provider))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(provider: &str, scope: &str, name: &str) -> McpConnection {
        McpConnection {
            provider: provider.into(),
            name: name.into(),
            scope: scope.into(),
            config_path: format!("/{provider}/{scope}.json"),
            transport: "stdio".into(),
            enabled: true,
        }
    }

    #[test]
    fn tags_grow_only_on_a_clash() {
        let first = new_mcp_tag(&server("claude", "user", "docs"), &[]);
        assert_eq!(first.token, "@mcp/docs");
        let second = new_mcp_tag(
            &server("cursor", "user", "docs"),
            std::slice::from_ref(&first),
        );
        assert_eq!(second.token, "@mcp/cursor/docs");
        let third = new_mcp_tag(
            &server("cursor", "project", "docs"),
            &[first.clone(), second.clone()],
        );
        assert_eq!(third.token, "@mcp/cursor/project/docs");
        let fourth = new_mcp_tag(
            &server("cursor", "project", "docs"),
            &[first, second, third],
        );
        assert_eq!(fourth.token, "@mcp/cursor/project/docs-2");
    }

    #[test]
    fn tags_count_only_as_whole_words() {
        let tags = vec![
            new_mcp_tag(&server("claude", "user", "docs"), &[]),
            McpTag {
                server: server("claude", "user", "db"),
                token: "@mcp/docs/db".into(),
            },
        ];
        let text = "ask (@mcp/docs), then @mcp/docs/db and x@mcp/docs or @mcp/docsy";
        let found: Vec<_> = mcp_tag_ranges(text, &tags)
            .into_iter()
            .map(|(range, ix)| (&text[range], ix))
            .collect();
        assert_eq!(found, vec![("@mcp/docs", 0), ("@mcp/docs/db", 1)]);
        assert_eq!(tagged_servers("plain text", &tags).len(), 0);
    }

    #[test]
    fn context_line_names_each_tagged_server() {
        let docs = server("claude", "user", "docs");
        let db = server("claude", "user", "db");
        assert_eq!(mcp_context_text(&[], "hi"), "hi");
        assert_eq!(
            mcp_context_text(&[&docs], "hi"),
            "MCP context: Use the configured server \"docs\" (claude) when relevant to this request.\n\nhi"
        );
        assert!(mcp_context_text(&[&docs, &db], "hi").starts_with(
            "MCP context: Use the configured servers \"docs\" (claude), \"db\" (claude)"
        ));
    }

    #[test]
    fn usable_servers_come_first() {
        let mut off = server("claude", "user", "aaa");
        off.enabled = false;
        let servers = vec![
            server("cursor", "user", "cursor-one"),
            off,
            server("claude", "user", "login"),
            server("claude", "user", "zeta"),
            server("claude", "user", "broken"),
        ];
        let status = HashMap::from([
            ("login".to_string(), "⚠ Needs authentication".to_string()),
            ("broken".to_string(), "✗ Failed to connect".to_string()),
            ("zeta".to_string(), "✓ Connected".to_string()),
        ]);
        let rows = picker_servers(&servers, "claude", &status, "");
        let order: Vec<_> = rows
            .iter()
            .map(|r| (r.server.name.as_str(), r.status()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("zeta", "Available"),
                ("login", "Needs authentication"),
                ("aaa", "Disabled in provider configuration"),
                ("broken", "Connection unavailable"),
                ("cursor-one", "Different provider"),
            ]
        );
        let found = picker_servers(&servers, "claude", &status, "CURSOR");
        assert_eq!(found.len(), 1);
    }
}
