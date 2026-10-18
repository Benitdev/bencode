//! MonoCode Settings › Monos (`SettingsView`'s `MonosPage`): Monos on or
//! off, pinned to the icon rail or not, then each Mono with Reset Mono
//! (its soul and name back to their defaults) and the switch showing the
//! sessions it starts in the sidebar.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::forms::Switch;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ControlSize;
use gpui::{Context, IntoElement, ParentElement as _, SharedString, Styled as _};

use crate::app::BenCodeApp;
use crate::monos::projects_phrase;
use crate::ui::mascot::pixel_sprite;
use crate::ui::scale::px;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow};

impl BenCodeApp {
    pub(crate) fn render_settings_monos(&self, cx: &Context<Self>) -> SettingsPage {
        let enabled = self.monos_enabled();
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
        let switches = SettingsGroup::new("Monos")
            .row(
                SettingsRow::new("Show monos")
                    .description(
                        "Agents of your own on the project rail. Each works on the projects you \
                         give it, remembers what matters and picks up habits it runs on its own. \
                         Turn this off to hide them.",
                    )
                    .control(toggle(
                        "monos-enabled",
                        enabled,
                        BenCodeApp::set_monos_enabled,
                    )),
            )
            .row(
                SettingsRow::new("Pin monos to the icon rail")
                    .description(
                        "When the project rail is collapsed to icons, show each Mono at the top \
                         of the rail, above a divider, instead of inside the project picker.",
                    )
                    .control(
                        toggle(
                            "rail-monos-pinned",
                            self.settings.rail_monos_pinned,
                            BenCodeApp::set_rail_monos_pinned,
                        )
                        .disabled(!enabled),
                    ),
            );
        let switches = if cfg!(target_os = "macos") {
            switches.row(
                SettingsRow::new("Menu bar icon")
                    .description(
                        "Chat with a Mono from the macOS menu bar. Turn this off to hide \
                             the icon.",
                    )
                    .control(
                        toggle(
                            "mono-menu-bar-icon",
                            !self.settings.mono_menu_bar_off,
                            BenCodeApp::set_mono_menu_bar,
                        )
                        .disabled(!enabled),
                    ),
            )
        } else {
            switches
        };
        let list = SettingsGroup::new("Your monos").description(
            "Add a Mono with the plus on the rail and choose its projects from its settings.",
        );
        let list = if self.settings.monos.is_empty() {
            list.note("No monos yet.")
        } else {
            list.rows(self.settings.monos.iter().map(|mono| {
                let names: Vec<String> = self
                    .mono_projects(mono)
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect();
                let description = if names.is_empty() {
                    "No projects yet".to_string()
                } else {
                    format!("Works on {}", projects_phrase(&names))
                };
                let id = mono.id.clone();
                let name = mono.display_name();
                let (entity, shown_id) = (cx.entity().downgrade(), mono.id.clone());
                let shown = Switch::new(
                    SharedString::from(format!("mono-started-sidebar-{}", mono.id)),
                    mono.show_started_sessions_in_sidebar,
                )
                .on_change(move |on, _, cx| {
                    let shown_id = shown_id.clone();
                    let toggled = entity.update(cx, |this, cx| {
                        this.update_monos(
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
                        log::debug!("Mono sidebar switch after app drop: {err:#}");
                    }
                });
                SettingsRow::new(name.clone())
                    .leading(pixel_sprite(
                        &self.mono_mascot(mono).rest,
                        px(16.0),
                        self.mono_color(mono),
                        false,
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
                                    SharedString::from(format!("mono-defaults-{}", mono.id)),
                                    IconName::RotateCcw,
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip(format!("Reset {name} to defaults"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.monos.reset_error = None;
                                        this.monos.pending_defaults = Some(id.clone());
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            }))
        };
        SettingsTab::Monos.page().group(switches).group(list)
    }
}
