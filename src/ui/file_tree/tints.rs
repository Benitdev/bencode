//! The Explorer's git tints (MonoCode `useGitFileStatuses` and
//! `GIT_STATUS_COLOR`).

use std::collections::HashMap;

use gpui::{Hsla, rgb};

use super::name;
use crate::git::GitFileStatus;
use crate::ui::appearance::DiffColors;

/// MonoCode `useGitFileStatuses`: each changed file's status, and each
/// folder's most important status below it (modified, then deleted, then
/// added / untracked).
pub(super) struct GitTints {
    files: HashMap<String, GitFileStatus>,
    dirs: HashMap<String, GitFileStatus>,
}

fn status_rank(status: &GitFileStatus) -> u8 {
    match status {
        GitFileStatus::Modified => 3,
        GitFileStatus::Deleted => 2,
        GitFileStatus::Added | GitFileStatus::Untracked => 1,
        GitFileStatus::Renamed => 0,
    }
}

impl GitTints {
    pub(super) fn new(changes: impl Iterator<Item = (String, GitFileStatus)>) -> Self {
        let mut files = HashMap::new();
        let mut dirs: HashMap<String, GitFileStatus> = HashMap::new();
        for (path, status) in changes {
            if status_rank(&status) == 0 {
                continue;
            }
            let mut dir = name::parent_of(&path);
            while !dir.is_empty() {
                let stronger = dirs
                    .get(&dir)
                    .is_none_or(|current| status_rank(&status) > status_rank(current));
                if stronger {
                    dirs.insert(dir.clone(), status.clone());
                }
                dir = name::parent_of(&dir);
            }
            files.insert(path, status);
        }
        Self { files, dirs }
    }

    /// MonoCode `GIT_STATUS_COLOR` (renamed files stay plain):
    /// `text-amber-400`, and `text-diff-add-fg` / `text-diff-del-fg` of the
    /// chosen diff palette.
    pub(super) fn color(&self, rel: &str, is_dir: bool, diff: DiffColors) -> Option<Hsla> {
        let status = if is_dir {
            self.dirs.get(rel)
        } else {
            self.files.get(rel)
        }?;
        Some(match status {
            GitFileStatus::Modified => rgb(0xfbbf24).into(),
            GitFileStatus::Added | GitFileStatus::Untracked => diff.add_fg,
            GitFileStatus::Deleted => diff.del_fg,
            GitFileStatus::Renamed => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::appearance::DiffPalette;

    #[test]
    fn folders_take_their_strongest_change() {
        let tints = GitTints::new(
            [
                ("src/a/new.rs".to_string(), GitFileStatus::Untracked),
                ("src/b.rs".to_string(), GitFileStatus::Modified),
                ("docs/old.md".to_string(), GitFileStatus::Deleted),
                ("moved.rs".to_string(), GitFileStatus::Renamed),
            ]
            .into_iter(),
        );
        assert_eq!(tints.dirs.get("src"), Some(&GitFileStatus::Modified));
        assert_eq!(tints.dirs.get("src/a"), Some(&GitFileStatus::Untracked));
        assert_eq!(tints.dirs.get("docs"), Some(&GitFileStatus::Deleted));
        let diff = DiffColors::new(DiffPalette::Default, true);
        assert!(
            tints.color("moved.rs", false, diff).is_none(),
            "renames stay plain"
        );
        assert!(tints.color("src", true, diff).is_some());
    }
}
