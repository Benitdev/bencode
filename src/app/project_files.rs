//! The project's file index (MonoCode `fileIndex.ts`), shared by Go to File,
//! the composer's `@` picker and Search. Re-listed off the UI thread whenever
//! one of them opens; the last list of the same folder stays usable meanwhile.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, SharedString};

use crate::ui::composer::mentions::MentionIndex;

use crate::app::BenCodeApp;

#[derive(Default)]
pub struct ProjectFiles {
    /// Folder the list belongs to.
    pub root: String,
    /// Paths relative to `root`, sorted.
    pub files: Vec<SharedString>,
    /// The folders holding them (MonoCode `withMentionDirectories`), for `@`.
    pub dirs: Vec<SharedString>,
    /// A listing of `root` is under way.
    pub loading: bool,
    /// `@` labels for `files`, `dirs` and notes. Shared with the prompt's
    /// highlighter, which cannot reach the app.
    pub mentions: Rc<RefCell<Arc<MentionIndex>>>,
    /// Other folders' last lists, put back at once on return while a fresh
    /// one loads (MonoCode keeps `fileIndex` per folder).
    others: HashMap<String, Listing>,
}

#[derive(Clone)]
struct Listing {
    files: Vec<SharedString>,
    dirs: Vec<SharedString>,
    mentions: Arc<MentionIndex>,
}

impl ProjectFiles {
    pub fn new(mentions: Rc<RefCell<Arc<MentionIndex>>>) -> Self {
        Self {
            mentions,
            ..Default::default()
        }
    }

    /// Moves to `root`, parking the current list and restoring `root`'s.
    fn switch_root(&mut self, root: &str) {
        let current = Listing {
            files: std::mem::take(&mut self.files),
            dirs: std::mem::take(&mut self.dirs),
            mentions: self.mentions.borrow().clone(),
        };
        let old = std::mem::replace(&mut self.root, root.to_string());
        if !old.is_empty() {
            self.others.insert(old, current);
        }
        let restored = self.others.remove(root);
        let mentions = restored
            .as_ref()
            .map(|l| l.mentions.clone())
            .unwrap_or_default();
        if let Some(listing) = restored {
            self.files = listing.files;
            self.dirs = listing.dirs;
        }
        *self.mentions.borrow_mut() = mentions;
    }
}

/// Every folder on the way to `files`, each once, sorted.
pub fn directories_of(files: &[String]) -> Vec<String> {
    let mut dirs = std::collections::BTreeSet::new();
    for file in files {
        let mut at = 0;
        while let Some(slash) = file[at..].find('/') {
            at += slash;
            dirs.insert(file[..at].to_string());
            at += 1;
        }
    }
    dirs.into_iter().collect()
}

impl BenCodeApp {
    /// MonoCode `loadProjectFiles(cwd, true)`.
    pub fn index_project_files(&mut self, cx: &mut Context<Self>) {
        let root = self.workspace_cwd();
        let index = &mut self.project_files;
        if root != index.root {
            index.switch_root(&root);
        }
        if root.trim().is_empty() || index.loading {
            return;
        }
        index.loading = true;
        let path = std::path::PathBuf::from(&root);
        let notes: Vec<String> = self
            .notes
            .items
            .iter()
            .map(|n| format!("note/{}", n.slug))
            .collect();
        let task = cx.background_executor().spawn(async move {
            let files = crate::workspace::list_project_files(&path);
            let dirs = directories_of(&files);
            let mentions = MentionIndex::build(&files, &dirs, &notes);
            (files, dirs, mentions)
        });
        cx.spawn(async move |this, cx| {
            let (files, dirs, mentions) = task.await;
            let stored = this.update(cx, |app, cx| {
                let index = &mut app.project_files;
                let listing = Listing {
                    files: files.into_iter().map(SharedString::from).collect(),
                    dirs: dirs.into_iter().map(SharedString::from).collect(),
                    mentions: Arc::new(mentions),
                };
                index.loading = false;
                if index.root == root {
                    index.files = listing.files;
                    index.dirs = listing.dirs;
                    *index.mentions.borrow_mut() = listing.mentions;
                } else {
                    // The folder was left meanwhile: keep this list for a
                    // return, and list the folder now shown.
                    index.others.insert(root, listing);
                    app.index_project_files(cx);
                }
                cx.notify();
            });
            if let Err(err) = stored {
                log::debug!("project files listed after app drop: {err:#}");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_cover_every_level_once() {
        let files = ["src/ui/a.rs", "src/ui/b.rs", "README.md"].map(String::from);
        assert_eq!(directories_of(&files), ["src", "src/ui"]);
    }

    #[test]
    fn returning_to_a_folder_restores_its_list() {
        let mut index = ProjectFiles::new(Rc::default());
        index.switch_root("/a");
        index.files = vec!["a.rs".into()];
        index.switch_root("/b");
        assert!(index.files.is_empty());
        index.switch_root("/a");
        assert_eq!(index.files, [SharedString::from("a.rs")]);
    }
}
