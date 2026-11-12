//! A menu opened at a point by a right click, drawn by the view that opened
//! it: the editing menus of text inputs, the code editor and the terminal.
//! MonoCode `input/context_menu.rs`. GPUI draws these fields itself, so no
//! system editing menu comes with them.

use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    Action, AnyElement, App, AppContext, Context, ElementId, Entity, FocusHandle, IntoElement,
    ParentElement, Pixels, Point, Styled, Subscription, Window, div,
};

use super::{
    menu::{Open, Spot, hang},
    model::Menu,
};

/// What a right click opened, held by the view until it closes.
pub(crate) struct PointMenu {
    id: ElementId,
    menu: Menu,
    state: Entity<Open>,
    _redraw: Subscription,
}

impl PointMenu {
    pub(crate) fn open<V: 'static>(menu: Menu, at: Point<Pixels>, cx: &mut Context<V>) -> Self {
        // A new name each time: the focus it takes is kept under this name,
        // and the view may not draw between one menu and the next.
        static OPENED: AtomicU64 = AtomicU64::new(0);
        let id = ElementId::Name(
            format!("point-menu-{}", OPENED.fetch_add(1, Ordering::Relaxed)).into(),
        );
        let state = cx.new(|_| Open::default());
        let redraw = cx.observe(&state, |_, _, cx| cx.notify());
        Open::show(&state, &menu, Spot::At(at), false, cx);
        cx.notify();
        Self {
            id,
            menu,
            state,
            _redraw: redraw,
        }
    }

    pub(crate) fn is_open(&self, cx: &App) -> bool {
        self.state.read(cx).is_open()
    }

    pub(crate) fn close(&self, window: &mut Window, cx: &mut App) {
        Open::close(&self.state, window, cx);
    }

    pub(crate) fn element(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        hang(&self.id, &self.menu, &self.state, None, window, cx)
    }
}

/// `root` with its open menu beside it, never inside: inside, the field's key
/// context would take the arrows and Enter from the menu, and a press on a
/// row would reach the field too.
pub(crate) fn beside(root: impl IntoElement, menu: Option<AnyElement>) -> AnyElement {
    match menu {
        Some(menu) => div().size_full().child(root).child(menu).into_any_element(),
        None => root.into_any_element(),
    }
}

/// A row's work: `action` on the focused field, as its shortcut would be, so
/// what its ancestors do with it (a composer's pasted images) still happens.
pub(crate) fn run_on<A: Action>(
    focus: &FocusHandle,
    action: A,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let focus = focus.clone();
    move |window, cx| {
        window.focus(&focus, cx);
        window.dispatch_action(action.boxed_clone(), cx);
    }
}

/// An editing shortcut, in the key syntax a row takes.
pub(crate) fn editing_keys(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}
