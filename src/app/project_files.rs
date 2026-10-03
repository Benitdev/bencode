//! The project's file index (MonoCode `fileIndex.ts`), shared by Go to File,
//! the composer's `@` picker and Search. Re-listed off the UI thread whenever
//! one of them opens; the last list of the same folder stays usable meanwhile.

use std::cell::RefCell;
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
            index.files.clear();
            index.dirs.clear();
            index.root = root.clone();
        }
        if root.trim().is_empty() || index.loading {
            return;
        }
        index.loading = true;
        let path = std::path::PathBuf::from(&root);
        let notes: Vec<String> = self
            .notes
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
                // A listing for a folder since left is stale.
                if index.root == root {
                    index.files = files.into_iter().map(SharedString::from).collect();
                    index.dirs = dirs.into_iter().map(SharedString::from).collect();
                    *index.mentions.borrow_mut() = Arc::new(mentions);
                }
                index.loading = false;
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
}
