//! Every command by name: what Settings › Shortcuts lists and the command
//! palette (⇧⌘P, `ui/quick_open.rs`) runs. A command is an action of
//! `app/commands.rs` with the words and icon it goes by; its keys are looked
//! up in the keymap, so neither view can drift from what is bound.

use bencode_ui::primitives::IconName;
use gpui::Action;

use crate::app::commands as cmd;

/// A group of the page: its title, the line under it, its shortcuts.
pub(crate) struct Group<T> {
    pub title: &'static str,
    pub description: Option<&'static str>,
    pub rows: Vec<T>,
}

/// A row as written in [`groups`]: the action it runs and where.
pub(crate) struct Entry {
    pub label: &'static str,
    pub icon: IconName,
    pub action: Box<dyn Action>,
    /// The key context the binding lives in (`None`: everywhere).
    pub context: Option<&'static str>,
    pub note: Option<&'static str>,
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
pub(crate) fn groups() -> Vec<Group<Entry>> {
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
                entry("Command Palette", I::Terminal, cmd::CommandPalette)
                    .note("Every command by name. Or type > in Go to File."),
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
                entry("Move Project Up", I::ChevronUp, cmd::MoveProjectUp)
                    .note("Within its list on the navigator."),
                entry("Move Project Down", I::ChevronDown, cmd::MoveProjectDown),
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
                entry("Focus Prompt", I::Keyboard, cmd::FocusPrompt)
                    .note("Back to the prompt of the thread in view."),
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
                entry("Toggle Bot", I::Bot, cmd::ToggleBot),
                entry("Ask Bot About This Thread", I::Bot, cmd::AskBot)
                    .note("With what is selected in it."),
                entry("New Terminal", I::Terminal, cmd::NewTerminal),
                entry("Move Terminal to Tab", I::Terminal, cmd::MoveTerminalToTab)
                    .note("The dock's terminal, beside the chat."),
                entry("Open Browser", I::Globe, cmd::OpenBrowser),
                entry(
                    "Reload Page Ignoring Cache",
                    I::RotateCw,
                    cmd::ReloadIgnoringCache,
                ),
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
            title: "Inbox",
            description: Some("While the Inbox's list has focus, on the selected item."),
            rows: vec![
                entry("Next Item", I::ArrowDown, cmd::InboxNext)
                    .within("InboxList")
                    .note("J works too."),
                entry("Previous Item", I::ArrowUp, cmd::InboxPrevious)
                    .within("InboxList")
                    .note("K works too."),
                entry(
                    "Open Thread or Send to Agent",
                    I::Sparkles,
                    cmd::InboxSendToAgent,
                )
                .within("InboxList")
                .note(
                    "Opens the item's newest thread; else sends an issue to an agent, or a pull request for review. With items selected, sends them all.",
                ),
                entry("Open in Tracker", I::ExternalLink, cmd::InboxOpenInTracker)
                    .within("InboxList"),
                entry("Open Beside the Chat", I::Columns2, cmd::InboxOpenBeside)
                    .within("InboxList")
                    .note("Leaves the Inbox; the item stays in a tab beside the thread."),
                entry("Filter", I::Search, cmd::InboxFocusFilter)
                    .within("InboxList")
                    .note("Esc in the filter field goes back to the list."),
                entry("Refresh", I::RefreshCw, cmd::InboxRefresh).within("InboxList"),
                entry("Mark Read or Unread", I::CheckCheck, cmd::InboxToggleRead)
                    .within("InboxList"),
                entry("Assign to Me", I::User, cmd::InboxAssignToMe).within("InboxList"),
                entry("Comment", I::MessageSquare, cmd::InboxFocusComment).within("InboxList"),
                entry("Translate", I::Languages, cmd::InboxToggleTranslation)
                    .within("InboxList")
                    .note("Into the language last picked; again for the original."),
                entry("Copy Key and Title", I::Copy, cmd::InboxCopyTicket).within("InboxList"),
                entry("Only Mine", I::User, cmd::InboxToggleMine)
                    .within("InboxList")
                    .note("Items assigned to you; again for all."),
                entry("Only Unread", I::Eye, cmd::InboxToggleUnread)
                    .within("InboxList")
                    .note("Unread items and those that need you; again for all."),
                entry("Select or Deselect", I::SquareCheck, cmd::InboxTogglePick)
                    .within("InboxList")
                    .note("The open item, to send with others; ⌘-click and ⇧-click in the list."),
                entry("Clear Selection", I::X, cmd::InboxClearPicks)
                    .within("InboxList")
                    .note("Then Esc closes the Inbox."),
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
                entry("Save", I::Save, cmd::Save)
                    .note("The open file, or the automation in the editor."),
                entry("Quit BenCode", I::Power, cmd::Quit),
            ],
        },
    ]
}

/// Commands no key is bound to: the palette's alone (Settings › Shortcuts
/// lists keys).
fn unbound() -> Vec<Entry> {
    use IconName as I;
    vec![
        entry("Inbox", I::Inbox, cmd::OpenInbox),
        entry("Notes", I::FileText, cmd::OpenNotes),
        entry("Automations", I::Zap, cmd::OpenAutomations),
        entry("Stage Session Changes", I::Plus, cmd::StageSessionChanges),
        entry(
            "Commit Session Changes",
            I::GitCommitHorizontal,
            cmd::CommitSessionChanges,
        ),
        entry(
            "Commit and Push Session Changes",
            I::CloudUpload,
            cmd::CommitAndPushSessionChanges,
        ),
        entry("Push Session Commits", I::Upload, cmd::PushSessionCommits),
        // The selected open pull request's review form ("Inbox: …").
        entry("Review Pull Request", I::Eye, cmd::InboxReview).within("InboxList"),
        entry("Check for Updates", I::Download, cmd::CheckForUpdates),
        entry("Show Logs", I::FileText, cmd::ShowLogs),
        entry("Reload BenCode", I::RefreshCw, cmd::ReloadApp),
    ]
}

/// A command as the palette lists it.
pub(crate) struct PaletteCommand {
    /// Its name, a scoped one led by where it works ("Inbox: Translate").
    pub label: String,
    pub icon: IconName,
    /// Its chord in keymap syntax, when one is bound.
    pub keys: Option<String>,
    pub action: Box<dyn Action>,
    /// The view it needs open (`None`: any).
    pub scope: Option<Scope>,
}

/// A view a palette command needs open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Inbox,
}

/// The palette's commands: every one bound everywhere, the Inbox's keys
/// (they act on the open item, not on a focused selection), and the unbound
/// ones. Keys that act on what a list or a text has selected stay out: from
/// the palette they would have nothing to act on.
pub(crate) fn palette() -> Vec<PaletteCommand> {
    let keymap = cmd::keymap();
    let mut out: Vec<PaletteCommand> = Vec::new();
    let listed = groups()
        .into_iter()
        // Recommended repeats rows of the other groups.
        .filter(|group| group.title != "Recommended")
        .flat_map(|group| group.rows)
        .chain(unbound());
    for entry in listed {
        let (label, scope) = match entry.context {
            None => (entry.label.to_string(), None),
            Some("InboxList") => (format!("Inbox: {}", entry.label), Some(Scope::Inbox)),
            Some(_) => continue,
        };
        // The palette is open already; Esc would close it, not stop a turn.
        if entry.action.partial_eq(&cmd::CommandPalette) || entry.action.partial_eq(&cmd::CloseView)
        {
            continue;
        }
        if out.iter().any(|known| known.label == label) {
            continue;
        }
        out.push(PaletteCommand {
            label,
            icon: entry.icon,
            keys: cmd::chord_for(&keymap, entry.action.as_ref(), entry.context),
            action: entry.action,
            scope,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_lists_each_command_once_with_its_keys() {
        let commands = palette();
        let mut labels: Vec<&str> = commands.iter().map(|c| c.label.as_str()).collect();
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count);
        let keys = |label: &str| {
            let command = commands.iter().find(|c| c.label == label);
            command.and_then(|c| c.keys.clone())
        };
        assert_eq!(keys("New Thread").as_deref(), Some("cmd-t"));
        assert_eq!(keys("Inbox: Translate").as_deref(), Some("t"));
        assert_eq!(keys("Reload BenCode"), None);
        // Bound to a selection that the palette's focus would not have.
        assert!(commands.iter().all(|c| c.label != "Copy Path"));
    }

    #[test]
    fn unbound_commands_have_no_key() {
        // One that gains a key belongs in `groups`, where Settings shows it.
        let keymap = cmd::keymap();
        for entry in unbound() {
            assert!(
                cmd::chord_for(&keymap, entry.action.as_ref(), None).is_none(),
                "{} is bound",
                entry.label
            );
        }
    }

    #[test]
    fn reviewing_a_pull_request_is_offered_in_the_inbox() {
        let commands = palette();
        let review = commands
            .iter()
            .find(|c| c.label == "Inbox: Review Pull Request")
            .expect("the palette lists it");
        assert_eq!(review.scope, Some(Scope::Inbox));
        assert_eq!(review.keys, None);
    }
}
