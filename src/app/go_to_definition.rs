//! Go to Definition (⌘-click, F12) by name: the name at the caret is
//! searched in the project as a declaration (`bencode_core::definitions`),
//! here or on the project's machine (`project_search`). One candidate,
//! or one ranked clearly first, opens at once; several open a picker; none says so in the
//! editor's toolbar. Nothing runs in the background between jumps.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bencode_core::definitions::{self, Definition};
use gpui::{Context, Window};

use crate::app::BenCodeApp;
use crate::ui::editor_pane::files::{relative_to, resolve_path};
use crate::ui::region::Region;

/// How long the toolbar shows that a jump went nowhere.
const MESSAGE_FOR: Duration = Duration::from_secs(5);

/// Several candidates for one name, waiting for a pick.
pub struct DefinitionPicker {
    pub name: String,
    /// The folder the candidates are relative to.
    pub root: String,
    pub items: Vec<Definition>,
}

#[derive(Default)]
pub struct DefinitionState {
    /// The latest jump; an older answer is dropped.
    generation: u64,
    /// The editor a ⌘-click is landing in: its caret's next move jumps.
    pub(crate) jump_on_click: Option<String>,
    pub(crate) picker: Option<DefinitionPicker>,
    /// Why the last jump went nowhere, for that file's toolbar.
    message: Option<(String, String)>,
}

/// The folder to search for an editor path, and the path within it: the
/// workspace when the file is in it, else the file's own folder.
fn search_root(cwd: &str, path: &str) -> (String, String) {
    let relative = relative_to(cwd, path);
    if !Path::new(&relative).is_absolute() && !crate::remote::is_remote(&relative) {
        return (cwd.to_string(), relative);
    }
    let abs = resolve_path(cwd, path);
    let dir = abs
        .parent()
        .map_or_else(|| cwd.to_string(), |dir| dir.to_string_lossy().into_owned());
    let name = abs
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    (dir, name)
}

impl BenCodeApp {
    /// Why the last jump from `path` went nowhere, while it shows.
    pub(crate) fn definition_message(&self, path: &str) -> Option<&str> {
        let (at, message) = self.definition.message.as_ref()?;
        (at == path).then_some(message.as_str())
    }

    fn say_definition(&mut self, path: &str, message: String, cx: &mut Context<Self>) {
        self.definition.message = Some((path.to_string(), message));
        self.redraw_regions(&[Region::FilePane], cx);
        let generation = self.definition.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MESSAGE_FOR).await;
            let landed = this.update(cx, |this, cx| {
                if this.definition.generation == generation {
                    this.definition.message = None;
                    this.redraw_regions(&[Region::FilePane], cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("definition message after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Go to Definition (F12): the name at the active file's caret.
    pub(crate) fn go_to_definition_at_caret(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.editor.files.active_path().map(str::to_string) else {
            return;
        };
        let Some(head) = self
            .editor
            .files
            .get(&path)
            .map(|file| file.handle.entity.read(cx).primary().head)
        else {
            return;
        };
        self.go_to_definition(&path, head, window, cx);
    }

    /// Finds where the name at byte `offset` of `path` is declared.
    pub(crate) fn go_to_definition(
        &mut self,
        path: &str,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.definition.generation += 1;
        self.definition.message = None;
        let generation = self.definition.generation;
        let Some(file) = self.editor.files.get(path) else {
            return;
        };
        let editor = file.handle.entity.read(cx);
        let text = editor.text();
        let Some(range) = definitions::name_at(text, offset) else {
            self.say_definition(path, "Put the caret on a name".into(), cx);
            return;
        };
        let name = text[range.clone()].to_string();
        let from_line = text[..range.start].matches('\n').count() as u32 + 1;
        // The file's own path: its tab may belong to another project.
        let cwd = self.workspace(cx).cwd.clone();
        let (root, from) = search_root(&cwd, &file.handle.abs_path.to_string_lossy());
        let options = definitions::search_options(&name, &from);
        let search_in = PathBuf::from(&root);
        let task = cx.background_executor().spawn(async move {
            crate::project_search::search(&search_in, &options, &AtomicBool::new(false))
        });
        let at = path.to_string();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let landed = this.update_in(cx, |this, window, cx| {
                if this.definition.generation != generation {
                    return;
                }
                let found = match result {
                    Ok(found) => definitions::rank(&name, &from, from_line, found.matches),
                    Err(err) => {
                        log::warn!("go to definition of {name}: {err}");
                        this.say_definition(&at, format!("Could not search: {err}"), cx);
                        return;
                    }
                };
                log::info!("go to definition of {name}: {} candidates", found.len());
                match found.as_slice() {
                    [] => this.say_definition(&at, format!("No definition of `{name}` found"), cx),
                    [only] if only.relative == from && only.line == from_line => {
                        this.say_definition(&at, format!("This is where `{name}` is defined"), cx)
                    }
                    _ if let Some(best) = definitions::clear_winner(&found) => {
                        let best = best.clone();
                        this.open_definition(&root, &best, window, cx);
                    }
                    _ => {
                        this.definition.picker = Some(DefinitionPicker {
                            name,
                            root,
                            items: found,
                        });
                        cx.notify();
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("definition landed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Opens a candidate with its name selected.
    pub(crate) fn open_definition(
        &mut self,
        root: &str,
        found: &Definition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.workspace(cx).cwd.clone();
        let path = if root == cwd {
            found.relative.clone()
        } else {
            format!("{}/{}", root.trim_end_matches('/'), found.relative)
        };
        log::info!("go to definition: {path}:{}", found.line);
        self.open_file_at(&path, found.line, found.column, found.len, window, cx);
    }

    pub(crate) fn close_definition_picker(&mut self, cx: &mut Context<Self>) {
        if self.definition.picker.take().is_some() {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_the_workspace_when_the_file_is_in_it() {
        assert_eq!(
            search_root("/repo", "src/a.rs"),
            ("/repo".to_string(), "src/a.rs".to_string())
        );
        assert_eq!(
            search_root("/repo", "/repo/src/a.rs"),
            ("/repo".to_string(), "src/a.rs".to_string())
        );
        assert_eq!(
            search_root("/repo", "/else/b.rs"),
            ("/else".to_string(), "b.rs".to_string())
        );
    }
}
