//! The pane MonoCode opens to the right of the chat (`openEditorTab`,
//! `openChangesTab`, `openCommitTab`): one strip of tabs holding open files,
//! per-file reviews, a working tree's Changes and past commits. Browsing
//! lists opens preview tabs that the next pick replaces; closing the last
//! tab gives the chat its full width back.

use crate::ui::git_changes_panel::Side;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneTab {
    /// A file in the code editor, workspace-relative or absolute.
    File { path: String },
    /// One file's working-tree diff, from a Changes row.
    Review {
        cwd: String,
        path: String,
        side: Side,
    },
    /// Every working-tree change stacked; `side` keeps one section only,
    /// `focus` is the file to scroll to.
    Changes {
        cwd: String,
        side: Option<Side>,
        focus: Option<String>,
    },
    /// What one thread's agent changed, from its checkpoints (read-only).
    SessionChanges {
        cwd: String,
        session_id: String,
        focus: Option<String>,
    },
    /// What a past commit changed.
    Commit {
        cwd: String,
        sha: String,
        short_sha: String,
        subject: String,
    },
    /// A page in the in-app browser (`app/browser.rs` keeps its view).
    Browser { id: u64 },
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl PaneTab {
    /// MonoCode `editorTabKey`: one tab per key.
    pub fn key(&self) -> String {
        match self {
            Self::File { path } => format!("file:{path}"),
            Self::Review { cwd, path, .. } => format!("review:{cwd}:{path}"),
            Self::Changes { cwd, .. } => format!("changes:{cwd}"),
            Self::SessionChanges {
                cwd, session_id, ..
            } => {
                format!("session-changes:{cwd}:{session_id}")
            }
            Self::Commit { cwd, sha, .. } => format!("commit:{cwd}:{sha}"),
            Self::Browser { id } => format!("browser:{id}"),
        }
    }

    /// MonoCode `isPreviewableTab`. Files stay put here: replacing one would
    /// drop its unsaved edits.
    fn previewable(&self) -> bool {
        matches!(self, Self::Review { .. } | Self::Commit { .. })
    }

    pub fn is_diff(&self) -> bool {
        !matches!(self, Self::File { .. } | Self::Browser { .. })
    }

    /// The working tree or repository a diff tab reads.
    pub fn cwd(&self) -> Option<&str> {
        match self {
            Self::File { .. } | Self::Browser { .. } => None,
            Self::Review { cwd, .. }
            | Self::Changes { cwd, .. }
            | Self::SessionChanges { cwd, .. }
            | Self::Commit { cwd, .. } => Some(cwd),
        }
    }

    /// Whether `other` lists the same files (a Changes tab's focus aside).
    pub fn same_source(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Review { side: a, .. }, Self::Review { side: b, .. }) => a == b,
            (Self::Changes { side: a, .. }, Self::Changes { side: b, .. }) => a == b,
            _ => self.key() == other.key(),
        }
    }

    /// MonoCode `surfaceTabPresentation`: the tab's label and tooltip.
    pub fn label(&self) -> (String, String) {
        match self {
            Self::File { path } => (basename(path).to_string(), path.clone()),
            Self::Review { path, .. } => (
                format!("{} (Working Tree)", basename(path)),
                format!("{path} (Working Tree)"),
            ),
            Self::Changes { side, .. } => {
                if *side == Some(Side::Staged) {
                    ("Staged Changes".into(), "Staged changes".into())
                } else {
                    ("Changes".into(), "Working tree changes".into())
                }
            }
            Self::SessionChanges { .. } => ("Session Changes".into(), "Session changes".into()),
            Self::Commit {
                short_sha, subject, ..
            } => {
                let name = subject.trim();
                let name = if name.is_empty() { short_sha } else { name };
                (name.to_string(), format!("{short_sha} — {subject}"))
            }
            // The page's title replaces it once loaded (`render_pane_tabs`).
            Self::Browser { .. } => ("Browser".into(), "Browser".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneEntry {
    pub tab: PaneTab,
    /// Shown in italics; the next preview replaces it.
    pub preview: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FilePane {
    entries: Vec<PaneEntry>,
    active: Option<String>,
}

impl FilePane {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[PaneEntry] {
        &self.entries
    }

    pub fn active_key(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&PaneTab> {
        let key = self.active.as_deref()?;
        self.get(key)
    }

    pub fn get(&self, key: &str) -> Option<&PaneTab> {
        self.entries
            .iter()
            .find(|e| e.tab.key() == key)
            .map(|e| &e.tab)
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.tab.key() == key)
    }

    /// Focuses `tab`'s existing tab (taking its new side and focus) or opens
    /// it, over the current preview when it is one. Returns its key.
    pub fn open(&mut self, tab: PaneTab, pin: bool) -> String {
        let key = tab.key();
        if let Some(ix) = self.position(&key) {
            let entry = &mut self.entries[ix];
            entry.tab = tab;
            entry.preview &= !pin;
        } else {
            let preview = tab.previewable() && !pin;
            let entry = PaneEntry { tab, preview };
            match self
                .entries
                .iter()
                .position(|e| e.preview)
                .filter(|_| preview)
            {
                Some(ix) => self.entries[ix] = entry,
                None => self.entries.push(entry),
            }
        }
        self.active = Some(key.clone());
        key
    }

    /// MonoCode `dropPerFileReviewTabs`: a Changes tab takes over the
    /// per-file reviews of its working tree.
    pub fn drop_reviews(&mut self, of_cwd: &str) {
        self.entries
            .retain(|e| !matches!(&e.tab, PaneTab::Review { cwd, .. } if cwd == of_cwd));
        self.fix_active();
    }

    pub fn activate(&mut self, key: &str) -> bool {
        let found = self.position(key).is_some();
        if found {
            self.active = Some(key.to_string());
        }
        found
    }

    /// A double click keeps a preview tab.
    pub fn keep(&mut self, key: &str) {
        if let Some(ix) = self.position(key) {
            self.entries[ix].preview = false;
        }
    }

    /// Closes a tab; the one after it (or before, at the end) takes focus.
    pub fn close(&mut self, key: &str) -> Option<PaneTab> {
        let ix = self.position(key)?;
        let entry = self.entries.remove(ix);
        if self.active.as_deref() == Some(key) {
            let next = ix.min(self.entries.len().saturating_sub(1));
            self.active = self.entries.get(next).map(|e| e.tab.key());
        }
        Some(entry.tab)
    }

    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.entries.len() && to < self.entries.len() {
            let entry = self.entries.remove(from);
            self.entries.insert(to, entry);
        }
    }

    fn fix_active(&mut self) {
        let alive = self
            .active
            .as_deref()
            .is_some_and(|k| self.position(k).is_some());
        if !alive {
            self.active = self.entries.last().map(|e| e.tab.key());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(path: &str) -> PaneTab {
        PaneTab::Review {
            cwd: "/repo".into(),
            path: path.into(),
            side: Side::Unstaged,
        }
    }

    fn file(path: &str) -> PaneTab {
        PaneTab::File { path: path.into() }
    }

    fn keys(pane: &FilePane) -> Vec<String> {
        pane.entries().iter().map(|e| e.tab.key()).collect()
    }

    #[test]
    fn previews_replace_each_other_until_kept() {
        let mut pane = FilePane::default();
        pane.open(file("a.rs"), false);
        pane.open(review("b.rs"), false);
        pane.open(review("c.rs"), false);
        assert_eq!(keys(&pane), ["file:a.rs", "review:/repo:c.rs"]);
        pane.keep("review:/repo:c.rs");
        pane.open(review("d.rs"), false);
        assert_eq!(pane.entries().len(), 3);
        // Pinning an open preview keeps it in place.
        pane.open(review("d.rs"), true);
        assert!(!pane.entries()[2].preview);
        assert_eq!(pane.active_key(), Some("review:/repo:d.rs"));
    }

    #[test]
    fn reopening_takes_the_new_side() {
        let mut pane = FilePane::default();
        pane.open(review("a.rs"), true);
        pane.open(
            PaneTab::Review {
                cwd: "/repo".into(),
                path: "a.rs".into(),
                side: Side::Staged,
            },
            false,
        );
        assert_eq!(pane.entries().len(), 1);
        assert!(matches!(
            pane.active(),
            Some(PaneTab::Review {
                side: Side::Staged,
                ..
            })
        ));
    }

    #[test]
    fn closing_hands_focus_to_a_neighbour() {
        let mut pane = FilePane::default();
        for path in ["a", "b", "c"] {
            pane.open(file(path), false);
        }
        pane.activate("file:b");
        pane.close("file:b");
        assert_eq!(pane.active_key(), Some("file:c"));
        pane.close("file:c");
        assert_eq!(pane.active_key(), Some("file:a"));
        pane.close("file:a");
        assert!(pane.is_empty() && pane.active_key().is_none());
    }

    #[test]
    fn changes_drop_reviews_of_their_tree() {
        let mut pane = FilePane::default();
        pane.open(review("a.rs"), true);
        pane.open(file("x.rs"), false);
        pane.drop_reviews("/repo");
        pane.open(
            PaneTab::Changes {
                cwd: "/repo".into(),
                side: None,
                focus: None,
            },
            false,
        );
        assert_eq!(keys(&pane), ["file:x.rs", "changes:/repo"]);
    }

    #[test]
    fn labels_follow_monocode() {
        assert_eq!(review("src/a.rs").label().0, "a.rs (Working Tree)");
        let commit = PaneTab::Commit {
            cwd: "/r".into(),
            sha: "abc".into(),
            short_sha: "abc".into(),
            subject: " ".into(),
        };
        assert_eq!(commit.label().0, "abc");
    }
}
