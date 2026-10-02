//! Settings: general defaults, provider CLIs, MCP, skills, appearance, about.

use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::Switch;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::settings::{SettingsLayout, SettingsRow, SettingsSection};
use ely_gpui_component::theme::{ActiveTheme, Mode};
use ely_gpui_component::typography::Code;
use gpui::{AnyElement, App, Context, IntoElement, ParentElement, SharedString, Styled, div, px};

use crate::app::BenCodeApp;
use crate::harness::HarnessInfo;
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;
use crate::workspace::BUILTIN_SKILLS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    General,
    #[default]
    Providers,
    Mcp,
    Skills,
    Appearance,
    About,
}

const SECTIONS: [(SettingsTab, &str, &str, IconName); 6] = [
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
];

impl SettingsTab {
    fn key(self) -> &'static str {
        SECTIONS
            .iter()
            .find(|(tab, ..)| *tab == self)
            .map_or("general", |(_, key, ..)| key)
    }

    fn from_key(key: &str) -> Option<Self> {
        SECTIONS
            .iter()
            .find(|(_, k, ..)| *k == key)
            .map(|(tab, ..)| *tab)
    }
}

impl BenCodeApp {
    pub fn render_settings_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let close = app_callback(cx, |this, cx| {
            this.is_settings_open = false;
            cx.notify();
        });
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
                Some(tab) => this.settings_tab = tab,
                None => log::warn!("unknown settings section {key}"),
            }
            cx.notify();
        }));
        Dialog::new("settings", "Settings", close)
            .fullscreen()
            .child(layout)
    }

    fn render_settings_page(&self, cx: &Context<Self>) -> AnyElement {
        match self.settings_tab {
            SettingsTab::General => self.render_settings_general().into_any_element(),
            SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
            SettingsTab::Mcp => self.render_settings_mcp().into_any_element(),
            SettingsTab::Skills => render_settings_skills().into_any_element(),
            SettingsTab::Appearance => render_settings_appearance(cx).into_any_element(),
            SettingsTab::About => render_settings_about().into_any_element(),
        }
    }

    fn render_settings_general(&self) -> impl IntoElement {
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
            .row(
                SettingsRow::new("External editors")
                    .description(
                        "Detected IDEs available to open files and workspaces; the first is used",
                    )
                    .control(editor_controls),
            )
    }

    fn render_settings_providers(&self, cx: &App) -> impl IntoElement {
        let section = SettingsSection::new("Providers")
            .description("Agent CLIs found when BenCode started. Sign in through each CLI.");
        if self.harnesses.is_empty() {
            return section.row(SettingsRow::new("No harness CLIs detected"));
        }
        self.harnesses
            .iter()
            .fold(section, |section, info| section.row(provider_row(info, cx)))
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

fn render_settings_skills() -> impl IntoElement {
    BUILTIN_SKILLS.iter().fold(
        SettingsSection::new("Skills")
            .description("Built-in skills. Type / in the composer to use one."),
        |section, skill| {
            section.row(
                SettingsRow::new(skill.name)
                    .description(skill.description)
                    .control(Code::new(skill.example)),
            )
        },
    )
}

fn render_settings_appearance(cx: &Context<BenCodeApp>) -> impl IntoElement {
    let app = cx.entity().downgrade();
    SettingsSection::new("Appearance").row(
        SettingsRow::new("Dark mode")
            .description("Use MonoCode's dark palette; turn off for light")
            .control(
                Switch::new("appearance-dark-mode", cx.theme().is_dark()).on_change(
                    move |dark, _, cx| {
                        let mode = if dark { Mode::Dark } else { Mode::Light };
                        let _ = app.update(cx, |this, cx| this.set_theme_mode(mode, cx));
                    },
                ),
            ),
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
