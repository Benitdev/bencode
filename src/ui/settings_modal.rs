//! Settings: general defaults, provider CLIs, MCP, skills, appearance, about.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::Switch;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::settings::{SettingsLayout, SettingsRow, SettingsSection};
use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};

use crate::app::BenCodeApp;
use crate::harness::HarnessInfo;
use crate::ui::HarnessIcon;
use crate::ui::scale::px;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    General,
    #[default]
    Providers,
    Mcp,
    Skills,
    Appearance,
    About,
    /// MonoCode Settings › Archive: archived projects.
    Archive,
    /// MonoCode Settings › Worktrees (`ui/settings_worktrees.rs`).
    Worktrees,
}

const SECTIONS: [(SettingsTab, &str, &str, IconName); 8] = [
    (
        SettingsTab::General,
        "general",
        "General",
        IconName::Settings,
    ),
    (
        SettingsTab::Providers,
        "providers",
        "Providers",
        IconName::Key,
    ),
    (
        SettingsTab::Mcp,
        "mcp",
        "MCP servers",
        IconName::SlidersHorizontal,
    ),
    (SettingsTab::Skills, "skills", "Skills", IconName::Sparkles),
    (
        SettingsTab::Appearance,
        "appearance",
        "Appearance",
        IconName::Palette,
    ),
    (SettingsTab::About, "about", "About", IconName::Info),
    (SettingsTab::Archive, "archive", "Archive", IconName::Archive),
    // MonoCode's nav uses `FolderTree`, which Ely's `IconName` lacks.
    (
        SettingsTab::Worktrees,
        "worktrees",
        "Worktrees",
        IconName::GitBranch,
    ),
];

/// MonoCode `SETTINGS_GROUPS` with BenCode's sections in them (About is
/// BenCode's own and sits with App), for the rail's settings nav.
pub(crate) const SETTINGS_GROUPS: [(&str, &[SettingsTab]); 3] = [
    (
        "App",
        &[SettingsTab::General, SettingsTab::Appearance, SettingsTab::About],
    ),
    (
        "Agents",
        &[SettingsTab::Providers, SettingsTab::Mcp, SettingsTab::Skills],
    ),
    ("Workspace", &[SettingsTab::Archive, SettingsTab::Worktrees]),
];

impl SettingsTab {
    fn key(self) -> &'static str {
        SECTIONS
            .iter()
            .find(|(tab, ..)| *tab == self)
            .map_or("general", |(_, key, ..)| key)
    }

    /// The nav's label and icon.
    pub(crate) fn label_icon(self) -> (&'static str, IconName) {
        SECTIONS
            .iter()
            .find(|(tab, ..)| *tab == self)
            .map_or(("General", IconName::Settings), |(_, _, name, icon)| (*name, *icon))
    }

    fn from_key(key: &str) -> Option<Self> {
        SECTIONS
            .iter()
            .find(|(_, k, ..)| *k == key)
            .map(|(tab, ..)| *tab)
    }
}

impl BenCodeApp {
    /// Skills found in the project and user folders (MonoCode `SkillsPage`).
    fn render_settings_skills(&self, cx: &Context<Self>) -> impl IntoElement {
        let skills = &self.integrations.skills;
        let section = SettingsSection::new("Skills").description(format!(
            "{} skills. Type / in the composer to use one; add folders with a SKILL.md under .agents/skills.",
            skills.len()
        ));
        let section = section.row(
            SettingsRow::new("Rescan")
                .description("Rescan skill folders")
                .control(
                    IconButton::new("skills-rescan", IconName::RefreshCw)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_skills(true, cx))),
                ),
        );
        skills.iter().fold(section, |section, skill| {
            let detail = if skill.path.is_empty() {
                skill.description.clone()
            } else {
                format!("{}\n{}", skill.description, skill.path)
            };
            section.row(
                SettingsRow::new(format!("/{}", skill.name))
                    .description(detail)
                    .control(Badge::new(format!("{} · {}", skill.scope, skill.source))),
            )
        })
    }

    pub(crate) fn render_settings_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // MonoCode: the project rail holds the sections while settings
        // are open, so the page stands alone.
        if self.is_rail_open {
            // MonoCode `mx-auto w-full max-w-5xl px-8 py-8 pb-16`, scrolling.
            let page = div()
                .id("settings-page")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .mx_auto()
                        .w_full()
                        .max_w(px(1024.0))
                        .px_8()
                        .pt_8()
                        .pb_16()
                        .child(self.render_settings_page(cx)),
                );
            return crate::ui::scrollbar::Scrolled::new("settings-page-scrollbar", page)
                .into_any_element();
        }
        let layout = SettingsLayout::new(
            "settings-layout",
            SECTIONS
                .iter()
                .map(|(_, key, name, icon)| (*key, *name, *icon)),
            self.settings_tab.key(),
        )
        .page(self.render_settings_page(cx))
        .on_select(cx.listener(|this, key: &SharedString, _, cx| {
            match SettingsTab::from_key(key) {
                Some(tab) => this.select_settings_tab(tab, cx),
                None => log::warn!("unknown settings section {key}"),
            }
        }));
        div().size_full().child(layout).into_any_element()
    }

    fn render_settings_page(&self, cx: &Context<Self>) -> AnyElement {
        match self.settings_tab {
            SettingsTab::General => self.render_settings_general(cx).into_any_element(),
            SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
            SettingsTab::Mcp => self.render_settings_mcp().into_any_element(),
            SettingsTab::Skills => self.render_settings_skills(cx).into_any_element(),
            SettingsTab::Appearance => self.render_settings_appearance(cx).into_any_element(),
            SettingsTab::About => render_settings_about().into_any_element(),
            SettingsTab::Archive => self.render_settings_archive(cx).into_any_element(),
            SettingsTab::Worktrees => self.render_settings_worktrees(cx).into_any_element(),
        }
    }

    /// MonoCode `ArchivePage` (projects): Restore puts one back on the rail
    /// and opens it; Delete asks first.
    fn render_settings_archive(&self, cx: &Context<Self>) -> impl IntoElement {
        let archived = &self.settings.rail.archived_projects;
        let section = SettingsSection::new("Archive").description("Projects and conversations you have archived.");
        if archived.is_empty() {
            return section.row(SettingsRow::new("No archived projects"));
        }
        archived.iter().enumerate().fold(section, |section, (ix, project)| {
            let (restore_path, delete_path) = (project.path.clone(), project.path.clone());
            section.row(
                SettingsRow::new(self.rail_project_label(&project.path))
                    .description(project.path.clone())
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                ely_gpui_component::buttons::Button::new(
                                    SharedString::from(format!("archive-restore-{ix}")),
                                    "Restore",
                                )
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.restore_rail_project(&restore_path, cx);
                                })),
                            )
                            .child(
                                ely_gpui_component::buttons::Button::new(
                                    SharedString::from(format!("archive-delete-{ix}")),
                                    "Delete",
                                )
                                .variant(ButtonVariant::Danger)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_remove_project(&delete_path, cx);
                                })),
                            ),
                    ),
            )
        })
    }

    fn render_settings_general(&self, cx: &Context<Self>) -> impl IntoElement {
        let editor_controls = match &self.integrations.editors {
            None => div().child(Badge::new("Scanning…").tone(Tone::Neutral)),
            Some(editors) if editors.is_empty() => {
                div().child(Badge::new("None detected").tone(Tone::Neutral))
            }
            Some(editors) => div().flex().items_center().gap_1().children(
                editors
                    .iter()
                    .map(|ed| Badge::new(ed.name).tone(Tone::Success).dot()),
            ),
        };

        SettingsSection::new("General")
            .description("Defaults for new threads and workspace tools.")
            .row(
                SettingsRow::new("Default model")
                    .description("Model used when a new thread starts")
                    .control(Badge::new(crate::harness::catalog::label_for(
                        &self.selected_model,
                    ))),
            )
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Composer mascot")
                    .description(
                        "When a turn is running, the project mascot runs along the composer, \
                         bonks the scroll-to-latest button the first time, then jumps it, and \
                         sometimes grabs a coin.",
                    )
                    .control(
                        Switch::new("composer-mascot", !self.composer_mascot_off).on_change(
                            move |on, _, cx| {
                                if let Err(err) =
                                    entity.update(cx, |this, cx| this.set_composer_mascot(on, cx))
                                {
                                    log::debug!("mascot toggle after app drop: {err:#}");
                                }
                            },
                        ),
                    )
            })
            .row(
                SettingsRow::new("External editors")
                    .description(
                        "Detected IDEs available to open files and workspaces; the first is used",
                    )
                    .control(editor_controls),
            )
    }

    fn render_settings_providers(&self, cx: &Context<Self>) -> impl IntoElement {
        let section = SettingsSection::new("Providers")
            .description("Agent CLIs found when BenCode started. Sign in through each CLI.");
        let providers = if self.harnesses.is_empty() {
            section.row(SettingsRow::new("No harness CLIs detected"))
        } else {
            self.harnesses
                .iter()
                .fold(section, |section, info| section.row(provider_row(info, cx)))
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.render_settings_accounts(cx))
            .child(providers)
            .child(self.render_settings_advanced(cx))
    }

    /// MonoCode Providers › Advanced.
    fn render_settings_advanced(&self, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        SettingsSection::new("Advanced").row(
            SettingsRow::new("Claude Code hooks")
                .description("Run hooks from your Claude settings. Applies from the next turn.")
                .control(
                    Switch::new("claude-hooks", !self.claude_hooks_disabled).on_change(
                        move |on, _, cx| {
                            if let Err(err) =
                                entity.update(cx, |this, cx| this.set_claude_hooks(on, cx))
                            {
                                log::debug!("hooks toggle after app drop: {err:#}");
                            }
                        },
                    ),
                ),
        )
    }

    fn render_settings_mcp(&self) -> impl IntoElement {
        let Some(servers) = &self.integrations.mcp_servers else {
            return div()
                .child(Badge::new("Scanning MCP configuration…").tone(Tone::Neutral))
                .into_any_element();
        };
        if servers.is_empty() {
            return div()
                .child(
                    EmptyState::new("mcp-empty", IconName::SlidersHorizontal, "No MCP servers configured")
                        .body("Add servers to ~/.cursor/mcp.json, Claude Desktop or .bencode/mcp.json to see them here."),
                )
                .into_any_element();
        }
        let section = SettingsSection::new("Model Context Protocol (MCP)")
            .description("Servers found in Claude, Cursor and project configuration. BenCode lists them; it does not start them.");
        servers
            .iter()
            .fold(section, |section, server| section.row(mcp_row(server)))
            .into_any_element()
    }
}

fn mcp_row(server: &crate::mcp::McpConnection) -> SettingsRow {
    let (status, tone) = if server.enabled {
        ("Enabled", Tone::Success)
    } else {
        ("Disabled", Tone::Neutral)
    };
    SettingsRow::new(server.name.clone())
        .description(format!(
            "{} ({}) • {}",
            server.provider, server.scope, server.config_path
        ))
        .control(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(HarnessIcon::new(&server.provider).size(px(14.0)))
                .child(Badge::new(server.transport.clone()).tone(Tone::Neutral))
                .child(Badge::new(status).tone(tone).dot()),
        )
}

fn provider_row(info: &HarnessInfo, _cx: &App) -> SettingsRow {
    let location = info.binary_path.as_ref().map_or_else(
        || "Not found on PATH".to_string(),
        |path| path.display().to_string(),
    );
    let status = if info.available {
        Badge::new("Available").tone(Tone::Success).dot()
    } else {
        Badge::new("Not installed").tone(Tone::Neutral)
    };
    SettingsRow::new(info.name).description(location).control(
        div()
            .flex()
            .items_center()
            .gap_2p5()
            .child(HarnessIcon::new(info.id).size(px(16.0)))
            .child(status),
    )
}

fn render_settings_about() -> impl IntoElement {
    SettingsSection::new("About BenCode")
        .description(env!("CARGO_PKG_DESCRIPTION"))
        .row(SettingsRow::new("Version").control(Badge::new(env!("CARGO_PKG_VERSION"))))
        .row(SettingsRow::new("License").control(Badge::new(env!("CARGO_PKG_LICENSE"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_keys_round_trip() {
        for (tab, key, ..) in SECTIONS {
            assert_eq!(tab.key(), key);
            assert_eq!(SettingsTab::from_key(key), Some(tab));
        }
        assert_eq!(SettingsTab::from_key("nope"), None);
    }
}
