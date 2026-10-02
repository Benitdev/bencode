//! Settings: general defaults, provider CLIs, MCP, skills, appearance, about.

use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::Switch;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::settings::{SettingsLayout, SettingsRow, SettingsSection};
use ely_gpui_component::theme::{ActiveTheme, Mode, Theme};
use ely_gpui_component::typography::Code;
use gpui::{AnyElement, App, Context, IntoElement, ParentElement, SharedString, Styled, div};

use crate::app::BenCodeApp;
use crate::harness::HarnessInfo;
use crate::ui::app_callback::app_callback;
use crate::ui::theme::harness_color;
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
    (SettingsTab::General, "general", "General", IconName::Settings),
    (SettingsTab::Providers, "providers", "Providers", IconName::Key),
    (SettingsTab::Mcp, "mcp", "MCP servers", IconName::SlidersHorizontal),
    (SettingsTab::Skills, "skills", "Skills", IconName::Sparkles),
    (SettingsTab::Appearance, "appearance", "Appearance", IconName::Palette),
    (SettingsTab::About, "about", "About", IconName::Info),
];

impl SettingsTab {
    fn key(self) -> &'static str {
        SECTIONS.iter().find(|(tab, ..)| *tab == self).map_or("general", |(_, key, ..)| key)
    }

    fn from_key(key: &str) -> Option<Self> {
        SECTIONS.iter().find(|(_, k, ..)| *k == key).map(|(tab, ..)| *tab)
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
            SECTIONS.iter().map(|(_, key, name, icon)| (*key, *name, *icon)),
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
        Dialog::new("settings", "Settings", close).fullscreen().child(layout)
    }

    fn render_settings_page(&self, cx: &App) -> AnyElement {
        match self.settings_tab {
            SettingsTab::General => self.render_settings_general().into_any_element(),
            SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
            SettingsTab::Mcp => render_settings_mcp().into_any_element(),
            SettingsTab::Skills => render_settings_skills().into_any_element(),
            SettingsTab::Appearance => render_settings_appearance(cx).into_any_element(),
            SettingsTab::About => render_settings_about().into_any_element(),
        }
    }

    fn render_settings_general(&self) -> impl IntoElement {
        SettingsSection::new("General").description("Defaults for new threads.").row(
            SettingsRow::new("Default model")
                .description("Model key used when a new thread starts")
                .control(Badge::new(self.selected_model.clone())),
        )
    }

    fn render_settings_providers(&self, cx: &App) -> impl IntoElement {
        let section = SettingsSection::new("Providers")
            .description("Agent CLIs found when BenCode started. Sign in through each CLI.");
        if self.harnesses.is_empty() {
            return section.row(SettingsRow::new("No harness CLIs detected"));
        }
        self.harnesses.iter().fold(section, |section, info| section.row(provider_row(info, cx)))
    }
}

fn provider_row(info: &HarnessInfo, cx: &App) -> SettingsRow {
    let theme = cx.theme();
    let location = info
        .binary_path
        .as_ref()
        .map_or_else(|| "Not found on PATH".to_string(), |path| path.display().to_string());
    let status = if info.available {
        Badge::new("Available").tone(Tone::Success).dot()
    } else {
        Badge::new("Not installed").tone(Tone::Neutral)
    };
    SettingsRow::new(info.name).description(location).control(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(theme.status_dot()).rounded_full().bg(harness_color(info.id, &theme.colors)))
            .child(status),
    )
}

fn render_settings_mcp() -> impl IntoElement {
    EmptyState::new("mcp-empty", IconName::SlidersHorizontal, "No MCP servers configured in BenCode")
        .body("BenCode does not manage MCP servers yet. Servers configured in each harness CLI still apply to its runs.")
}

fn render_settings_skills() -> impl IntoElement {
    BUILTIN_SKILLS.iter().fold(
        SettingsSection::new("Skills").description("Built-in skills. Type / in the composer to use one."),
        |section, skill| {
            section.row(
                SettingsRow::new(skill.name)
                    .description(skill.description)
                    .control(Code::new(skill.example)),
            )
        },
    )
}

fn render_settings_appearance(cx: &App) -> impl IntoElement {
    SettingsSection::new("Appearance").row(
        SettingsRow::new("Dark mode")
            .description("Use MonoCode's dark palette; turn off for light")
            .control(Switch::new("appearance-dark-mode", cx.theme().is_dark()).on_change(|dark, _, cx| {
                Theme::set_mode(if dark { Mode::Dark } else { Mode::Light }, cx);
            })),
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
