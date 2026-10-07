//! Moving the window from its own title bar (MonoCode
//! `data-tauri-drag-region`).
//!
//! The window is created with `app_owns_titlebar_drag`, so AppKit no longer
//! drags it from the transparent titlebar strip. That strip covers the
//! title-bar tabs, and AppKit's drag moved the window whenever a tab was
//! dragged. A drag region instead moves the window itself, as Zed's
//! `PlatformTitleBar` does: a press on its bare background followed by a
//! move calls `start_window_move`, and a double click is the titlebar's.
//!
//! A press on a tab or a button inside a region must not reach it: wrap
//! such elements in `claim_press`. Their own click and drag handlers still
//! run, since GPUI bubbles an element's own listeners before its
//! `on_mouse_down`.

use gpui::{
    Context, InteractiveElement, MouseButton, MouseDownEvent, MouseMoveEvent, WindowControlArea,
};

use crate::app::BenCodeApp;

/// Keeps a left press on `el` from reaching the drag region behind it.
pub fn claim_press<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl BenCodeApp {
    /// Makes `el` a window drag region: a press on its background and a move
    /// drag the window, a double click zooms or minimizes it (the system's
    /// titlebar double-click setting).
    pub fn window_drag_region<E: InteractiveElement>(&self, el: E, cx: &Context<Self>) -> E {
        el.window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    if event.click_count == 2 {
                        this.window_drag_pressed = false;
                        if cfg!(target_os = "macos") {
                            window.titlebar_double_click();
                        } else {
                            window.zoom_window();
                        }
                    } else {
                        this.window_drag_pressed = true;
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                if std::mem::take(&mut this.window_drag_pressed)
                    && event.pressed_button == Some(MouseButton::Left)
                {
                    window.start_window_move();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.window_drag_pressed = false),
            )
    }
}
