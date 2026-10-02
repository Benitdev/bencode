use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_workspace_files_skips_ignored_and_dotfiles() {
        let root = std::env::temp_dir().join(format!("bencode-ws-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        std::fs::write(root.join("A.md"), "").unwrap();
        std::fs::write(root.join(".env"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();

        assert_eq!(
            list_workspace_files(&root, 10),
            ["A.md", "b.txt", "src/main.rs"]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}
