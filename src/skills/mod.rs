//! Agent skills: `SKILL.md` folders discovered the way MonoCode does
//! (`src-tauri/src/skills.rs`) and injected into a prompt when the user types
//! `/name` (`features/skills/model/skills.ts`).
//!
//! Discovery reads the disk; call it on the background executor and keep the
//! result in `Integrations`.

mod inject;

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub use inject::{inject_skill_prompt, skill_names_in_text};

const MAX_SKILLS: usize = 300;
const MAX_FRONTMATTER_BYTES: u64 = 16 * 1024;
const MAX_NAME_LEN: usize = 64;

/// MonoCode's one built-in skill.
pub const CREATE_SKILL_NAME: &str = "create-skill";
const CREATE_SKILL_BODY: &str = include_str!("create_skill.md");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// `SKILL.md` path; empty for the built-in skill.
    pub path: String,
    /// `project`, `user` or `builtin`.
    pub scope: &'static str,
    /// Folder family it came from: `agents`, `claude`, `codex`, … or `monocode`.
    pub source: &'static str,
}

impl Skill {
    fn builtin() -> Self {
        Self {
            name: CREATE_SKILL_NAME.to_string(),
            description: "Create a skill as a SKILL.md in .agents/skills.".to_string(),
            path: String::new(),
            scope: "builtin",
            source: "monocode",
        }
    }

    /// The text injected for `/name`, or a note when the file is unreadable.
    pub fn body(&self) -> String {
        if self.scope == "builtin" {
            return CREATE_SKILL_BODY.to_string();
        }
        std::fs::read_to_string(&self.path).unwrap_or_else(|err| {
            log::warn!("could not read skill {}: {err}", self.path);
            format!(
                "Skill \"{}\" could not be read from {}.",
                self.name, self.path
            )
        })
    }
}

/// Native harness folders, searched after `.agents/skills`.
const HARNESS_DIRS: [(&str, &str); 9] = [
    (".claude/skills", "claude"),
    (".cursor/skills", "cursor"),
    (".codex/skills", "codex"),
    (".opencode/skills", "opencode"),
    (".pi/skills", "pi"),
    (".omp/skills", "omp"),
    (".fx/skills", "fx"),
    (".grok/skills", "grok"),
    (".hermes/skills", "hermes"),
];

type Root = (PathBuf, &'static str, &'static str);

/// Skill roots in priority order: for a duplicate name the first wins.
fn skill_roots(project: &Path, home: Option<&Path>) -> Vec<Root> {
    let mut roots = vec![(project.join(".agents/skills"), "project", "agents")];
    if let Some(home) = home {
        roots.push((home.join(".agents/skills"), "user", "agents"));
    }
    for (dir, source) in HARNESS_DIRS {
        roots.push((project.join(dir), "project", source));
        if let Some(home) = home {
            roots.push((home.join(dir), "user", source));
        }
    }
    if let Some(home) = home {
        roots.push((home.join(".pi/agent/skills"), "user", "pi"));
        roots.push((home.join(".omp/agent/skills"), "user", "omp"));
        roots.push((
            home.join(".gemini/antigravity/skills"),
            "user",
            "antigravity",
        ));
    }
    roots
}

/// Skills visible in `project`, sorted by name, the built-in one included.
/// Disabled paths are dropped before duplicates are resolved, so a
/// lower-priority skill with the same name can take over.
pub fn list_skills(project: &Path, home: Option<&Path>, disabled: &HashSet<String>) -> Vec<Skill> {
    let mut by_name: HashMap<String, Skill> = HashMap::new();
    let mut seen_roots = HashSet::new();
    for (root, scope, source) in skill_roots(project, home) {
        let key = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if !seen_roots.insert(key) {
            continue;
        }
        for skill in scan_root(&root, scope, source) {
            if by_name.len() >= MAX_SKILLS {
                break;
            }
            if !disabled.contains(&skill.path) {
                by_name.entry(skill.name.clone()).or_insert(skill);
            }
        }
    }
    by_name
        .entry(CREATE_SKILL_NAME.to_string())
        .or_insert_with(Skill::builtin);
    let mut skills: Vec<Skill> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn scan_root(root: &Path, scope: &'static str, source: &'static str) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| read_skill(&entry.path(), scope, source))
        .collect()
}

fn read_skill(dir: &Path, scope: &'static str, source: &'static str) -> Option<Skill> {
    let folder = dir.file_name()?.to_str()?;
    if !dir.is_dir() || folder.starts_with('.') || folder == "skills-cursor" {
        return None;
    }
    let file = ["SKILL.md", "skill.md"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())?;
    let mut text = String::new();
    std::fs::File::open(&file)
        .ok()?
        .take(MAX_FRONTMATTER_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let fallback = slug_name(folder);
    if fallback.is_empty() {
        return None;
    }
    let (name, description) = parse_frontmatter(&text, &fallback);
    Some(Skill {
        name,
        description,
        path: file.to_string_lossy().into_owned(),
        scope,
        source,
    })
}

/// `name` and `description` from YAML frontmatter, including folded
/// (`>`) and literal (`|`) descriptions. An invalid name falls back to the
/// folder's slug.
fn parse_frontmatter(text: &str, fallback: &str) -> (String, String) {
    let trimmed = text.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (fallback.to_string(), String::new());
    };
    let rest = rest.trim_start_matches('\r');
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let yaml = &rest[..rest.find("\n---").unwrap_or(rest.len())];

    let mut name = None;
    let mut description = String::new();
    let mut block: Option<char> = None; // separator inside a > or | scalar
    for line in yaml.lines() {
        if let Some(separator) = block {
            if line.starts_with([' ', '\t']) {
                let piece = line.trim();
                if !piece.is_empty() {
                    if !description.is_empty() {
                        description.push(separator);
                    }
                    description.push_str(piece);
                }
                continue;
            }
            block = None;
        }
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if value.starts_with(['>', '|']) {
                block = Some(if value.starts_with('>') { ' ' } else { '\n' });
                description.clear();
            } else {
                description = unquote(value);
            }
        }
    }
    let name = name
        .filter(|n| is_valid_skill_name(n))
        .unwrap_or_else(|| fallback.to_string());
    (name, description.trim().to_string())
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let quoted = value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')));
    if quoted {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// `^[a-z0-9]+(-[a-z0-9]+)*$`, at most 64 characters.
pub fn is_valid_skill_name(name: &str) -> bool {
    let part_ok = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    };
    !name.is_empty() && name.len() <= MAX_NAME_LEN && name.split('-').all(part_ok)
}

/// MonoCode `blankSkillMarkdown`: the starter `SKILL.md` "New skill" writes.
pub fn blank_skill_markdown(name: &str) -> String {
    let title = name
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    let words = name.replace('-', " ");
    format!(
        "---\nname: {name}\ndescription: {title}. Use when the user asks to {words}.\n---\n\n# {title}\n\n## Instructions\n\n"
    )
}

/// MonoCode `createBlankSkill`: writes `.agents/skills/<name>/SKILL.md`
/// under `root` (the project, or home for a personal skill) and returns its
/// path. Never overwrites an existing skill.
pub fn create_blank_skill(root: &Path, name: &str) -> std::io::Result<PathBuf> {
    let name = slug_name(name);
    if !is_valid_skill_name(&name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Use a lowercase name with letters, numbers, and hyphens.",
        ));
    }
    let dir = root.join(".agents/skills").join(&name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                std::io::Error::new(
                    err.kind(),
                    format!("A skill named \"{name}\" already exists."),
                )
            } else {
                err
            }
        })?;
    std::io::Write::write_all(&mut file, blank_skill_markdown(&name).as_bytes())?;
    Ok(path)
}

/// Folder name as a skill name: lower-case alphanumerics joined by dashes
/// (MonoCode `slugSkillName`).
pub fn slug_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(MAX_NAME_LEN);
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not clean {}: {err}", self.0.display());
            }
        }
    }

    fn tmp(label: &str) -> Tmp {
        let dir =
            std::env::temp_dir().join(format!("bencode-skills-{label}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    fn write_skill(root: &Path, folder: &str, body: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn frontmatter_reads_quoted_and_folded_fields() {
        let text = "---\nname: 'review-pr'\ndescription: >\n  Review a\n  pull request\n---\nbody";
        let (name, desc) = parse_frontmatter(text, "x");
        assert_eq!(name, "review-pr");
        assert_eq!(desc, "Review a pull request");
        let (name, desc) = parse_frontmatter("no frontmatter", "folder-name");
        assert_eq!((name.as_str(), desc.as_str()), ("folder-name", ""));
        let (name, _) = parse_frontmatter("---\nname: Bad Name\n---", "fallback");
        assert_eq!(name, "fallback");
    }

    #[test]
    fn names_and_slugs_follow_monocode_rules() {
        assert!(is_valid_skill_name("create-skill"));
        assert!(!is_valid_skill_name("Create"));
        assert!(!is_valid_skill_name("a--b"));
        assert!(!is_valid_skill_name("-a"));
        assert_eq!(slug_name("My Cool_Skill!"), "my-cool-skill");
        assert!(blank_skill_markdown("fix-ci").starts_with(
            "---\nname: fix-ci\ndescription: Fix Ci. Use when the user asks to fix ci.\n---\n\n# Fix Ci\n"
        ));
    }

    #[test]
    fn project_agents_folder_wins_and_disabled_paths_fall_through() {
        let project = tmp("project");
        let home = tmp("home");
        let agents = project.0.join(".agents/skills");
        write_skill(
            &agents,
            "deploy",
            "---\nname: deploy\ndescription: project\n---",
        );
        let claude = home.0.join(".claude/skills");
        write_skill(
            &claude,
            "deploy",
            "---\nname: deploy\ndescription: user claude\n---",
        );

        let skills = list_skills(&project.0, Some(&home.0), &HashSet::new());
        let deploy = skills.iter().find(|s| s.name == "deploy").unwrap();
        assert_eq!((deploy.scope, deploy.source), ("project", "agents"));
        assert!(skills.iter().any(|s| s.name == CREATE_SKILL_NAME));

        let disabled: HashSet<String> = [deploy.path.clone()].into_iter().collect();
        let skills = list_skills(&project.0, Some(&home.0), &disabled);
        let deploy = skills.iter().find(|s| s.name == "deploy").unwrap();
        assert_eq!(deploy.description, "user claude");
    }

    #[test]
    fn new_skill_is_written_once() {
        let root = tmp("create");
        let path = create_blank_skill(&root.0, "Fix CI").unwrap();
        assert_eq!(path, root.0.join(".agents/skills/fix-ci/SKILL.md"));
        let skills = list_skills(&root.0, None, &HashSet::new());
        assert!(skills.iter().any(|s| s.name == "fix-ci"));
        assert!(create_blank_skill(&root.0, "fix-ci").is_err());
        assert!(create_blank_skill(&root.0, "!!!").is_err());
    }
}
