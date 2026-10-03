//! Installed external editors and configured MCP servers, discovered once on
//! the background executor. Both scans touch PATH and config files, so views
//! read this cache instead of calling the discovery functions in render.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::external_editor::{ExternalEditor, list_external_editors};
use crate::mcp::{McpConnection, discover_mcp_servers};
use crate::skills::{self, Skill};

/// `None` until the first scan finishes.
#[derive(Default)]
pub struct Integrations {
    pub editors: Option<Vec<ExternalEditor>>,
    pub mcp_servers: Option<Vec<McpConnection>>,
    /// Skills of `skills_project`, rescanned when the project changes.
    pub skills: Vec<Skill>,
    pub skills_project: String,
    /// The skills' names, shared with the prompt's highlighter.
    pub skill_names: std::rc::Rc<std::cell::RefCell<std::sync::Arc<Vec<String>>>>,
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
                app.integrations.editors = Some(editors);
                app.integrations.mcp_servers = Some(mcp_servers);
                cx.notify();
            });
        })
        .detach();
    }

    /// Rescans `SKILL.md` folders when the current project changed since the
    /// last scan (or `force`, e.g. the Skills page's refresh).
    pub fn refresh_skills(&mut self, force: bool, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        if !force && self.integrations.skills_project == project {
            return;
        }
        self.integrations.skills_project = project.clone();
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let scan_project = project.clone();
        let task = cx.background_executor().spawn(async move {
            let disabled = std::collections::HashSet::new();
            skills::list_skills(
                std::path::Path::new(&scan_project),
                home.as_deref(),
                &disabled,
            )
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let updated = this.update(cx, |app, cx| {
                if app.integrations.skills_project == project {
                    *app.integrations.skill_names.borrow_mut() =
                        std::sync::Arc::new(found.iter().map(|s| s.name.clone()).collect());
                    app.integrations.skills = found;
                    cx.notify();
                }
            });
            if let Err(err) = updated {
                log::debug!("skills scanned after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// `prompt` with the bodies of the `/skills` it names prepended, as
    /// MonoCode's `applySkillsToTurn` does for harnesses without native
    /// slash commands (all of BenCode's).
    pub fn apply_skills(&self, prompt: &str) -> String {
        let picked: Vec<(String, String)> = skills::skill_names_in_text(prompt)
            .into_iter()
            .filter_map(|name| self.integrations.skills.iter().find(|s| s.name == name))
            .map(|skill| (skill.name.clone(), skill.body()))
            .collect();
        skills::inject_skill_prompt(prompt, &picked)
    }

    /// The preferred external editor, once discovery has run.
    pub fn preferred_editor(&self) -> Option<&ExternalEditor> {
        self.integrations.editors.as_ref()?.first()
    }
}
