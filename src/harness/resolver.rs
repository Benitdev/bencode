use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub binary_path: Option<PathBuf>,
    pub available: bool,
}

/// Where to look for one CLI: its binary name plus install locations relative
/// to `$HOME` that are commonly missing from a Finder-launched app's PATH.
struct BinarySpec {
    id: &'static str,
    name: &'static str,
    binary: &'static str,
    home_dirs: &'static [&'static str],
}

const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

const CLAUDE: BinarySpec = BinarySpec {
    id: "claude",
    name: "Claude Code",
    binary: "claude",
    home_dirs: &[".local/bin", ".claude/local", ".local/share/claude", ".npm-global/bin"],
};
const ANTIGRAVITY: BinarySpec = BinarySpec {
    id: "antigravity",
    name: "Antigravity",
    binary: "agy",
    home_dirs: &[".local/bin", ".antigravity/antigravity/bin"],
};
const CODEX: BinarySpec = BinarySpec {
    id: "codex",
    name: "Codex",
    binary: "codex",
    home_dirs: &[".local/bin", ".cargo/bin", ".npm-global/bin"],
};
const CURSOR: BinarySpec = BinarySpec {
    id: "cursor",
    name: "Cursor Agent",
    binary: "cursor-agent",
    home_dirs: &[".local/bin"],
};
const OPENCODE: BinarySpec = BinarySpec {
    id: "opencode",
    name: "OpenCode",
    binary: "opencode",
    home_dirs: &[".local/bin", ".cargo/bin", ".opencode/bin"],
};

const ALL: [&BinarySpec; 5] = [&CLAUDE, &ANTIGRAVITY, &CODEX, &CURSOR, &OPENCODE];

pub struct HarnessResolver;

impl HarnessResolver {
    /// Probes the filesystem for every known harness. Does disk IO, so call it
    /// once (or on explicit refresh) and cache the result; never from render.
    pub fn discover() -> Vec<HarnessInfo> {
        ALL.iter()
            .map(|spec| {
                let binary_path = resolve(spec);
                HarnessInfo {
                    id: spec.id,
                    name: spec.name,
                    available: binary_path.is_some(),
                    binary_path,
                }
            })
            .collect()
    }

    pub fn resolve_claude() -> Option<PathBuf> {
        resolve(&CLAUDE)
    }

    /// The `agy` CLI. MonoCode talks to `agy_acp_server.par` over ACP instead;
    /// that binary does not accept the print-mode flags used here.
    pub fn resolve_antigravity_cli() -> Option<PathBuf> {
        resolve(&ANTIGRAVITY)
    }

    pub fn resolve_codex() -> Option<PathBuf> {
        resolve(&CODEX)
    }

    pub fn resolve_opencode() -> Option<PathBuf> {
        resolve(&OPENCODE)
    }
}

fn resolve(spec: &BinarySpec) -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let from_home = spec
        .home_dirs
        .iter()
        .filter_map(|dir| home.as_ref().map(|home| home.join(dir)));
    let from_system = SYSTEM_DIRS.iter().map(PathBuf::from);

    from_path
        .chain(from_home)
        .chain(from_system)
        .map(|dir| dir.join(spec.binary))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_lists_every_known_harness_once() {
        let ids: Vec<_> = HarnessResolver::discover().iter().map(|h| h.id).collect();
        assert_eq!(ids, ["claude", "antigravity", "codex", "cursor", "opencode"]);
    }

    #[test]
    fn availability_matches_binary_path() {
        for info in HarnessResolver::discover() {
            assert_eq!(info.available, info.binary_path.is_some(), "{}", info.id);
        }
    }
}
