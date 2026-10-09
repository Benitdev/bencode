use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub binary_path: Option<PathBuf>,
    pub available: bool,
}

/// Where to look for one CLI: its binary name plus install locations relative
/// to `$HOME` that are commonly missing from a Finder-launched app's PATH,
/// and, last, copies an app bundles but never puts on PATH (tried under
/// `$HOME` and then under `/`).
struct BinarySpec {
    id: &'static str,
    name: &'static str,
    binary: &'static str,
    home_dirs: &'static [&'static str],
    bundled: &'static [&'static str],
}

const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

const CLAUDE: BinarySpec = BinarySpec {
    id: "claude",
    name: "Claude Code",
    binary: "claude",
    home_dirs: &[
        ".local/bin",
        ".claude/local",
        ".local/share/claude",
        ".npm-global/bin",
    ],
    bundled: &[],
};
const ANTIGRAVITY: BinarySpec = BinarySpec {
    id: "antigravity",
    name: "Antigravity",
    binary: "agy",
    home_dirs: &[".local/bin", ".antigravity/antigravity/bin"],
    bundled: &[],
};
const CODEX: BinarySpec = BinarySpec {
    id: "codex",
    name: "Codex",
    binary: "codex",
    home_dirs: &[
        ".local/bin",
        ".cargo/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".volta/bin",
        "n/bin",
    ],
    // The Codex app's CLI, and the one the ChatGPT app ships for its Codex.
    bundled: &[
        "Applications/Codex.app/Contents/Resources/codex",
        "Applications/ChatGPT.app/Contents/Resources/codex",
    ],
};
const CURSOR: BinarySpec = BinarySpec {
    id: "cursor",
    name: "Cursor Agent",
    binary: "cursor-agent",
    home_dirs: &[".local/bin"],
    bundled: &[],
};
const OPENCODE: BinarySpec = BinarySpec {
    id: "opencode",
    name: "OpenCode",
    binary: "opencode",
    home_dirs: &[".local/bin", ".cargo/bin", ".opencode/bin"],
    bundled: &[],
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

    /// Codex is often installed twice: a package-manager CLI, and the copy
    /// the Codex or ChatGPT app bundles and keeps current. The server offers
    /// its newest models only to recent CLIs, and an old one lists models a
    /// ChatGPT account can no longer run, so the newest copy wins; on a tie,
    /// the earlier one (PATH first, the app bundles last, as in MonoCode).
    /// Asking each copy its version can take a few hundred ms, so a lone
    /// copy is used without asking, and the pick is kept until that file
    /// goes away (or until Codex is installed, when none was found); the
    /// startup model probe makes it off the UI thread.
    pub fn resolve_codex() -> Option<PathBuf> {
        let mut pick = CODEX_PICK.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(path) = pick.as_ref().filter(|path| path.is_file()) {
            return Some(path.clone());
        }
        *pick = pick_codex();
        pick.clone()
    }

    /// The copy `resolve_codex` last picked. Does no IO.
    pub fn resolved_codex() -> Option<PathBuf> {
        CODEX_PICK
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn resolve_opencode() -> Option<PathBuf> {
        resolve(&OPENCODE)
    }
}

static CODEX_PICK: Mutex<Option<PathBuf>> = Mutex::new(None);

fn resolve(spec: &'static BinarySpec) -> Option<PathBuf> {
    candidates(spec).next()
}

fn pick_codex() -> Option<PathBuf> {
    let found: Vec<PathBuf> = candidates(&CODEX).collect();
    if found.len() < 2 {
        return found.into_iter().next();
    }
    let found: Vec<_> = found
        .into_iter()
        .map(|path| {
            let version = cli_version(&path);
            (path, version)
        })
        .collect();
    let pick = newest(found.iter().cloned());
    log::info!("Codex copies {found:?}, using {pick:?}");
    pick
}

/// Every copy of `spec`'s binary on disk, in lookup order, without repeats.
/// Lazy, so taking the first one stops at the first hit.
fn candidates(spec: &'static BinarySpec) -> impl Iterator<Item = PathBuf> {
    let from_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let home_for_bundled = home.clone();
    let from_home = spec
        .home_dirs
        .iter()
        .filter_map(move |dir| home.as_ref().map(|home| home.join(dir)));
    let from_system = SYSTEM_DIRS.iter().map(PathBuf::from);
    let bundled = spec.bundled.iter().flat_map(move |path| {
        home_for_bundled
            .iter()
            .map(|home| home.join(path))
            .chain([Path::new("/").join(path)])
            .collect::<Vec<_>>()
    });

    let mut seen = HashSet::new();
    from_path
        .chain(from_home)
        .chain(from_system)
        .map(|dir| dir.join(spec.binary))
        .chain(bundled)
        .filter(move |candidate| candidate.is_file() && seen.insert(candidate.clone()))
}

/// What `<binary> --version` reports, as numbers; `None` when it fails.
fn cli_version(path: &Path) -> Option<Vec<u64>> {
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_version(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => {
            log::debug!("{} --version exited with {}", path.display(), output.status);
            None
        }
        Err(err) => {
            log::debug!("{} --version: {err}", path.display());
            None
        }
    }
}

/// `codex-cli 0.153.1` → `[0, 153, 1]`. A pre-release or build tag is
/// dropped, so a pre-release ties with its release.
fn parse_version(text: &str) -> Option<Vec<u64>> {
    let word = text.split_whitespace().last()?;
    let core = word.trim_start_matches('v').split(['-', '+']).next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

/// The path with the highest version; the earlier one on a tie. A copy whose
/// version could not be read loses to any that reported one.
fn newest(found: impl IntoIterator<Item = (PathBuf, Option<Vec<u64>>)>) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, Option<Vec<u64>>)> = None;
    for (path, version) in found {
        if best.as_ref().is_none_or(|(_, top)| version > *top) {
            best = Some((path, version));
        }
    }
    best.map(|(path, _)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_lists_every_known_harness_once() {
        let ids: Vec<_> = HarnessResolver::discover().iter().map(|h| h.id).collect();
        assert_eq!(
            ids,
            ["claude", "antigravity", "codex", "cursor", "opencode"]
        );
    }

    #[test]
    fn versions_parse_from_cli_output() {
        assert_eq!(parse_version("codex-cli 0.153.1\n"), Some(vec![0, 153, 1]));
        assert_eq!(
            parse_version("codex-cli 0.154.0-alpha.3"),
            Some(vec![0, 154, 0])
        );
        assert_eq!(parse_version("v1.2"), Some(vec![1, 2]));
        assert_eq!(parse_version("codex-cli dev"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn newest_copy_wins_and_ties_keep_lookup_order() {
        let path = |p: &str| PathBuf::from(p);
        let picked = newest([
            (path("/volta/codex"), Some(vec![0, 143, 0])),
            (path("/ChatGPT.app/codex"), Some(vec![0, 153, 1])),
        ]);
        assert_eq!(picked, Some(path("/ChatGPT.app/codex")));

        let tie = newest([
            (path("/usr/local/bin/codex"), Some(vec![0, 153, 1])),
            (path("/Codex.app/codex"), Some(vec![0, 153, 1])),
        ]);
        assert_eq!(tie, Some(path("/usr/local/bin/codex")));

        let unread = newest([
            (path("/broken/codex"), None),
            (path("/Codex.app/codex"), Some(vec![0, 1, 0])),
        ]);
        assert_eq!(unread, Some(path("/Codex.app/codex")));
        assert_eq!(
            newest([(path("/only/codex"), None)]),
            Some(path("/only/codex"))
        );
        assert_eq!(newest([]), None);
    }

    #[test]
    fn availability_matches_binary_path() {
        for info in HarnessResolver::discover() {
            assert_eq!(info.available, info.binary_path.is_some(), "{}", info.id);
        }
    }
}
