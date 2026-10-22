//! Line blame and File History (GitLens's current-line blame and file
//! history): the caret line's last commit as a note at the end of the line,
//! the commit's message and change for the toolbar's card, and the commits
//! that changed a file, for its File History tab. Blame is read off the UI
//! thread once per saved copy of a file; a buffer with unsaved edits shows
//! none, as its lines no longer match.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ely_gpui_component::editor::InlayHint;
use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::git::blame::{self, BlameCommit, CommitDetail, FileBlame, FileCommit};
use crate::ui::editor_pane::files::resolve_path;
use crate::ui::region::Region;

/// A load and what it gave.
#[derive(Clone, Debug)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// What the line blame and File History tabs keep.
#[derive(Default)]
pub struct BlameState {
    /// Each open file's blame, with the `disk_hash` it was read for.
    files: HashMap<String, (u64, Load<Arc<FileBlame>>)>,
    /// Commits' messages and changes, by sha and the file's path there.
    details: HashMap<(String, String), Load<Arc<CommitDetail>>>,
    /// The repository each folder is in.
    repos: HashMap<String, blame::Repo>,
    /// Each editor's caret line (from zero) and the note it shows there.
    shown: HashMap<String, (usize, Option<String>)>,
    /// File History tabs' repository top and commits, by tab key.
    pub histories: HashMap<String, Load<(String, Vec<FileCommit>)>>,
    /// The File History list's scroll, shared by its tabs.
    pub history_scroll: gpui::UniformListScrollHandle,
}

/// The caret line's blame, for the toolbar's card.
#[derive(Clone, Debug)]
pub struct LineBlame {
    /// The repository's top, where its commits open.
    pub cwd: String,
    pub commit: BlameCommit,
    /// The line (from one) in the commit's copy of the file.
    pub orig_line: usize,
    /// The repository's own user wrote it.
    pub you: bool,
    pub detail: Option<Load<Arc<CommitDetail>>>,
}

impl LineBlame {
    /// "You" or the author's name.
    pub fn who(&self) -> &str {
        if self.you || self.commit.is_uncommitted() {
            "You"
        } else {
            &self.commit.author
        }
    }
}

/// The folder to run git in for an editor path, and the file's name
/// there: a path on a machine stays one (`remote://…`).
pub fn git_target(cwd: &str, path: &str) -> (String, String) {
    split_target(&resolve_path(cwd, path), cwd)
}

/// A file's folder and name; `fallback` when it has no folder.
fn split_target(abs: &Path, fallback: &str) -> (String, String) {
    let dir = abs
        .parent()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|| fallback.to_string());
    let name = abs
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    (dir, name)
}

/// The note at the end of a blamed line: "You, 3 weeks ago • subject".
fn note(blame: &LineBlame, now_ms: i64) -> String {
    const SUBJECT_CHARS: usize = 60;
    if blame.commit.is_uncommitted() {
        return "    You • Uncommitted changes".to_string();
    }
    let when = crate::ui::relative_time::since(blame.commit.time * 1000, now_ms);
    let mut subject: String = blame.commit.summary.chars().take(SUBJECT_CHARS).collect();
    if blame.commit.summary.chars().count() > SUBJECT_CHARS {
        subject.push('…');
    }
    format!("    {}, {when} • {subject}", blame.who())
}

/// The byte offset where the line holding `at` ends.
fn line_end(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text[at..].find('\n').map_or(text.len(), |n| at + n)
}

impl BenCodeApp {
    /// `git_target` of an editor path: an open file's own folder, which
    /// stays the one it opened from when the workspace moves to another
    /// project.
    fn file_git_target(&self, path: &str, cx: &gpui::App) -> (String, String) {
        let cwd = &self.workspace(cx).cwd;
        match self.editor.files.get(path) {
            Some(file) => split_target(&file.handle.abs_path, cwd),
            None => git_target(cwd, path),
        }
    }

    /// The blame of `path`'s caret line, once read; none while it has
    /// unsaved edits.
    pub(crate) fn caret_blame(&self, path: &str, cx: &gpui::App) -> Option<LineBlame> {
        let (line, _) = self.blame.shown.get(path)?;
        self.line_blame(path, *line, cx)
    }

    fn line_blame(&self, path: &str, line: usize, cx: &gpui::App) -> Option<LineBlame> {
        let file = self.editor.files.get(path)?;
        if file.is_dirty() {
            return None;
        }
        let (hash, Load::Ready(blame)) = self.blame.files.get(path)? else {
            return None;
        };
        if *hash != file.handle.disk_hash {
            return None;
        }
        let (commit, orig_line) = blame.line(line)?;
        let (dir, _) = self.file_git_target(path, cx);
        let repo = self.blame.repos.get(&dir);
        let cwd = repo.map_or(dir.clone(), |repo| repo.root.clone());
        let you = repo
            .and_then(|repo| repo.email.as_deref())
            .is_some_and(|email| email.eq_ignore_ascii_case(&commit.author_mail));
        let detail = self
            .blame
            .details
            .get(&(commit.sha.clone(), commit.path.clone()))
            .cloned();
        Some(LineBlame {
            cwd,
            commit: commit.clone(),
            orig_line,
            you,
            detail,
        })
    }

    /// Follows `path`'s caret: its line's blame as a note at the line's
    /// end, read first if this copy of the file has none. Called on every
    /// change to the editor, so it does nothing when nothing moved.
    pub(crate) fn track_line_blame(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(file) = self.editor.files.get(path) else {
            return;
        };
        let (entity, hash, dirty) = (
            file.handle.entity.clone(),
            file.handle.disk_hash,
            file.is_dirty(),
        );
        let (line, head) = {
            let editor = entity.read(cx);
            (editor.position().0 - 1, editor.primary().head)
        };
        let fresh = self
            .blame
            .files
            .get(path)
            .is_some_and(|(read_for, _)| *read_for == hash);
        if !dirty && !fresh {
            self.load_blame(path, hash, cx);
        }
        let blame = self.line_blame(path, line, cx);
        let text = blame
            .as_ref()
            .map(|blame| note(blame, crate::app::now_ms()));
        let next = (line, text);
        if self.blame.shown.get(path) == Some(&next) {
            return;
        }
        if let Some(blame) = &blame {
            self.load_commit_detail(&blame.cwd, &blame.commit, cx);
        }
        let hints = next.1.clone().map(|text| InlayHint {
            offset: line_end(entity.read(cx).text(), head),
            text: text.into(),
        });
        self.blame.shown.insert(path.to_string(), next);
        entity.update(cx, |editor, cx| {
            editor.set_inlay_hints(hints.into_iter().collect(), cx)
        });
    }

    /// The editor's text was replaced (a reload): its note went with it.
    pub(crate) fn forget_line_blame(&mut self, path: &str) {
        self.blame.shown.remove(path);
    }

    /// `path` closed.
    pub(crate) fn forget_file_blame(&mut self, path: &str) {
        self.blame.shown.remove(path);
        self.blame.files.remove(path);
    }

    /// Commits moved (a commit, a checkout): every blame is read again
    /// when next shown.
    pub(crate) fn invalidate_blames(&mut self, cx: &mut Context<Self>) {
        if self.blame.files.is_empty() {
            return;
        }
        self.blame.files.clear();
        self.blame.repos.clear();
        let paths: Vec<String> = self.blame.shown.keys().cloned().collect();
        for path in paths {
            self.track_line_blame(&path, cx);
        }
    }

    fn load_blame(&mut self, path: &str, hash: u64, cx: &mut Context<Self>) {
        self.blame
            .files
            .insert(path.to_string(), (hash, Load::Loading));
        let (cwd, name) = self.file_git_target(path, cx);
        let need_repo = !self.blame.repos.contains_key(&cwd);
        let key = path.to_string();
        let task = cx.background_executor().spawn(async move {
            let repo = need_repo.then(|| blame::repo_of(&cwd));
            (cwd.clone(), repo, blame::blame_file(&cwd, &name))
        });
        cx.spawn(async move |this, cx| {
            let (cwd, repo, result) = task.await;
            let landed = this.update(cx, |this, cx| {
                if let Some(repo) = repo {
                    this.blame.repos.insert(cwd, repo);
                }
                // A newer copy of the file was asked for meanwhile.
                if this.blame.files.get(&key).is_none_or(|(at, _)| *at != hash) {
                    return;
                }
                let load = match result {
                    Ok(blame) => Load::Ready(Arc::new(blame)),
                    Err(err) => {
                        // Untracked files and folders outside git land here.
                        log::debug!("no blame for {key}: {err:#}");
                        Load::Failed(err.to_string())
                    }
                };
                this.blame.files.insert(key.clone(), (hash, load));
                this.track_line_blame(&key, cx);
            });
            if let Err(err) = landed {
                log::debug!("blame landed after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn load_commit_detail(&mut self, cwd: &str, commit: &BlameCommit, cx: &mut Context<Self>) {
        if commit.is_uncommitted() {
            return;
        }
        let key = (commit.sha.clone(), commit.path.clone());
        if self.blame.details.contains_key(&key) {
            return;
        }
        self.blame.details.insert(key.clone(), Load::Loading);
        let (cwd, sha, path) = (cwd.to_string(), key.0.clone(), key.1.clone());
        let task = cx
            .background_executor()
            .spawn(async move { blame::commit_detail(&cwd, &sha, &path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                let load = match result {
                    Ok(detail) => Load::Ready(Arc::new(detail)),
                    Err(err) => {
                        log::warn!("could not read commit {}: {err:#}", key.0);
                        Load::Failed(err.to_string())
                    }
                };
                this.blame.details.insert(key, load);
                // The toolbar's card is the file pane's.
                this.redraw_regions(&[Region::FilePane], cx);
            });
            if let Err(err) = landed {
                log::debug!("commit detail landed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Opens the commit the blame names, as a tab beside the file.
    pub(crate) fn open_blame_commit(
        &mut self,
        cwd: String,
        sha: String,
        subject: String,
        cx: &mut Context<Self>,
    ) {
        let short_sha = sha.chars().take(7).collect();
        self.open_pane_tab(
            PaneTab::Commit {
                cwd,
                sha,
                short_sha,
                subject,
            },
            true,
            cx,
        );
    }

    /// Opens `path`'s File History tab (an editor path) and reads its
    /// commits.
    pub(crate) fn open_file_history(&mut self, path: &str, cx: &mut Context<Self>) {
        let (cwd, name) = self.file_git_target(path, cx);
        self.open_pane_tab(
            PaneTab::FileHistory {
                cwd: cwd.clone(),
                path: name.clone(),
            },
            true,
            cx,
        );
        self.load_file_history(cwd, name, cx);
    }

    /// Reads the commits that changed `name` in `cwd` for its tab.
    pub(crate) fn load_file_history(&mut self, cwd: String, name: String, cx: &mut Context<Self>) {
        let key = PaneTab::FileHistory {
            cwd: cwd.clone(),
            path: name.clone(),
        }
        .key();
        self.blame.histories.insert(key.clone(), Load::Loading);
        let task = cx.background_executor().spawn(async move {
            let root = blame::repo_of(&cwd).root;
            blame::file_history(&cwd, &name).map(|commits| (root, commits))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                let load = match result {
                    Ok(history) => Load::Ready(history),
                    Err(err) => {
                        log::warn!("could not read the history of {key}: {err:#}");
                        Load::Failed(err.to_string())
                    }
                };
                this.blame.histories.insert(key, load);
                this.redraw_regions(&[Region::FilePane], cx);
            });
            if let Err(err) = landed {
                log::debug!("file history landed after app drop: {err:#}");
            }
        })
        .detach();
        self.redraw_regions(&[Region::FilePane], cx);
    }

    /// File History of the file open in the pane (the command).
    pub(crate) fn show_active_file_history(&mut self, cx: &mut Context<Self>) {
        if let Some(PaneTab::File { path }) = self.file_pane.active().cloned() {
            self.open_file_history(&path, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blamed(author: &str, you: bool, sha: &str) -> LineBlame {
        LineBlame {
            cwd: "/r".into(),
            commit: BlameCommit {
                sha: sha.into(),
                author: author.into(),
                time: 0,
                summary: "docs: spec".into(),
                ..BlameCommit::default()
            },
            orig_line: 1,
            you,
            detail: None,
        }
    }

    #[test]
    fn notes_name_the_author_and_when() {
        let day = 86_400_000;
        let mine = blamed("Ben", true, "43ac07a");
        assert_eq!(note(&mine, 3 * day), "    You, 3 days ago • docs: spec");
        let theirs = blamed("Ana", false, "43ac07a");
        assert_eq!(note(&theirs, 3 * day), "    Ana, 3 days ago • docs: spec");
        let zeros = "0".repeat(40);
        assert_eq!(
            note(&blamed("Not Committed Yet", false, &zeros), 0),
            "    You • Uncommitted changes"
        );
    }

    #[test]
    fn line_end_stops_at_the_break() {
        assert_eq!(line_end("ab\ncd", 1), 2);
        assert_eq!(line_end("ab\ncd", 3), 5);
        assert_eq!(line_end("ab", 9), 2);
    }

    #[test]
    fn targets_split_folder_and_name() {
        assert_eq!(
            git_target("/repo", "docs/a.md"),
            ("/repo/docs".to_string(), "a.md".to_string())
        );
        assert_eq!(
            git_target("/repo", "/else/b.rs"),
            ("/else".to_string(), "b.rs".to_string())
        );
    }
}
