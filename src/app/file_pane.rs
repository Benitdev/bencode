//! The pane MonoCode opens to the right of the chat (`openEditorTab`,
//! `openChangesTab`, `openCommitTab`): one strip of tabs holding open files,
//! per-file reviews, a working tree's Changes and past commits. Browsing
//! lists opens preview tabs that the next pick replaces; closing the last
//! tab gives the chat its full width back. An Inbox item opens here too,
//! so a ticket stays readable beside the thread working on it.

use crate::ui::git_changes_panel::Side;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneTab {
    /// A file in the code editor, workspace-relative or absolute.
    File { path: String },
    /// A picture, shown rather than edited, by its full path.
    Image { path: String },
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
    /// The commits that changed a file (`app/blame.rs` keeps them): the
    /// folder git runs in and the file's name there.
    FileHistory { cwd: String, path: String },
    /// A page in the in-app browser (`app/browser.rs` keeps its view).
    Browser { id: u64 },
    /// An Inbox item, by its key (`WorkItem::key`); the Inbox's state
    /// holds what it shows (`ui/inbox_view/tab.rs`).
    WorkItem { key: String },
}

/// How an Inbox key names its item before the item is known: `PROJ-12`,
/// `#42`.
fn work_item_name(key: &str) -> String {
    match key.strip_prefix("backlog:") {
        Some(issue) => issue.to_ascii_uppercase(),
        None => key
            .rsplit_once(':')
            .map_or_else(|| key.to_string(), |(_, number)| format!("#{number}")),
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl PaneTab {
    /// MonoCode `editorTabKey`: one tab per key.
    pub fn key(&self) -> String {
        match self {
            Self::File { path } => format!("file:{path}"),
            Self::Image { path } => format!("image:{path}"),
            Self::Review { cwd, path, .. } => format!("review:{cwd}:{path}"),
            Self::Changes { cwd, .. } => format!("changes:{cwd}"),
            Self::SessionChanges {
                cwd, session_id, ..
            } => {
                format!("session-changes:{cwd}:{session_id}")
            }
            Self::Commit { cwd, sha, .. } => format!("commit:{cwd}:{sha}"),
            Self::FileHistory { cwd, path } => format!("file-history:{cwd}/{path}"),
            Self::Browser { id } => format!("browser:{id}"),
            Self::WorkItem { key } => format!("work-item:{key}"),
        }
    }

    /// MonoCode `isPreviewableTab`. Files stay put here: replacing one would
    /// drop its unsaved edits.
    fn previewable(&self) -> bool {
        matches!(
            self,
            Self::Review { .. } | Self::Commit { .. } | Self::WorkItem { .. }
        )
    }

    pub fn is_diff(&self) -> bool {
        !matches!(
            self,
            Self::File { .. }
                | Self::Image { .. }
                | Self::Browser { .. }
                | Self::FileHistory { .. }
                | Self::WorkItem { .. }
        )
    }

    /// The working tree or repository a diff tab reads.
    pub fn cwd(&self) -> Option<&str> {
        match self {
            Self::File { .. }
            | Self::Image { .. }
            | Self::Browser { .. }
            | Self::WorkItem { .. } => None,
            Self::Review { cwd, .. }
            | Self::Changes { cwd, .. }
            | Self::SessionChanges { cwd, .. }
            | Self::Commit { cwd, .. }
            | Self::FileHistory { cwd, .. } => Some(cwd),
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
            Self::File { path } | Self::Image { path } => {
                (basename(path).to_string(), path.clone())
            }
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
            Self::FileHistory { path, .. } => (
                format!("{} (History)", basename(path)),
                format!("History of {path}"),
            ),
            // The page's title replaces it once loaded (`render_pane_tabs`).
            Self::Browser { .. } => ("Browser".into(), "Browser".into()),
            // The item's key and title replace it once known.
            Self::WorkItem { key } => (work_item_name(key), key.clone()),
        }
    }
}

/// Which tabs a tab's menu closes besides (or with) it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMany {
    Others,
    Right,
    All,
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

    /// An Inbox item has a tab: the pane draws from the Inbox's state (its
    /// detail, or the tab's label) and follows it.
    pub fn shows_work_item(&self) -> bool {
        (self.entries.iter()).any(|e| matches!(e.tab, PaneTab::WorkItem { .. }))
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

    /// The keys `which` closes, counted from `key`'s tab, in strip order.
    pub fn close_keys(&self, key: &str, which: CloseMany) -> Vec<String> {
        let Some(at) = self.position(key) else {
            return Vec::new();
        };
        (self.entries.iter().enumerate())
            .filter(|(ix, _)| match which {
                CloseMany::Others => *ix != at,
                CloseMany::Right => *ix > at,
                CloseMany::All => true,
            })
            .map(|(_, e)| e.tab.key())
            .collect()
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
    fn close_keys_count_from_the_clicked_tab() {
        let mut pane = FilePane::default();
        for path in ["a", "b", "c"] {
            pane.open(file(path), false);
        }
        assert_eq!(
            pane.close_keys("file:b", CloseMany::Others),
            ["file:a", "file:c"]
        );
        assert_eq!(pane.close_keys("file:b", CloseMany::Right), ["file:c"]);
        assert!(pane.close_keys("file:c", CloseMany::Right).is_empty());
        assert_eq!(pane.close_keys("file:a", CloseMany::All).len(), 3);
        assert!(pane.close_keys("file:x", CloseMany::All).is_empty());
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
    fn inbox_items_are_previews_keyed_by_their_item() {
        let ticket = PaneTab::WorkItem {
            key: "backlog:proj_d-1147".into(),
        };
        assert_eq!(ticket.key(), "work-item:backlog:proj_d-1147");
        assert_eq!(ticket.label().0, "PROJ_D-1147");
        assert!(!ticket.is_diff() && ticket.cwd().is_none());
        let pr = PaneTab::WorkItem {
            key: "owner/repo:pr:182".into(),
        };
        assert_eq!(pr.label().0, "#182");
        // Reading one item after another keeps one tab.
        let mut pane = FilePane::default();
        pane.open(file("a.rs"), false);
        pane.open(ticket, false);
        pane.open(pr.clone(), false);
        assert_eq!(keys(&pane), ["file:a.rs", "work-item:owner/repo:pr:182"]);
        pane.open(pr, true);
        assert!(!pane.entries()[1].preview);
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
