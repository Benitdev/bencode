//! MonoCode `GitChangesPanel` tree view: changed files nested by folder,
//! status rolled up.

use crate::git::{GitFileChange, GitFileStatus};

pub(super) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(super) fn dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// MonoCode `ChangeDir`: changed files nested under their folders, each
/// folder with the status its files share (or none when they differ).
#[derive(Debug, Default)]
pub struct ChangeDir {
    pub name: String,
    pub path: String,
    pub dirs: Vec<ChangeDir>,
    pub files: Vec<GitFileChange>,
    pub status: Option<GitFileStatus>,
}

/// MonoCode `buildChangeTree` + `sortChangeDir`.
pub fn build_tree(files: &[GitFileChange]) -> ChangeDir {
    fn insert(dir: &mut ChangeDir, segments: &[&str], file: &GitFileChange) {
        match segments {
            [] | [_] => dir.files.push(file.clone()),
            [first, rest @ ..] => {
                let path = if dir.path.is_empty() {
                    first.to_string()
                } else {
                    format!("{}/{first}", dir.path)
                };
                let at = match dir.dirs.iter().position(|d| d.path == path) {
                    Some(at) => at,
                    None => {
                        dir.dirs.push(ChangeDir {
                            name: first.to_string(),
                            path,
                            ..ChangeDir::default()
                        });
                        dir.dirs.len() - 1
                    }
                };
                insert(&mut dir.dirs[at], rest, file);
            }
        }
    }
    fn sort(dir: &mut ChangeDir) -> Option<GitFileStatus> {
        dir.dirs.sort_by(|a, b| a.name.cmp(&b.name));
        dir.files
            .sort_by(|a, b| basename(&a.path).cmp(basename(&b.path)));
        let mut status: Option<GitFileStatus> = None;
        let mut mixed = false;
        let mut merge = |next: Option<GitFileStatus>| match (next, &status) {
            (None, _) => mixed = true,
            (Some(next), None) => status = Some(next),
            (Some(next), Some(current)) if &next != current => mixed = true,
            _ => {}
        };
        for child in &mut dir.dirs {
            merge(sort(child));
        }
        for file in &dir.files {
            merge(Some(file.status.clone()));
        }
        dir.status = if mixed { None } else { status };
        dir.status.clone()
    }
    let mut root = ChangeDir::default();
    for file in files {
        let segments: Vec<&str> = file.path.split('/').collect();
        insert(&mut root, &segments, file);
    }
    sort(&mut root);
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, status: GitFileStatus) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            status,
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn the_tree_nests_and_rolls_status_up() {
        let files = [
            change("src/app/b.rs", GitFileStatus::Modified),
            change("src/app/a.rs", GitFileStatus::Modified),
            change("src/ui/new.rs", GitFileStatus::Untracked),
            change("README.md", GitFileStatus::Modified),
        ];
        let root = build_tree(&files);
        assert_eq!(root.files[0].path, "README.md");
        let src = &root.dirs[0];
        assert_eq!(src.name, "src");
        assert_eq!(src.status, None, "mixed below");
        let app = &src.dirs[0];
        assert_eq!((app.path.as_str(), app.status.clone()), ("src/app", Some(GitFileStatus::Modified)));
        assert_eq!(app.files[0].path, "src/app/a.rs");
        assert_eq!(src.dirs[1].status, Some(GitFileStatus::Untracked));
    }
}
