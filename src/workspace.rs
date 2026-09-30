use std::path::{Path, PathBuf};

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

            if name_str.starts_with('.')
                || name_str == "target"
                || name_str == "node_modules"
                || name_str == "dist"
                || name_str == "build"
            {
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
