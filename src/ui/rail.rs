//! Leftmost activity bar: switcher for sidebar panels, modals, and workspace tools.

use ely_gpui_component::primitives::IconName;
use ely_gpui_component::shell::{ActivityBar, ActivityItem};
use gpui::{Context, IntoElement, SharedString};

use crate::app::{BenCodeApp, SidebarMode};

impl BenCodeApp {
    pub fn render_project_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let changes = (self.git_status.staged.len() + self.git_status.unstaged.len()) as u32;

        let selected = if self.is_settings_open {
            Some("settings")
        } else if self.is_search_open {
            Some("search")
        } else if self.is_notes_open {
            Some("notes")
        } else if self.is_automations_open {
            Some("automations")
        } else if self.is_inbox_open {
            Some("inbox")
        } else {
            match self.sidebar_mode {
                SidebarMode::Files => Some("files"),
                SidebarMode::Changes => Some("changes"),
                SidebarMode::Sessions => None,
            }
        };

        let mut bar = ActivityBar::new("project-rail")
            .item(ActivityItem::new("search", IconName::Search, "Search (⌘K)"))
            .item(ActivityItem::new("notes", IconName::FileText, "Notes & Scratchpad"))
            .item(ActivityItem::new("automations", IconName::Zap, "Automations"))
            .item(ActivityItem::new("inbox", IconName::Inbox, "Inbox & PR Reviews"))
            .item(ActivityItem::new("files", IconName::Folder, "Files Explorer"))
            .item(ActivityItem::new("changes", IconName::GitBranch, "Source Control").badge(changes))
            .footer(ActivityItem::new("settings", IconName::Settings, "Settings (⌘,)"));

        if let Some(sel) = selected {
            bar = bar.selected(sel);
        }

        bar.on_select(cx.listener(|this, id: &SharedString, _, cx| {
            match id.as_ref() {
                "search" => this.open_search_modal(cx),
                "notes" => this.open_notes(cx),
                "automations" => this.open_automations(cx),
                "inbox" => this.open_inbox_modal(cx),
                "files" => {
                    this.sidebar_mode = if this.sidebar_mode == SidebarMode::Files {
                        SidebarMode::Sessions
                    } else {
                        SidebarMode::Files
                    };
                    cx.notify();
                }
                "changes" => {
                    this.sidebar_mode = if this.sidebar_mode == SidebarMode::Changes {
                        SidebarMode::Sessions
                    } else {
                        SidebarMode::Changes
                    };
                    this.refresh_git_status(cx);
                }
                "settings" => {
                    this.is_settings_open = true;
                    cx.notify();
                }
                _ => {}
            }
        }))
    }
}
