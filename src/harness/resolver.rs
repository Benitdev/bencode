use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub binary_path: Option<PathBuf>,
    pub available: bool,
}

pub struct HarnessResolver;

impl HarnessResolver {
    /// Discovers all available coding agent harnesses on the local machine
    pub fn discover() -> Vec<HarnessInfo> {
        vec![
            HarnessInfo {
                id: "claude",
                name: "Claude Code",
                binary_path: Self::resolve_claude(),
                available: Self::resolve_claude().is_some(),
            },
            HarnessInfo {
                id: "antigravity",
                name: "Antigravity",
                binary_path: Self::resolve_antigravity(),
                available: Self::resolve_antigravity().is_some(),
            },
            HarnessInfo {
                id: "codex",
                name: "Codex",
                binary_path: Self::resolve_codex(),
                available: Self::resolve_codex().is_some(),
            },
            HarnessInfo {
                id: "cursor",
                name: "Cursor Agent",
                binary_path: Self::resolve_cursor(),
                available: Self::resolve_cursor().is_some(),
            },
            HarnessInfo {
                id: "opencode",
                name: "OpenCode",
                binary_path: Self::resolve_opencode(),
                available: Self::resolve_opencode().is_some(),
            },
        ]
    }

    /// Resolve Claude Code CLI binary path
    pub fn resolve_claude() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut candidates = Vec::new();

        if let Some(shell_path) = which("claude") {
            candidates.push(shell_path);
        }

        if let Some(home) = &home {
            candidates.push(home.join(".local/bin/claude"));
            candidates.push(home.join(".claude/local/claude"));
            candidates.push(home.join(".local/share/claude/claude"));
            candidates.push(home.join(".npm-global/bin/claude"));
            candidates.push(home.join(".cargo/bin/claude"));
        }

        candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
        candidates.push(PathBuf::from("/usr/local/bin/claude"));
        candidates.push(PathBuf::from("/usr/bin/claude"));

        first_existing_binary(candidates)
    }

    /// Resolve Antigravity binary path
    pub fn resolve_antigravity() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut candidates = Vec::new();

        if let Some(shell_path) = which("agy") {
            candidates.push(shell_path);
        }

        if let Some(home) = &home {
            candidates.push(home.join(".local/bin/agy_acp_server.par"));
            candidates.push(home.join(".local/share/agy-acp/agy_acp_server.par"));
            candidates.push(home.join(".local/bin/agy"));
        }

        candidates.push(PathBuf::from("/opt/homebrew/bin/agy"));
        candidates.push(PathBuf::from("/usr/local/bin/agy"));

        first_existing_binary(candidates)
    }

    /// Resolve Codex CLI binary path
    pub fn resolve_codex() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut candidates = Vec::new();

        if let Some(shell_path) = which("codex") {
            candidates.push(shell_path);
        }

        if let Some(home) = &home {
            candidates.push(home.join(".local/bin/codex"));
            candidates.push(home.join(".cargo/bin/codex"));
            candidates.push(home.join(".npm-global/bin/codex"));
        }

        candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
        candidates.push(PathBuf::from("/usr/local/bin/codex"));

        first_existing_binary(candidates)
    }

    /// Resolve Cursor Agent binary path
    pub fn resolve_cursor() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut candidates = Vec::new();

        if let Some(shell_path) = which("cursor-agent") {
            candidates.push(shell_path);
        }

        if let Some(home) = &home {
            candidates.push(home.join(".local/bin/cursor-agent"));
        }

        candidates.push(PathBuf::from("/opt/homebrew/bin/cursor-agent"));
        candidates.push(PathBuf::from("/usr/local/bin/cursor-agent"));

        first_existing_binary(candidates)
    }

    /// Resolve OpenCode binary path
    pub fn resolve_opencode() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let mut candidates = Vec::new();

        if let Some(shell_path) = which("opencode") {
            candidates.push(shell_path);
        }

        if let Some(home) = &home {
            candidates.push(home.join(".local/bin/opencode"));
            candidates.push(home.join(".cargo/bin/opencode"));
        }

        candidates.push(PathBuf::from("/opt/homebrew/bin/opencode"));
        candidates.push(PathBuf::from("/usr/local/bin/opencode"));

        first_existing_binary(candidates)
    }
}

fn which(binary_name: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_existing_binary(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    for p in candidates {
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
