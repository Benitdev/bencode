//! Settings › Shortcuts: BenCode's keys in groups, read only. MonoCode's
//! Keybindings page (`SettingsView.tsx` `KeybindingsPage`) also rebinds
//! them; BenCode's keymap is fixed (`app/commands.rs`), and the keys shown
//! here are looked up in it, so the page cannot drift from what is bound.

use std::sync::OnceLock;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use bencode_ui::typography::KbdCombo;
use gpui::{Action, Context};

use crate::app::BenCodeApp;
use crate::app::commands::{self as cmd, chord_for};
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow};
use crate::ui::tokens::{Ink, Tone as _};

/// A group of the page: its title, the line under it, its shortcuts.
struct Group<T> {
    title: &'static str,
    description: Option<&'static str>,
    rows: Vec<T>,
}

/// A row as written in [`groups`]: the action it runs and where.
struct Entry {
    label: &'static str,
    icon: IconName,
    action: Box<dyn Action>,
    /// The key context the binding lives in (`None`: everywhere).
    context: Option<&'static str>,
    note: Option<&'static str>,
}

/// A row with its keys found in the keymap.
struct Shown {
    label: &'static str,
    icon: IconName,
    keys: String,
    note: Option<&'static str>,
}

fn entry(label: &'static str, icon: IconName, action: impl Action) -> Entry {
    Entry {
        label,
        icon,
        action: Box::new(action),
        context: None,
        note: None,
    }
}

impl Entry {
    fn within(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
    }

    fn note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }
}

/// What the page lists, in the menu bar's words. Recommended repeats the
/// keys most worth learning first.
fn groups() -> Vec<Group<Entry>> {
    use IconName as I;
    vec![
        Group {
            title: "Recommended",
            description: None,
            rows: vec![
                entry("Search", I::Search, cmd::Search),
                entry("Go to File", I::File, cmd::GoToFile),
                entry("New Thread", I::SquarePen, cmd::NewThread),
                entry("Switch Model", I::Cpu, cmd::SwitchModel),
                entry("Toggle Terminal", I::PanelBottom, cmd::ToggleTerminal),
            ],
        },
        Group {
            title: "Navigation",
            description: None,
            rows: vec![
                entry("Search", I::Search, cmd::Search),
                entry("Go to File", I::File, cmd::GoToFile),
                entry("Find in Files", I::FileCode, cmd::FindInProject),
                entry("File History", I::History, cmd::ShowFileHistory)
                    .note("The commits that changed the open file."),
                entry("Go to Definition", I::SquareFunction, cmd::GoToDefinition)
                    .note("Or ⌘-click a name. Found by searching the project."),
                entry("Back", I::ArrowLeft, cmd::GoBack),
                entry("Forward", I::ArrowRight, cmd::GoForward),
                entry("Previous Session", I::ArrowUp, cmd::PreviousSession)
                    .note("Opens it in a tab of its own."),
                entry("Next Session", I::ArrowDown, cmd::NextSession)
                    .note("Opens it in a tab of its own."),
                entry(
                    "Previous Session in Pane",
                    I::ChevronUp,
                    cmd::PreviousSessionInTab,
                )
                .note("Opens it in place of the focused pane."),
                entry(
                    "Next Session in Pane",
                    I::ChevronDown,
                    cmd::NextSessionInTab,
                )
                .note("Opens it in place of the focused pane."),
                entry("Previous Project", I::ChevronLeft, cmd::PreviousProject),
                entry("Next Project", I::ChevronRight, cmd::NextProject),
                entry("Next Agent", I::Bot, cmd::NextAgent).note("Those waiting on you first."),
                entry("Open Settings", I::Settings, cmd::OpenSettings),
            ],
        },
        Group {
            title: "Tabs and panes",
            description: None,
            rows: vec![
                entry("Next Tab", I::ArrowRight, cmd::NextTab).note("⌃Tab works too."),
                entry("Previous Tab", I::ArrowLeft, cmd::PreviousTab).note("⌃⇧Tab works too."),
                entry("Go to First Tab", I::ListOrdered, cmd::ActivateTab(Some(0)))
                    .note("⌘2 to ⌘8 show the tabs after it."),
                entry("Go to Last Tab", I::ListOrdered, cmd::ActivateTab(None)),
                entry("Close", I::X, cmd::CloseActive),
                entry("Close Other Tabs", I::X, cmd::CloseOtherTabs),
                entry("Close All Tabs", I::X, cmd::CloseAllTabs),
                entry("Split Pane Right", I::Columns2, cmd::SplitRight),
                entry("Split Pane Down", I::Rows2, cmd::SplitDown),
                entry("Focus Pane Left", I::ArrowLeft, cmd::FocusLeft),
                entry("Focus Pane Right", I::ArrowRight, cmd::FocusRight),
                entry("Focus Pane Up", I::ArrowUp, cmd::FocusUp),
                entry("Focus Pane Down", I::ArrowDown, cmd::FocusDown),
            ],
        },
        Group {
            title: "Conversation",
            description: None,
            rows: vec![
                entry("New Thread", I::SquarePen, cmd::NewThread),
                entry("Switch Model", I::Cpu, cmd::SwitchModel),
                entry("Stop the Turn", I::CircleStop, cmd::CloseView)
                    .note("Closes an open popover, search or overlay first."),
                entry("Find in Conversation", I::Search, cmd::FindInConversation),
                entry("Find Next", I::ChevronDown, cmd::FindNext),
                entry("Find Previous", I::ChevronUp, cmd::FindPrevious),
                entry("Toggle Worktree", I::GitBranch, cmd::ToggleWorkspaceMode)
                    .within("DraftComposer")
                    .note("In a new thread's composer: start it in a worktree or not."),
                entry("Copy Selection", I::Copy, cmd::TranscriptCopy).within("Transcript"),
                entry("Select All", I::Type, cmd::TranscriptSelectAll).within("Transcript"),
                entry("Copy Selected Lines", I::Copy, cmd::DiffCopy)
                    .within("DiffText")
                    .note("In a review of changes."),
                entry("Select All Lines", I::Type, cmd::DiffSelectAll)
                    .within("DiffText")
                    .note("In a review: one side of a side by side review."),
                entry("Copy Selected Text", I::Copy, cmd::InboxCopy)
                    .within("InboxText")
                    .note("In an Inbox item's description and comments."),
                entry("Select All Text", I::Type, cmd::InboxSelectAll)
                    .within("InboxText")
                    .note("In an Inbox item: its description and comments."),
                entry("Archive Session", I::Archive, cmd::ArchiveSession),
            ],
        },
        Group {
            title: "View",
            description: None,
            rows: vec![
                entry("Toggle Navigator", I::PanelLeft, cmd::ToggleSidebar),
                entry("Toggle Inspector", I::PanelRight, cmd::ToggleInspector),
                entry("Explorer", I::Folder, cmd::ShowExplorer),
                entry("Changes", I::GitBranch, cmd::ShowChanges),
                entry("Toggle Terminal", I::PanelBottom, cmd::ToggleTerminal),
                entry("Toggle Mono", I::Bot, cmd::ToggleMono),
                entry("New Terminal", I::Terminal, cmd::NewTerminal),
                entry("Open Browser", I::Globe, cmd::OpenBrowser),
                entry(
                    "Swap Chat and File Pane",
                    I::Columns2,
                    cmd::SwapFilePaneSide,
                ),
                entry("Zoom In", I::ZoomIn, cmd::ZoomIn),
                entry("Zoom Out", I::ZoomOut, cmd::ZoomOut),
                entry("Reset Zoom", I::RotateCcw, cmd::ZoomReset),
            ],
        },
        Group {
            title: "Session list",
            description: Some("While the navigator's threads have focus."),
            rows: vec![
                entry("Rename", I::Pencil, cmd::RenameSelectedSession).within("SessionList"),
                entry("Delete", I::Trash2, cmd::DeleteSelectedSessions).within("SessionList"),
                entry("Clear Selection", I::X, cmd::ClearSessionSelection).within("SessionList"),
            ],
        },
        Group {
            title: "Explorer",
            description: Some("While the file tree has focus, on the selected entry."),
            rows: vec![
                entry("Copy Path", I::Link, cmd::TreeCopyPath).within("FileTree"),
                entry("Copy", I::Copy, cmd::TreeCopy).within("FileTree"),
                entry("Cut", I::Scissors, cmd::TreeCut).within("FileTree"),
                entry("Paste", I::Clipboard, cmd::TreePaste).within("FileTree"),
                entry("Rename", I::Pencil, cmd::TreeRename).within("FileTree"),
                entry("Delete", I::Trash2, cmd::TreeDelete).within("FileTree"),
                entry("Cancel Cut", I::X, cmd::TreeClearCut).within("FileTree"),
            ],
        },
        Group {
            title: "App",
            description: None,
            rows: vec![
                entry("Open Project", I::FolderOpen, cmd::OpenProject),
                entry("Save", I::Save, cmd::Save),
                entry("Quit BenCode", I::Power, cmd::Quit),
            ],
        },
    ]
}

/// [`groups`] with their keys, looked up once: the keymap does not change
/// while BenCode runs.
fn shown() -> &'static [Group<Shown>] {
    static SHOWN: OnceLock<Vec<Group<Shown>>> = OnceLock::new();
    SHOWN.get_or_init(|| {
        let keymap = cmd::keymap();
        groups()
            .into_iter()
            .map(|group| Group {
                title: group.title,
                description: group.description,
                rows: group
                    .rows
                    .into_iter()
                    .filter_map(|entry| {
                        let Some(keys) = chord_for(&keymap, entry.action.as_ref(), entry.context)
                        else {
                            log::warn!("shortcuts page: nothing binds {:?}", entry.label);
                            return None;
                        };
                        Some(Shown {
                            label: entry.label,
                            icon: entry.icon,
                            keys,
                            note: entry.note,
                        })
                    })
                    .collect(),
            })
            .collect()
    })
}

impl BenCodeApp {
    pub(crate) fn render_settings_shortcuts(&self, cx: &Context<Self>) -> SettingsPage {
        let muted = cx.theme().colors.fg.ink(Ink::Secondary);
        shown()
            .iter()
            .fold(SettingsTab::Shortcuts.page(), |page, group| {
                let card = SettingsGroup::new(group.title).rows(group.rows.iter().map(|row| {
                    let item = SettingsRow::new(row.label)
                        .leading(Icon::new(row.icon).size(IconSize::Sm).color(muted))
                        .control(KbdCombo::new(&row.keys));
                    match row.note {
                        Some(note) => item.description(note),
                        None => item,
                    }
                }));
                page.group(match group.description {
                    Some(text) => card.description(text),
                    None => card,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_shortcut_is_bound() {
        let keymap = cmd::keymap();
        for group in groups() {
            for entry in group.rows {
                assert!(
                    chord_for(&keymap, entry.action.as_ref(), entry.context).is_some(),
                    "{} › {} has no binding",
                    group.title,
                    entry.label
                );
            }
        }
    }
}
