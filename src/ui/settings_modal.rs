//! Settings: general defaults, provider CLIs, MCP, skills, integrations,
//! appearance, about.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::forms::Switch;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::settings::SettingsLayout;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};

use crate::app::BenCodeApp;
use crate::harness::HarnessInfo;
use crate::ui::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow, icon_tile};
use crate::ui::sidebar_popovers::pretty_path;

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
    /// Trackers the Inbox reads (`ui/settings_integrations.rs`).
    Integrations,
}

const SECTIONS: [(SettingsTab, &str, &str, IconName); 9] = [
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
    (
        SettingsTab::Archive,
        "archive",
        "Archive",
        IconName::Archive,
    ),
    // MonoCode's nav uses `FolderTree`, which Ely's `IconName` lacks.
    (
        SettingsTab::Worktrees,
        "worktrees",
        "Worktrees",
        IconName::GitBranch,
    ),
    (
        SettingsTab::Integrations,
        "integrations",
        "Integrations",
        IconName::Inbox,
    ),
];

/// MonoCode `SETTINGS_GROUPS` with BenCode's sections in them (About is
/// BenCode's own and sits with App), for the rail's settings nav.
pub(crate) const SETTINGS_GROUPS: [(&str, &[SettingsTab]); 3] = [
    (
        "App",
        &[
            SettingsTab::General,
            SettingsTab::Appearance,
            SettingsTab::About,
        ],
    ),
    (
        "Agents",
        &[
            SettingsTab::Providers,
            SettingsTab::Mcp,
            SettingsTab::Skills,
        ],
    ),
    (
        "Workspace",
        &[
            SettingsTab::Archive,
            SettingsTab::Worktrees,
            SettingsTab::Integrations,
        ],
    ),
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
            .map_or(("General", IconName::Settings), |(_, _, name, icon)| {
                (*name, *icon)
            })
    }

    /// MonoCode `settingsSectionDescription`: the line under the page title.
    fn description(self) -> &'static str {
        match self {
            Self::General => {
                "The model new threads start with, the panels BenCode shows, and the editors it \
                 opens files in."
            }
            Self::Appearance => {
                "Theme, tint, translucency, workspace layout, and conversation backgrounds."
            }
            Self::About => "The build you are running and how it stays up to date.",
            Self::Providers => {
                "Provider accounts, the agent CLIs BenCode drives, and how they run."
            }
            Self::Mcp => "MCP servers found in Claude, Cursor and project configuration.",
            Self::Skills => {
                "File skills from project and personal folders. Type / in the composer to use one."
            }
            Self::Archive => "Projects you have archived from the rail.",
            Self::Worktrees => "Manage additional worktrees for each project.",
            Self::Integrations => {
                "Where the Inbox reads from: GitHub through its CLI, and a Nulab Backlog space."
            }
        }
    }

    /// The page's header, ready for its groups.
    pub(crate) fn page(self) -> SettingsPage {
        SettingsPage::new(self.label_icon().0).description(self.description())
    }

    fn from_key(key: &str) -> Option<Self> {
        SECTIONS
            .iter()
            .find(|(_, k, ..)| *k == key)
            .map(|(tab, ..)| *tab)
    }
}

impl BenCodeApp {
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
            SettingsTab::Mcp => self.render_settings_mcp(cx).into_any_element(),
            SettingsTab::Skills => self.render_settings_skills(cx).into_any_element(),
            SettingsTab::Appearance => self.render_settings_appearance(cx).into_any_element(),
            SettingsTab::About => self.render_settings_about(cx).into_any_element(),
            SettingsTab::Archive => self.render_settings_archive(cx).into_any_element(),
            SettingsTab::Worktrees => self.render_settings_worktrees(cx).into_any_element(),
            SettingsTab::Integrations => self.render_settings_integrations(cx).into_any_element(),
        }
    }

    fn render_settings_general(&self, cx: &Context<Self>) -> SettingsPage {
        let model = &self.selected_model;
        let harness = model.split_once(':').map_or("", |(harness, _)| harness);
        let default_model = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.0))
            .text_color(cx.theme().colors.fg)
            .child(HarnessIcon::new(harness).size(px(14.0)))
            .child(crate::harness::catalog::label_for(model));
        let threads = SettingsGroup::new("New threads").row(
            SettingsRow::new("Default model")
                .description("The model a new thread starts with.")
                .control(default_model),
        );

        let workspace = SettingsGroup::new("Workspace")
            .description("What BenCode shows around your chats.")
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Working agents")
                    .description(
                        "When two or more chats are in flight, a card on the project rail lists \
                         them so you can jump across projects. Finished turns stay until you \
                         open that session.",
                    )
                    .control(
                        Switch::new("working-agents", !self.live_agents_off).on_change(
                            move |on, _, cx| {
                                if let Err(err) =
                                    entity.update(cx, |this, cx| this.set_live_agents(on, cx))
                                {
                                    log::debug!("working agents toggle after app drop: {err:#}");
                                }
                            },
                        ),
                    )
            })
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
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Resume interrupted chats")
                    .description(
                        "When a quit, an update's restart or a crash cuts a turn off, carry it on \
                         at the next launch without asking first.",
                    )
                    .control(
                        Switch::new("resume-interrupted", self.resume_interrupted_auto).on_change(
                            move |on, _, cx| {
                                if let Err(err) = entity
                                    .update(cx, |this, cx| this.set_resume_interrupted_auto(on, cx))
                                {
                                    log::debug!("resume toggle after app drop: {err:#}");
                                }
                            },
                        ),
                    )
            });

        let editors = SettingsGroup::new("External editors")
            .description("IDEs found on this Mac. Files and workspaces open in the first one.");
        let editors = match &self.integrations.editors {
            None => editors.note("Looking for editors…"),
            Some(found) if found.is_empty() => editors.note(
                "No supported editor found. Install VS Code, Cursor or Zed to open files outside BenCode.",
            ),
            Some(found) => editors.rows(found.iter().enumerate().map(|(ix, editor)| {
                SettingsRow::new(editor.name).control(if ix == 0 {
                    Badge::new("Default").tone(Tone::Accent)
                } else {
                    Badge::new("Installed").tone(Tone::Neutral)
                })
            })),
        };

        SettingsTab::General
            .page()
            .group(threads)
            .group(workspace)
            .group(editors)
    }

    fn render_settings_providers(&self, cx: &Context<Self>) -> SettingsPage {
        let clis = SettingsGroup::new("Agent CLIs")
            .description("Found on PATH when BenCode started. Sign in through each CLI.");
        let clis = if self.harnesses.is_empty() {
            clis.note("No agent CLIs found. Install Claude Code, Codex, OpenCode or Antigravity, then restart BenCode.")
        } else {
            clis.rows(self.harnesses.iter().map(|info| provider_row(info, cx)))
        };
        let entity = cx.entity().downgrade();
        let advanced = SettingsGroup::new("Advanced").row(
            SettingsRow::new("Claude Code hooks")
                .description(
                    "Run the hooks from your Claude settings, as the Claude Code CLI would. Turn \
                     this off if a hook is misbehaving. Applies from the next turn.",
                )
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
        );
        SettingsTab::Providers
            .page()
            .group(self.render_settings_accounts(cx))
            .group(clis)
            .group(advanced)
    }

    fn render_settings_mcp(&self, cx: &Context<Self>) -> SettingsPage {
        let fg = cx.theme().colors.fg;
        let group = SettingsGroup::new("Servers")
            .description("BenCode lists these servers; each agent CLI starts its own.");
        let group = match &self.integrations.mcp_servers {
            None => group.note("Reading MCP configuration…"),
            Some(servers) if servers.is_empty() => group.note(
                "No MCP servers configured. Add servers to ~/.cursor/mcp.json, Claude Desktop or \
                 .bencode/mcp.json to see them here.",
            ),
            Some(servers) => group
                .action(Badge::new(plural(servers.len(), "server", "servers")))
                .rows(servers.iter().map(|server| mcp_row(server, fg))),
        };
        SettingsTab::Mcp.page().group(group)
    }

    /// Skills found in the project and user folders (MonoCode `SkillsPage`).
    fn render_settings_skills(&self, cx: &Context<Self>) -> SettingsPage {
        let skills = &self.integrations.skills;
        let group = SettingsGroup::new(plural(skills.len(), "skill", "skills"))
            .description("Add a folder with a SKILL.md under .agents/skills, in a project or your home folder.")
            .action(
                IconButton::new("skills-rescan", IconName::RefreshCw)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Rescan skill folders")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_skills(true, cx))),
            );
        let group = if skills.is_empty() {
            group.note("No skills found yet.")
        } else {
            group.rows(skills.iter().map(|skill| {
                let detail = if skill.path.is_empty() {
                    skill.description.clone()
                } else {
                    format!("{}\n{}", skill.description, pretty_path(&skill.path))
                };
                SettingsRow::new(format!("/{}", skill.name))
                    .description(detail)
                    .control(Badge::new(format!("{} · {}", skill.scope, skill.source)))
            }))
        };
        SettingsTab::Skills.page().group(group)
    }

    /// MonoCode `ArchivePage` (projects): Restore puts one back on the rail
    /// and opens it; Delete asks first.
    fn render_settings_archive(&self, cx: &Context<Self>) -> SettingsPage {
        let fg = cx.theme().colors.fg;
        let archived = &self.settings.rail.archived_projects;
        let group = SettingsGroup::new("Archived projects").description(
            "Archive a project from the rail to keep its chats without listing it in the sidebar.",
        );
        let group = if archived.is_empty() {
            group.note("No archived projects.")
        } else {
            group.rows(archived.iter().enumerate().map(|(ix, project)| {
                let (restore_path, delete_path) = (project.path.clone(), project.path.clone());
                SettingsRow::new(self.rail_project_label(&project.path))
                    .leading(icon_tile(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.6)),
                        fg,
                    ))
                    .description(pretty_path(&project.path))
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(
                                    SharedString::from(format!("archive-restore-{ix}")),
                                    "Restore",
                                )
                                .variant(ButtonVariant::Outline)
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.restore_rail_project(&restore_path, cx);
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("archive-delete-{ix}")),
                                    IconName::Trash2,
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .tooltip("Delete project")
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_remove_project(&delete_path, cx);
                                    },
                                )),
                            ),
                    )
            }))
        };
        SettingsTab::Archive.page().group(group)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn mcp_row(server: &crate::mcp::McpConnection, fg: gpui::Hsla) -> SettingsRow {
    let (status, tone) = if server.enabled {
        ("Enabled", Tone::Success)
    } else {
        ("Disabled", Tone::Neutral)
    };
    SettingsRow::new(server.name.clone())
        .leading(icon_tile(
            HarnessIcon::new(&server.provider).size(px(14.0)),
            fg,
        ))
        .description(format!(
            "{} · {} · {}",
            server.provider,
            server.scope,
            pretty_path(&server.config_path)
        ))
        .control(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Badge::new(server.transport.clone()).tone(Tone::Neutral))
                .child(Badge::new(status).tone(tone).dot()),
        )
}

fn provider_row(info: &HarnessInfo, cx: &Context<BenCodeApp>) -> SettingsRow {
    let location = info.binary_path.as_ref().map_or_else(
        || "Not found on PATH".to_string(),
        |path| pretty_path(&path.display().to_string()),
    );
    let status = if info.available {
        Badge::new("Available").tone(Tone::Success).dot()
    } else {
        Badge::new("Not installed").tone(Tone::Neutral)
    };
    SettingsRow::new(info.name)
        .leading(icon_tile(
            HarnessIcon::new(info.id).size(px(16.0)),
            cx.theme().colors.fg,
        ))
        .description(location)
        .control(status)
}

impl BenCodeApp {
    /// About, with MonoCode's `UpdateRow`: the version, what the updater
    /// last found, What's new, and Check for updates / Download (Restart
    /// once an update waits for one).
    fn render_settings_about(&self, cx: &Context<Self>) -> SettingsPage {
        use crate::app::updater::Phase;
        let state = &self.updater;
        let busy = matches!(state.phase, Phase::Checking | Phase::Downloading);
        let (label, icon) = match state.phase {
            Phase::Available => ("Download", IconName::Download),
            Phase::Ready => ("Restart", IconName::RefreshCw),
            _ => ("Check for updates", IconName::RefreshCw),
        };
        let current = state.current_version().to_string();
        let controls = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("about-whats-new", "What's new")
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_whats_new(current.clone(), cx)),
                    ),
            )
            .child(
                Button::new("about-update", label)
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .icon(icon)
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| match this.updater.phase {
                        Phase::Available | Phase::Ready => this.install_update(cx),
                        _ => this.check_for_updates(window, cx),
                    })),
            );
        let app = SettingsGroup::new("BenCode")
            .description(env!("CARGO_PKG_DESCRIPTION"))
            .row(
                SettingsRow::new("Version")
                    .aside(state.current_version().to_string())
                    .description(state.status_line())
                    .control(controls),
            )
            .row(SettingsRow::new("License").control(Badge::new(env!("CARGO_PKG_LICENSE"))));
        SettingsTab::About.page().group(app)
    }
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
