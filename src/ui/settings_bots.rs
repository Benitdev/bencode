//! MonoCode Settings › Monos (`SettingsView`'s `MonosPage`): Bots on or
//! off, pinned to the icon rail or not, the menu bar icon and (BenCode's
//! own) the shortcut that summons one from any app, then each Bot with
//! Reset Bot (its soul and name back to their defaults) and the switch
//! showing the sessions it starts in the sidebar.

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::forms::{Choice, Select, Switch};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::ControlSize;
use gpui::{Context, IntoElement, ParentElement as _, SharedString, Styled as _};

use crate::app::BenCodeApp;
use crate::bots::projects_phrase;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow};

impl BenCodeApp {
    pub(crate) fn render_settings_bots(&self, cx: &Context<Self>) -> SettingsPage {
        let enabled = self.bots_enabled();
        let toggle =
            |id: &'static str,
             on: bool,
             set: fn(&mut BenCodeApp, bool, &mut Context<BenCodeApp>)| {
                let entity = cx.entity().downgrade();
                Switch::new(id, on).on_change(move |on, _, cx| {
                    let toggled = entity.update(cx, |this, cx| {
                        set(this, on, cx);
                        this.play_cue(crate::sounds::Cue::Switch);
                    });
                    if let Err(err) = toggled {
                        log::debug!("{id} toggle after app drop: {err:#}");
                    }
                })
            };
        let switches = SettingsGroup::new("Bots")
            .row(
                SettingsRow::new("Show bots")
                    .description(
                        "Agents of your own on the project rail. Each works on the projects you \
                         give it, remembers what matters and picks up habits it runs on its own. \
                         Turn this off to hide them.",
                    )
                    .control(toggle(
                        "bots-enabled",
                        enabled,
                        BenCodeApp::set_bots_enabled,
                    )),
            )
            .row(
                SettingsRow::new("Pin bots to the icon rail")
                    .description(
                        "When the project rail is collapsed to icons, show each bot at the top \
                         of the rail, above a divider, instead of inside the project picker.",
                    )
                    .control(
                        toggle(
                            "rail-bots-pinned",
                            self.settings.rail_bots_pinned,
                            BenCodeApp::set_rail_bots_pinned,
                        )
                        .disabled(!enabled),
                    ),
            );
        let switches = if cfg!(target_os = "macos") {
            let entity = cx.entity().downgrade();
            let summon = crate::app::summon::summon_key(self.settings.summon_key.as_deref())
                .map_or(crate::app::summon::SUMMON_OFF, |key| key.id);
            let keys = crate::app::summon::SUMMON_KEYS
                .iter()
                .map(|key| Choice::new(key.id, key.label))
                .chain([Choice::new(crate::app::summon::SUMMON_OFF, "Off")]);
            let picker = Select::new("bot-summon-key", keys)
                .size(ControlSize::Sm)
                .selected(summon)
                .disabled(!enabled)
                .on_change(move |value, _, cx| {
                    let value = value.to_string();
                    if let Err(err) = entity.update(cx, |this, cx| this.set_summon_key(&value, cx))
                    {
                        log::debug!("summon key after app drop: {err:#}");
                    }
                });
            let summon_row = SettingsRow::new("Summon from anywhere")
                .description(self.summon.error.clone().unwrap_or_else(|| {
                    "Opens the floating chat over any app, on the bot you last chatted with. \
                     Press it again to hide the chat."
                        .to_string()
                }))
                .control(picker);
            switches.row(summon_row).row(
                SettingsRow::new("Menu bar icon")
                    .description(
                        "Chat with a bot from the macOS menu bar. Turn this off to hide \
                             the icon.",
                    )
                    .control(
                        toggle(
                            "bot-menu-bar-icon",
                            !self.settings.bot_menu_bar_off,
                            BenCodeApp::set_bot_menu_bar,
                        )
                        .disabled(!enabled),
                    ),
            )
        } else {
            switches
        };
        let list = SettingsGroup::new("Your bots").description(
            "Add a bot with the plus on the rail and choose its projects from its settings.",
        );
        let list = if self.settings.bots.is_empty() {
            list.note("No bots yet.")
        } else {
            list.rows(self.settings.bots.iter().map(|bot| {
                let names: Vec<String> = self
                    .bot_projects(bot)
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect();
                let description = if names.is_empty() {
                    "No projects yet".to_string()
                } else {
                    format!("Works on {}", projects_phrase(&names))
                };
                let id = bot.id.clone();
                let name = bot.display_name();
                let (entity, shown_id) = (cx.entity().downgrade(), bot.id.clone());
                let shown = Switch::new(
                    SharedString::from(format!("bot-started-sidebar-{}", bot.id)),
                    bot.show_started_sessions_in_sidebar,
                )
                .on_change(move |on, _, cx| {
                    let shown_id = shown_id.clone();
                    let toggled = entity.update(cx, |this, cx| {
                        this.update_bots(
                            |roster| {
                                if let Some(entry) = roster.iter_mut().find(|m| m.id == shown_id) {
                                    entry.show_started_sessions_in_sidebar = on;
                                }
                            },
                            cx,
                        );
                        this.play_cue(crate::sounds::Cue::Switch);
                    });
                    if let Err(err) = toggled {
                        log::debug!("bot sidebar switch after app drop: {err:#}");
                    }
                });
                SettingsRow::new(name.clone())
                    .leading(crate::ui::figure::figure_still(
                        bot.figure(),
                        16.0,
                        self.bot_color(bot),
                    ))
                    .description(description)
                    .control(
                        gpui::div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(shown)
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("bot-defaults-{}", bot.id)),
                                    IconName::RotateCcw,
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip(format!("Reset {name} to defaults"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.bots.reset_error = None;
                                        this.bots.pending_defaults = Some(id.clone());
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            }))
        };
        SettingsTab::Bots.page().group(switches).group(list)
    }
}
