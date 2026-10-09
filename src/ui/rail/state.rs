//! The rail's transient UI state: the open menus and dialogs, the resize
//! drag and the reorder drag.

use std::collections::HashMap;
use std::time::Instant;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Pixels, Point, Subscription, Window};

use crate::app::BenCodeApp;

/// What a project or group menu (MonoCode `TabGroupMenu`) acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    Project(String),
    Group(String),
}

#[derive(Clone, Debug)]
pub struct RailMenu {
    pub target: MenuTarget,
    pub position: Point<Pixels>,
}

/// The rows a menu row's submenu lists (MonoCode `ExplorerMenu` beside it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmenuKind {
    MoveToGroup,
    Editor,
    Mute,
    /// The Inbox menu's "Mute all projects".
    InboxMute,
}

#[derive(Clone, Debug)]
pub struct RailSubmenu {
    pub kind: SubmenuKind,
    pub position: Point<Pixels>,
    pub active: usize,
}

/// MonoCode `NotificationMuteDatePicker`: who it mutes and the date and
/// time chosen so far.
#[derive(Clone, Debug)]
pub struct MutePicker {
    pub ids: Vec<String>,
    pub title: String,
    pub position: Point<Pixels>,
    pub value: jiff::civil::DateTime,
    pub error: Option<String>,
}

/// MonoCode `RemoveProjectDialog`.
#[derive(Clone, Debug)]
pub struct RemoveProject {
    pub path: String,
    pub name: String,
    /// Saved conversations it will delete, once counted.
    pub sessions: Option<usize>,
}

/// A project row held by the pointer: its list and the slot it is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailReorder {
    pub list: String,
    pub path: String,
    pub to: usize,
}

pub struct RailUi {
    /// The menu's name field (MonoCode's `aria-label="Group name"` input).
    pub name_input: Entity<TextInput>,
    pub menu: Option<RailMenu>,
    pub submenu: Option<RailSubmenu>,
    pub custom_color_open: bool,
    /// MonoCode `menuError`: an editor that failed to open, a save that failed.
    pub menu_error: Option<String>,
    /// MonoCode `InboxNotificationMenu`, at the pointer.
    pub inbox_menu: Option<Point<Pixels>>,
    pub inbox_active: usize,
    pub mute_picker: Option<MutePicker>,
    pub removing: Option<RemoveProject>,
    /// The width shown while the sash is dragged; saved on release.
    pub drag_width: Option<f32>,
    /// Pointer x and width when the sash was pressed.
    pub drag_origin: (f32, f32),
    pub reorder: Option<RailReorder>,
    /// Rows sliding to a new slot: (offset px, sequence, start).
    pub slides: HashMap<String, (f32, u64, Instant)>,
    pub slide_seq: u64,
    /// A press on a trigger, so the open popover's outside-press check
    /// leaves the toggle to the trigger.
    pub trigger_hit: bool,
    _subscriptions: Vec<Subscription>,
}

impl RailUi {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let name_input = cx.new(|cx| TextInput::new(window, cx));
        let keys_input = name_input.clone();
        let weak_app = cx.entity().downgrade();
        let subscriptions = vec![
            // MonoCode: Enter commits the name and closes; leaving the field
            // commits it.
            cx.subscribe(
                &name_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => {
                        this.commit_rail_menu_name(cx);
                        this.close_rail_menu(cx);
                    }
                    InputEvent::Blur => this.commit_rail_menu_name(cx),
                    _ => {}
                },
            ),
            // Esc in the name field closes the menu (MonoCode `Popover`
            // `onDismiss("escape")`), a submenu first.
            cx.intercept_keystrokes(move |event, window, cx| {
                if event.keystroke.key != "escape"
                    || !keys_input.read(cx).focus_handle(cx).is_focused(window)
                {
                    return;
                }
                let handled = weak_app.update(cx, |this, cx| this.rail_menu_key("escape", cx));
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
            }),
        ];
        Self {
            name_input,
            menu: None,
            submenu: None,
            custom_color_open: false,
            menu_error: None,
            inbox_menu: None,
            inbox_active: 0,
            mute_picker: None,
            removing: None,
            drag_width: None,
            drag_origin: (0.0, 0.0),
            reorder: None,
            slides: HashMap::new(),
            slide_seq: 0,
            trigger_hit: false,
            _subscriptions: subscriptions,
        }
    }
}
