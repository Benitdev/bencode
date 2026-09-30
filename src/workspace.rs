use std::path::Path;

#[derive(Clone, Debug)]
pub struct SkillItem {
    pub name: &'static str,
    pub description: &'static str,
    pub example: &'static str,
}

pub const BUILTIN_SKILLS: &[SkillItem] = &[
    SkillItem {
        name: "/commit",
        description: "Review staged changes and generate git commit message",
        example: "/commit -m 'feat: ...'",
    },
    SkillItem {
        name: "/review",
        description: "Perform comprehensive code review of current diff",
        example: "/review check security and edge cases",
    },
    SkillItem {
        name: "/test",
        description: "Run test suite and investigate failing cases",
        example: "/test cargo test --all",
    },
    SkillItem {
        name: "/explain",
        description: "Explain architecture, concepts, or specific file logic",
        example: "/explain how GPUI view layout works",
    },
    SkillItem {
        name: "/refactor",
        description: "Refactor code to be cleaner, idiomatic, and maintainable",
        example: "/refactor simplify state machine",
    },
    SkillItem {
        name: "/compact",
        description: "Summarize and compact conversation to save token window",
        example: "/compact",
    },
    SkillItem {
        name: "/help",
        description: "Display all available capabilities, models, and shortcuts",
        example: "/help",
    },
];

pub fn list_workspace_files(root: &Path, max_files: usize) -> Vec<String> {
    let mut results = Vec::new();
    let mut dirs_to_visit = vec![root.to_path_buf()];

    while let Some(current_dir) = dirs_to_visit.pop() {
        if results.len() >= max_files {
            break;
        }

        let entries = match std::fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            if is_ignored(&name_str) {
                continue;
            }

            if path.is_dir() {
                dirs_to_visit.push(path);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    results.push(rel.to_string_lossy().to_string());
                }
                if results.len() >= max_files {
                    break;
                }
            }
        }
    }

    results.sort();
    results
}

/// Directories and dotfiles hidden from the file tree and `@` mentions.
const IGNORED_NAMES: &[&str] = &["target", "node_modules", "dist", "build"];

fn is_ignored(name: &str) -> bool {
    name.starts_with('.') || IGNORED_NAMES.contains(&name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FsNode>,
}

/// Directory tree down to `max_depth`, folders first, case-insensitive order.
pub fn scan_directory(dir: &Path, max_depth: usize) -> Vec<FsNode> {
    if max_depth == 0 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut nodes: Vec<FsNode> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_ignored(&name) {
                return None;
            }
            let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
            let children = if is_dir { scan_directory(&entry.path(), max_depth - 1) } else { Vec::new() };
            Some(FsNode { name, path: entry.path().to_string_lossy().to_string(), is_dir, children })
        })
        .collect();
    nodes.sort_by_key(|node| (!node.is_dir, node.name.to_lowercase()));
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_directory_lists_folders_first_and_skips_ignored() {
        let root = std::env::temp_dir().join(format!("bencode-ws-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        std::fs::write(root.join("A.md"), "").unwrap();
        std::fs::write(root.join(".env"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();

        let names: Vec<_> = scan_directory(&root, 2).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["src", "A.md", "b.txt"]);
        assert_eq!(list_workspace_files(&root, 10), ["A.md", "b.txt", "src/main.rs"]);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
