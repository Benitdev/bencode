//! Installed external editors and configured MCP servers, discovered once on
//! the background executor. Both scans touch PATH and config files, so views
//! read this cache instead of calling the discovery functions in render.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::external_editor::{ExternalEditor, list_external_editors};
use crate::mcp::{McpConnection, discover_mcp_servers};

/// `None` until the first scan finishes.
#[derive(Default)]
pub struct Integrations {
    pub editors: Option<Vec<ExternalEditor>>,
    pub mcp_servers: Option<Vec<McpConnection>>,
}

impl BenCodeApp {
    /// Rescans editors and MCP config for the current workspace.
    pub fn refresh_integrations(&mut self, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        let task = cx
            .background_executor()
            .spawn(async move { (list_external_editors(), discover_mcp_servers(&cwd)) });
        cx.spawn(async move |this, cx| {
            let (editors, mcp_servers) = task.await;
            let _ = this.update(cx, |app, cx| {
                app.integrations = Integrations {
                    editors: Some(editors),
                    mcp_servers: Some(mcp_servers),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// The preferred external editor, once discovery has run.
    pub fn preferred_editor(&self) -> Option<&ExternalEditor> {
        self.integrations.editors.as_ref()?.first()
    }
}
