//! The one external-editor launcher, over the app's cached editor discovery.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::external_editor::launch_external_editor_with;

impl BenCodeApp {
    /// Opens the active file at its cursor line in the external editor.
    pub fn open_active_file_externally(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.editor.files.active() else {
            return;
        };
        let path = file.path.clone();
        let (line, _) = file.handle.entity.read(cx).position();
        let line = u32::try_from(line).unwrap_or(u32::MAX);
        self.open_in_external_editor(Some(&path), Some(line), cx);
    }

    /// Opens the workspace, or a file at a line, in the preferred external editor.
    pub fn open_in_external_editor(
        &mut self,
        rel_path: Option<&str>,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.preferred_editor().cloned() else {
            let body = if self.integrations.editors.is_none() {
                "Still looking for installed editors. Try again in a moment."
            } else {
                "Install VS Code, Cursor, Zed, Windsurf or Sublime Text."
            };
            self.show_editor_notice("No external editor found".into(), body.into(), cx);
            return;
        };
        let cwd = self.workspace.cwd.clone();
        let rel_path = rel_path.map(str::to_string);
        let task = cx.background_executor().spawn(async move {
            launch_external_editor_with(
                &editor,
                &cwd,
                rel_path.as_deref(),
                line.map(|line| line as usize),
            )
            .map_err(|err| (editor.name, format!("{err:#}")))
        });
        cx.spawn(async move |this, cx| {
            let Err((name, err)) = task.await else {
                return;
            };
            log::warn!("failed to launch external editor {name}: {err}");
            let shown = this.update(cx, |this, cx| {
                this.show_editor_notice(format!("Could not open {name}"), err, cx);
            });
            if let Err(err) = shown {
                log::warn!("app closed before launch error could be shown: {err:#}");
            }
        })
        .detach();
    }
}
