//! Settings dialog: general defaults, provider CLIs, MCP, skills,
//! integrations, appearance, shortcuts, about.
//!
//! Drawn in a bencode-ui layer (`Backdrop`), with the grouped section nav on
//! the left and the active section scrolling on the right (matching
//! `ui/bot_settings.rs`). What opens inside it is raised in that layer, never
//! deferred.

use bencode_ui::buttons::{Button, ButtonVariant, IconButton};
use bencode_ui::data_display::{Badge, Tone};
use bencode_ui::forms::Switch;
use bencode_ui::primitives::{Backdrop, Icon, IconName};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::harness::HarnessInfo;
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::settings_appearance::ColorScope;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow, icon_tile};
use crate::ui::sidebar_popovers::pretty_path;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

const DIALOG_WIDTH: f32 = 980.0;
const DIALOG_HEIGHT: f32 = 720.0;
const NAV_WIDTH: f32 = 208.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    General,
    /// MonoCode Settings › Connections: machines (`ui/settings_connections.rs`).
    Connections,
    #[default]
    Providers,
    Mcp,
    Skills,
    Appearance,
    /// The keys BenCode answers to, read only (`ui/settings_shortcuts.rs`).
    Shortcuts,
    About,
    /// MonoCode Settings › Archive: archived projects.
    Archive,
    /// MonoCode Settings › Worktrees (`ui/settings_worktrees.rs`).
    Worktrees,
    /// Trackers the Inbox reads (`ui/settings_integrations.rs`).
    Integrations,
    /// MonoCode Settings › Bots (`ui/settings_bots.rs`).
    Bots,
}

const SECTIONS: [(SettingsTab, &str, IconName); 12] = [
    (SettingsTab::General, "General", IconName::Settings),
    (SettingsTab::Connections, "Connections", IconName::Globe),
    (SettingsTab::Providers, "Providers", IconName::Key),
    (SettingsTab::Mcp, "MCP servers", IconName::SlidersHorizontal),
    (SettingsTab::Skills, "Skills", IconName::Sparkles),
    (SettingsTab::Bots, "Bots", IconName::Bot),
    (SettingsTab::Appearance, "Appearance", IconName::Palette),
    (SettingsTab::Shortcuts, "Shortcuts", IconName::Keyboard),
    (SettingsTab::About, "About", IconName::Info),
    (SettingsTab::Archive, "Archive", IconName::Archive),
    // MonoCode's nav uses `FolderTree`, which bencode-ui's `IconName` lacks.
    (SettingsTab::Worktrees, "Worktrees", IconName::GitBranch),
    (SettingsTab::Integrations, "Integrations", IconName::Inbox),
];

/// MonoCode `SETTINGS_GROUPS` with BenCode's sections in them (About is
/// BenCode's own and sits with App), for the dialog's left nav.
const SETTINGS_GROUPS: [(&str, &[SettingsTab]); 3] = [
    (
        "App",
        &[
            SettingsTab::General,
            SettingsTab::Connections,
            SettingsTab::Appearance,
            SettingsTab::Shortcuts,
            SettingsTab::About,
        ],
    ),
    (
        "Agents",
        &[
            SettingsTab::Providers,
            SettingsTab::Mcp,
            SettingsTab::Skills,
            SettingsTab::Bots,
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
    /// The nav's label and icon.
    pub(crate) fn label_icon(self) -> (&'static str, IconName) {
        SECTIONS
            .iter()
            .find(|(tab, ..)| *tab == self)
            .map_or(("General", IconName::Settings), |(_, name, icon)| {
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
            Self::Connections => "Connect your machines and run agents remotely through SSH.",
            Self::Appearance => {
                "Theme, tint, translucency, workspace layout, and conversation backgrounds."
            }
            Self::Shortcuts => "Keyboard shortcuts for quick navigation and control.",
            Self::About => "The build you are running and how it stays up to date.",
            Self::Providers => {
                "Provider accounts, the agent CLIs BenCode drives, and how they run."
            }
            Self::Mcp => "MCP servers found in Claude, Cursor and project configuration.",
            Self::Skills => {
                "File skills from project and personal folders. Type / in the composer to use one."
            }
            Self::Bots => "The resident agent beside your tabs, and which projects have one.",
            Self::Archive => "Projects you have archived from the navigator.",
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
}

impl BenCodeApp {
    /// Opens the Settings dialog on its current section.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        let opening = !self.settings_open;
        self.settings_open = true;
        if opening {
            self.load_settings_tab(self.settings_tab, cx);
        }
        cx.notify();
    }

    /// Opens the Settings dialog directly on `tab`.
    pub(crate) fn open_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.select_settings_tab(tab, cx);
    }

    /// Closes the Settings dialog and any popover it opened. Returns whether
    /// it was open.
    pub fn close_settings(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.settings_open {
            return false;
        }
        self.settings_open = false;
        if self.accent_picker == Some(ColorScope::App) {
            self.accent_picker = None;
        }
        if self.models_for_translation() {
            self.close_model_picker(cx);
        }
        cx.notify();
        true
    }

    /// Escape in the dialog: an open popover inside Settings closes first,
    /// then the dialog itself.
    pub fn close_settings_layer(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.settings_open {
            return false;
        }
        if self.accent_picker == Some(ColorScope::App) {
            self.accent_picker = None;
            cx.notify();
            return true;
        }
        if self.models_for_translation() {
            self.close_model_picker(cx);
            return true;
        }
        self.close_settings(cx)
    }

    pub(crate) fn select_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_tab = tab;
        if self.accent_picker == Some(ColorScope::App) {
            self.accent_picker = None;
        }
        if self.models_for_translation() {
            self.close_model_picker(cx);
        }
        self.load_settings_tab(tab, cx);
        cx.notify();
    }

    fn load_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        match tab {
            // MonoCode's page starts on the open project, freshly listed.
            SettingsTab::Worktrees => self.open_worktrees_page(cx),
            SettingsTab::Providers => self.load_accounts_page(false, cx),
            SettingsTab::Integrations => self.open_integrations_page(cx),
            // MonoCode checks each machine while the page shows.
            SettingsTab::Connections => {
                self.remote.checked = false;
                self.check_machines(cx);
            }
            _ => {}
        }
    }

    /// The Settings dialog, while `settings_open` is set. Full-window: in
    /// `BenCodeApp::render`'s `dialogs` list ahead of confirmations it can
    /// open.
    pub(crate) fn render_settings_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.settings_open {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let close = app_callback(cx, |this, cx| {
            this.close_settings(cx);
        });
        let close_button = IconButton::new("settings-close", IconName::X)
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .tooltip("Close (Esc)")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_settings(cx);
            }));
        let page = div()
            .id("settings-page")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .px_6()
                    .pt_5()
                    .pb_10()
                    .child(self.render_settings_page(cx)),
            );
        let card = div()
            .id("settings-dialog")
            .relative()
            .flex()
            .w(px(DIALOG_WIDTH))
            .max_w(relative(0.94))
            .h(px(DIALOG_HEIGHT))
            .max_h(relative(0.88))
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(fg.fill(Fill::Line))
            .shadow_2xl()
            .text_color(fg)
            .child(crate::ui::frost::dialog_frost(15.0, colors.bg, cx))
            .child(self.render_settings_nav(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_settings_header(cx))
                    .child(
                        div().flex_1().min_h_0().child(
                            crate::ui::scrollbar::Scrolled::new("settings-page-scrollbar", page),
                        ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(8.0))
                    .child(close_button),
            );
        Some(
            Backdrop::new("settings-scrim")
                .on_dismiss(close)
                .child(card)
                .into_any_element(),
        )
    }

    fn render_settings_nav(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let current = self.settings_tab;
        let groups = SETTINGS_GROUPS.iter().map(|(label, tabs)| {
            div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .px(px(10.0))
                        .pb_1()
                        .text_size(Text::Sm.px())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg.ink(Ink::Faint))
                        .child(*label),
                )
                .children(tabs.iter().map(|&tab| {
                    let (name, icon) = tab.label_icon();
                    nav_row(
                        SharedString::from(format!("settings-nav-{name}")),
                        name,
                        icon,
                        tab == current,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_settings_tab(tab, cx)))
                }))
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(NAV_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(fg.fill(Fill::Line))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(40.0))
                    .px_4()
                    .border_b_1()
                    .border_color(fg.fill(Fill::Line))
                    .child(
                        Icon::new(IconName::Settings)
                            .size(IconSize::Sm)
                            .color(fg.ink(Ink::Muted)),
                    )
                    .child(
                        div()
                            .text_size(Text::Md.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child("Settings"),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    crate::ui::scrollbar::Scrolled::new(
                        "settings-nav-scrollbar",
                        div()
                            .id("settings-nav")
                            .flex()
                            .flex_col()
                            .gap_4()
                            .size_full()
                            .overflow_y_scroll()
                            .pl_2()
                            .py_3()
                            .children(groups),
                    )
                    .gutter(px(8.0)),
                ),
            )
    }

    fn render_settings_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let (title, icon) = self.settings_tab.label_icon();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(40.0))
            .pl_6()
            .pr(px(12.0 + 32.0))
            .border_b_1()
            .border_color(fg.fill(Fill::Line))
            .child(Icon::new(icon).size(IconSize::Sm).color(fg.ink(Ink::Muted)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Md.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(title),
            )
            .when(self.settings_tab == SettingsTab::Appearance, |el| {
                el.child(self.render_restore_appearance(cx))
            })
    }

    fn render_settings_page(&self, cx: &Context<Self>) -> AnyElement {
        match self.settings_tab {
            SettingsTab::General => self.render_settings_general(cx).into_any_element(),
            SettingsTab::Connections => self.render_settings_connections(cx).into_any_element(),
            SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
            SettingsTab::Mcp => self.render_settings_mcp(cx).into_any_element(),
            SettingsTab::Skills => self.render_settings_skills(cx).into_any_element(),
            SettingsTab::Appearance => self.render_settings_appearance(cx).into_any_element(),
            SettingsTab::Shortcuts => self.render_settings_shortcuts(cx).into_any_element(),
            SettingsTab::About => self.render_settings_about(cx).into_any_element(),
            SettingsTab::Archive => self.render_settings_archive(cx).into_any_element(),
            SettingsTab::Worktrees => self.render_settings_worktrees(cx).into_any_element(),
            SettingsTab::Integrations => self.render_settings_integrations(cx).into_any_element(),
            SettingsTab::Bots => self.render_settings_bots(cx).into_any_element(),
        }
    }

    fn render_settings_general(&self, cx: &Context<Self>) -> SettingsPage {
        let model = &self.selected_model;
        let harness = model.split_once(':').map_or("", |(harness, _)| harness);
        let default_model = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(Text::Base.px())
            .text_color(cx.theme().colors.fg)
            .child(HarnessIcon::new(harness).size(px(14.0)))
            .child(crate::harness::catalog::label_for(model));
        let threads = SettingsGroup::new("New threads").row(
            SettingsRow::new("Default model")
                .description("The model a new thread starts with.")
                .control(default_model),
        );

        let alerts = self.render_settings_alerts(cx);
        let workspace = SettingsGroup::new("Workspace")
            .description("What BenCode shows around your chats.")
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Agents section")
                    .description(
                        "At the top of the navigator, the chats in flight across projects, those \
                         waiting on you first, with Allow and Deny for a command one asks to run. \
                         Finished turns stay until you open that session.",
                    )
                    .control(
                        Switch::new("working-agents", !self.live_agents_off).on_change(
                            move |on, _, cx| {
                                if let Err(err) = entity.update(cx, |this, cx| {
                                    this.set_live_agents(on, cx);
                                    this.play_cue(crate::sounds::Cue::Switch);
                                }) {
                                    log::debug!("working agents toggle after app drop: {err:#}");
                                }
                            },
                        ),
                    )
            })
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Composer runner")
                    .description(
                        "When a turn is running, the project's figure runs along the composer, \
                         bonks the scroll-to-latest button the first time, then jumps it, and \
                         sometimes grabs a coin.",
                    )
                    .control(
                        Switch::new("composer-mascot", !self.composer_mascot_off).on_change(
                            move |on, _, cx| {
                                if let Err(err) = entity.update(cx, |this, cx| {
                                    this.set_composer_mascot(on, cx);
                                    this.play_cue(crate::sounds::Cue::Switch);
                                }) {
                                    log::debug!("runner toggle after app drop: {err:#}");
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
                                if let Err(err) = entity.update(cx, |this, cx| {
                                    this.set_resume_interrupted_auto(on, cx);
                                    this.play_cue(crate::sounds::Cue::Switch);
                                }) {
                                    log::debug!("resume toggle after app drop: {err:#}");
                                }
                            },
                        ),
                    )
            });

        let chat = self.render_settings_chat(cx);

        let editors = SettingsGroup::new("External editors")
            .description("IDEs found on this Mac. Files and workspaces open in the default one.");
        let preferred = self.preferred_editor().map(|editor| editor.id);
        let editors = match &self.integrations.editors {
            None => editors.note("Looking for editors…"),
            Some(found) if found.is_empty() => editors.note(
                "No supported editor found. Install VS Code, Cursor or Zed to open files outside BenCode.",
            ),
            Some(found) => editors.rows(found.iter().enumerate().map(|(ix, editor)| {
                let id = editor.id;
                SettingsRow::new(editor.name).control(if preferred == Some(id) {
                    Badge::new("Default").tone(Tone::Accent).into_any_element()
                } else {
                    Button::new(("editor-make-default", ix), "Make default")
                        .variant(ButtonVariant::Outline)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_default_editor(id, cx);
                            this.play_cue(crate::sounds::Cue::Switch);
                        }))
                        .into_any_element()
                })
            })),
        };

        SettingsTab::General
            .page()
            .group(alerts)
            .group(threads)
            .group(workspace)
            .group(chat)
            .group(editors)
    }

    /// MonoCode Chat › "Collapse tool groups" and "Fold finished work".
    fn render_settings_chat(&self, cx: &Context<Self>) -> SettingsGroup {
        let entity = cx.entity().downgrade();
        let collapse = Switch::new("collapse-tool-groups", self.collapse_tool_groups).on_change(
            move |on, _, cx| {
                if let Err(err) = entity.update(cx, |this, cx| {
                    this.set_collapse_tool_groups(on, cx);
                    this.play_cue(crate::sounds::Cue::Switch);
                }) {
                    log::debug!("tool groups toggle after app drop: {err:#}");
                }
            },
        );
        let entity = cx.entity().downgrade();
        let fold = Switch::new("fold-finished-work", !self.finished_work_unfolded).on_change(
            move |on, _, cx| {
                if let Err(err) = entity.update(cx, |this, cx| {
                    this.set_fold_finished_work(on, cx);
                    this.play_cue(crate::sounds::Cue::Switch);
                }) {
                    log::debug!("fold work toggle after app drop: {err:#}");
                }
            },
        );
        SettingsGroup::new("Chat")
            .description("How a thread shows the agent's work.")
            .row(
                SettingsRow::new("Collapse tool groups")
                    .description(
                        "Keep tool groups closed, even while they run. Click a group to see its \
                         steps.",
                    )
                    .control(collapse),
            )
            .row(
                SettingsRow::new("Fold finished work")
                    .description(
                        "Fold a finished reply's tool calls under \"worked for\". Turn off to \
                         keep text and tool groups inline.",
                    )
                    .control(fold),
            )
    }

    /// MonoCode General › Alerts: Sounds and Notifications.
    fn render_settings_alerts(&self, cx: &Context<Self>) -> SettingsGroup {
        use crate::notifications::Permission;
        let muted = cx.theme().colors.fg.ink(Ink::Muted);
        let notifications = self.alerts.notifications;
        // MonoCode shows the reason beside the switch while it is on.
        let status = match self.alerts.permission {
            Permission::Denied if notifications => Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child("Permission needed")
                    .child(
                        Button::new("notifications-open-settings", "Open System Settings")
                            .variant(ButtonVariant::Outline)
                            .size(ControlSize::Sm)
                            .on_click(
                                cx.listener(|this, _, _, _| this.open_notification_settings()),
                            ),
                    )
                    .into_any_element(),
            ),
            // BenCode's own: the center needs the app bundle, so a build run
            // from `cargo run` has none.
            Permission::Unsupported if notifications => {
                Some("Only in BenCode.app".into_any_element())
            }
            _ => None,
        };
        SettingsGroup::new("Alerts")
            .description("How BenCode reaches you while you are looking somewhere else.")
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Sounds")
                    .description(
                        "Short cues for project activity, finished turns, and available updates. \
                         Mute a project from its menu on the rail. Switches and Copy on a finished \
                         turn also play.",
                    )
                    .control(Switch::new("sounds", self.alerts.sounds).on_change(
                        move |on, _, cx| {
                            let toggled = entity.update(cx, |this, cx| {
                                this.set_sounds(on, cx);
                                this.play_cue(crate::sounds::Cue::Switch);
                            });
                            if let Err(err) = toggled {
                                log::debug!("sounds toggle after app drop: {err:#}");
                            }
                        },
                    ))
            })
            .row({
                let entity = cx.entity().downgrade();
                SettingsRow::new("Notifications")
                    .description(
                        "Notify when a reminder is due, or when an agent finishes or needs input \
                         in another session or while BenCode is in the background. Click the \
                         notification to open that session.",
                    )
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_size(Text::Base.px())
                            .text_color(muted)
                            .children(status)
                            .child(Switch::new("notifications", notifications).on_change(
                                move |on, _, cx| {
                                    let toggled = entity.update(cx, |this, cx| {
                                        this.set_notifications(on, cx);
                                        this.play_cue(crate::sounds::Cue::Switch);
                                    });
                                    if let Err(err) = toggled {
                                        log::debug!("notifications toggle after app drop: {err:#}");
                                    }
                                },
                            )),
                    )
            })
    }

    fn render_settings_providers(&self, cx: &Context<Self>) -> SettingsPage {
        let clis = SettingsGroup::new("Agent CLIs")
            .description("Found on PATH. Sign in through each CLI.")
            .action(
                IconButton::new("harnesses-rescan", IconName::RefreshCw)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .disabled(self.harness_rescan)
                    .tooltip("Look for agent CLIs again")
                    .on_click(cx.listener(|this, _, _, cx| this.rescan_harnesses(cx))),
            );
        let clis = if self.harnesses.is_empty() {
            clis.note("No agent CLIs found. Install Claude Code, Codex, OpenCode or Antigravity, then refresh.")
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
                            if let Err(err) = entity.update(cx, |this, cx| {
                                this.set_claude_hooks(on, cx);
                                this.play_cue(crate::sounds::Cue::Switch);
                            }) {
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
            .group(self.render_settings_cli_import(cx))
            .group(advanced)
    }

    /// One row per CLI whose sessions BenCode can read, each with its
    /// Import button and how the last import went.
    fn render_settings_cli_import(&self, cx: &Context<Self>) -> SettingsGroup {
        use crate::app::cli_import::ImportState;
        use crate::harness::HarnessKind;
        let rows = [
            (HarnessKind::Claude, "Claude Code", "~/.claude/projects"),
            (HarnessKind::Codex, "Codex", "~/.codex/sessions"),
        ]
        .into_iter()
        .map(|(kind, name, folder)| {
            let state = self.cli_imports.get(&kind);
            let description = match state {
                None => {
                    format!("Terminal and editor sessions in {folder} and your accounts' folders.")
                }
                Some(ImportState::Running) => "Reading sessions…".to_string(),
                Some(ImportState::Done(report)) => {
                    let mut parts = vec![format!(
                        "Imported {}",
                        plural(report.added, "session", "sessions")
                    )];
                    if report.known > 0 {
                        parts.push(format!("{} already in BenCode", report.known));
                    }
                    if report.headless > 0 {
                        parts.push(format!(
                            "{} skipped (started by BenCode or a script)",
                            report.headless
                        ));
                    }
                    if report.homeless > 0 {
                        parts.push(format!(
                            "{} skipped (folder gone or home folder)",
                            report.homeless
                        ));
                    }
                    format!("{}.", parts.join(", "))
                }
                Some(ImportState::Failed(err)) => format!("Import failed: {err}"),
            };
            SettingsRow::new(name)
                .leading(icon_tile(
                    HarnessIcon::new(kind.id()).size(px(16.0)),
                    cx.theme().colors.fg,
                ))
                .description(description)
                .control(
                    Button::new(
                        SharedString::from(format!("cli-import-{}", kind.id())),
                        "Import",
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .icon(IconName::Download)
                    .loading(state == Some(&ImportState::Running))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.import_cli_sessions(kind, cx);
                    })),
                )
        });
        SettingsGroup::new("Import sessions")
            .description(
                "Bring in the sessions you ran in a terminal, as threads that carry on in the \
                 same conversation. Sessions BenCode already has are left as they are.",
            )
            .rows(rows)
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
            "Archive a project from the navigator to keep its chats without listing it there.",
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
                            .color(fg.ink(Ink::Secondary)),
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

fn nav_row(
    id: impl Into<SharedString>,
    label: &'static str,
    icon: IconName,
    active: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    let id: SharedString = id.into();
    let ink = if active { fg } else { fg.ink(Ink::Body) };
    div()
        .id(id.clone())
        .group(id.clone())
        .flex()
        .w_full()
        .items_center()
        .gap(px(10.0))
        .rounded(px(8.0))
        .px(px(10.0))
        .py_1p5()
        .text_size(Text::Md.px())
        .text_color(ink)
        .cursor_pointer()
        .map(|el| {
            if active {
                el.bg(fg.fill(Fill::Selected))
            } else {
                el.hover(move |s| s.bg(fg.fill(Fill::Hover)).text_color(fg))
            }
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(ink.opacity(if active { 0.9 } else { 0.7 }))
                .group_hover_color(id, fg),
        )
        .child(div().min_w_0().flex_1().truncate().child(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_belongs_to_a_group() {
        let grouped: Vec<SettingsTab> = SETTINGS_GROUPS
            .iter()
            .flat_map(|(_, tabs)| tabs.iter().copied())
            .collect();
        for (tab, ..) in SECTIONS {
            assert!(grouped.contains(&tab), "{tab:?} missing from SETTINGS_GROUPS");
        }
        assert_eq!(grouped.len(), SECTIONS.len());
    }
}
