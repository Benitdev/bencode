//! Shared behaviour of the composer's popovers (MonoCode `Popover`): the
//! open menu takes keyboard focus, a mouse-down outside every popover and
//! its chip dismisses it, and closing with Esc or a pick hands focus back
//! to the prompt.

use gpui::{App, Context, FocusHandle, Focusable, InteractiveElement, Window};

use super::PERMISSION_MODES;
use super::model_picker::{ModelTab, Submenu};
use crate::app::BenCodeApp;

pub struct MenuState {
    /// Focused while a keyboard-driven menu (access, model) is open.
    pub focus: FocusHandle,
    /// Highlighted row of the access picker.
    pub access_index: usize,
    /// Model menu: highlighted row, open flyout, highlighted choice in a
    /// setting flyout, and the provider tab of the models flyout.
    pub model_entry: usize,
    pub model_submenu: Option<Submenu>,
    pub setting_index: usize,
    pub model_tab: ModelTab,
    /// The recent-models menu (⌘.) and its highlighted row.
    pub recent_open: bool,
    pub recent_index: usize,
    /// A mouse-down this dispatch landed inside a popover or its chip.
    click_inside: bool,
    /// The outside-click check for this dispatch is already queued.
    check_queued: bool,
}

impl MenuState {
    pub fn new(focus: FocusHandle) -> Self {
        Self {
            focus,
            access_index: 0,
            model_entry: 0,
            model_submenu: None,
            setting_index: 0,
            model_tab: ModelTab::default(),
            recent_open: false,
            recent_index: 0,
            click_inside: false,
            check_queued: false,
        }
    }
}

/// Moves window focus to `handle` once the current update ends.
pub fn focus_later(handle: FocusHandle, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window()
            && let Err(err) = window.update(cx, |_, window, cx| window.focus(&handle, cx))
        {
            log::debug!("composer: could not move focus: {err:#}");
        }
    });
}

/// Makes `el` part of the open popover: mouse-downs on it never dismiss,
/// mouse-downs outside it do unless they land on another part.
pub(super) fn popover_surface<E: InteractiveElement>(el: E, cx: &Context<BenCodeApp>) -> E {
    popover_anchor(el, cx).on_mouse_down_out(cx.listener(|this, _, window, cx| {
        this.queue_outside_check(window, cx);
    }))
}

/// A chip that toggles a popover: clicking it never counts as outside, so
/// its own click can close the popover instead of reopening it.
pub(super) fn popover_anchor<E: InteractiveElement>(el: E, cx: &Context<BenCodeApp>) -> E {
    el.capture_any_mouse_down(cx.listener(|this, _, window, cx| {
        this.composer_menus.click_inside = true;
        this.queue_outside_check(window, cx);
    }))
}

impl BenCodeApp {
    /// Every part of the popover has seen the mouse-down once the dispatch
    /// ends; only then is it known whether the press was outside them all.
    fn queue_outside_check(&mut self, window: &Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.composer_menus.check_queued, true) {
            return;
        }
        cx.defer_in(window, |this, _, cx| {
            this.composer_menus.check_queued = false;
            if !std::mem::take(&mut this.composer_menus.click_inside) {
                this.close_composer_popovers(cx);
            }
        });
    }

    pub(super) fn focus_composer_menu(&self, cx: &mut App) {
        focus_later(self.composer_menus.focus.clone(), cx);
    }

    pub fn refocus_prompt(&self, cx: &mut App) {
        focus_later(self.prompt_input.read(cx).focus_handle(cx), cx);
    }

    /// Keys while a composer menu holds focus (MonoCode `onMenuKey`).
    pub fn handle_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.is_permission_picker_open {
            self.access_menu_key(key, cx)
        } else if self.is_model_picker_open {
            self.model_menu_key(key, cx)
        } else if self.composer_menus.recent_open {
            self.recent_menu_key(key, cx)
        } else {
            false
        }
    }

    /// MonoCode `AccessPicker`: ↑/↓ stop at the ends, Enter picks, Esc
    /// closes; both hand focus back to the prompt.
    fn access_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let last = PERMISSION_MODES.len() - 1;
        let index = &mut self.composer_menus.access_index;
        match key {
            "down" => *index = (*index + 1).min(last),
            "up" => *index = index.saturating_sub(1),
            "enter" => {
                let mode = PERMISSION_MODES[(*index).min(last)].0;
                self.is_permission_picker_open = false;
                self.set_permission_mode(mode, cx);
                self.refocus_prompt(cx);
            }
            "escape" => {
                self.is_permission_picker_open = false;
                self.refocus_prompt(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }
}
