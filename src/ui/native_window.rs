//! What GPUI's windows lack on macOS, asked of their `NSWindow`: the
//! floating chat's panel behaviour (MonoCode `make_panel`), hiding one
//! window rather than the app (MonoCode `window.hide()`), its shadow kept
//! to its rounded shape (MonoCode `invalidateShadow`), and the screen under
//! the pointer (MonoCode `monitor_from_point(cursor_position())`).
//!
//! What can move the keyboard to another window (taking one off screen,
//! bringing the app forward) waits until the update asking for it has
//! returned, as GPUI's `activate_window` does: AppKit tells the window
//! that becomes key at once, and GPUI draws it then, which it cannot do
//! while the app is borrowed (the frame is lost, "RefCell already
//! borrowed").

#[cfg(not(target_os = "macos"))]
pub fn make_floating_panel(_: &gpui::Window) {}

#[cfg(not(target_os = "macos"))]
pub fn order_out(_: &gpui::Window, _: &gpui::App) {}

/// Brings BenCode forward, over the other apps.
#[cfg(not(target_os = "macos"))]
pub fn activate_app(cx: &gpui::App) {
    cx.activate(true);
}

#[cfg(not(target_os = "macos"))]
pub fn invalidate_shadow(_: &gpui::Window) {}

#[cfg(not(target_os = "macos"))]
pub fn display_under_pointer() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub use macos::{
    activate_app, display_under_pointer, invalidate_shadow, make_floating_panel, order_out,
};

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod macos {
    use objc::runtime::{NO, Object, YES};
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type Id = *mut Object;

    /// `NSWindowStyleMaskResizable`.
    const RESIZABLE: usize = 1 << 3;
    /// `NSWindowCloseButton`, `NSWindowMiniaturizeButton`, `NSWindowZoomButton`.
    const BUTTONS: [usize; 3] = [0, 1, 2];

    /// The window's `NSWindow`, or nil.
    fn ns_window(window: &gpui::Window) -> Option<Id> {
        // `Window::window_handle` is GPUI's own handle; this is the
        // platform's.
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        let view = appkit.ns_view.as_ptr() as Id;
        // SAFETY: AppKit calls on the main thread, where GPUI runs its
        // windows. `view` is GPUI's live NSView; its window is borrowed.
        let native: Id = unsafe { msg_send![view, window] };
        (!native.is_null()).then_some(native)
    }

    /// GPUI gives a window without a title bar no resizable style, whatever
    /// `is_resizable` says, and its floating window is an `NSPanel`, which
    /// AppKit hides whenever BenCode is not the active app. MonoCode's panel
    /// resizes, has no traffic lights and stays over the other apps.
    pub fn make_floating_panel(window: &gpui::Window) {
        let Some(native) = ns_window(window) else {
            log::warn!("the floating chat has no native window to set up");
            return;
        };
        // SAFETY: as in `ns_window`; each button is borrowed and checked
        // for nil.
        unsafe {
            let mask: usize = msg_send![native, styleMask];
            let _: () = msg_send![native, setStyleMask: mask | RESIZABLE];
            // A resizable window gets its traffic lights back; MonoCode's
            // panel has none.
            for button in BUTTONS {
                let button: Id = msg_send![native, standardWindowButton: button];
                if !button.is_null() {
                    let _: () = msg_send![button, setHidden: YES];
                }
            }
            let _: () = msg_send![native, setHidesOnDeactivate: NO];
        }
    }

    /// A window drawn with clear corners casts its shadow from what it drew
    /// last: AppKit is asked again once it has a new size.
    pub fn invalidate_shadow(window: &gpui::Window) {
        let Some(native) = ns_window(window) else {
            return;
        };
        // SAFETY: as in `ns_window`.
        unsafe {
            let _: () = msg_send![native, invalidateShadow];
        }
    }

    /// AppKit's `NSPoint` and `NSRect`, in points with the origin at the
    /// primary screen's bottom left.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Point,
    }

    /// The display the pointer is on, as GPUI names it (its
    /// `CGDirectDisplayID`).
    pub fn display_under_pointer() -> Option<u64> {
        use objc::class;
        // SAFETY: AppKit class methods and the screens they return, read on
        // the main thread; nothing is kept past this call. The key is a
        // static C string, copied by `stringWithUTF8String:`.
        unsafe {
            let pointer: Point = msg_send![class!(NSEvent), mouseLocation];
            let screens: Id = msg_send![class!(NSScreen), screens];
            if screens.is_null() {
                return None;
            }
            let count: usize = msg_send![screens, count];
            let key: Id =
                msg_send![class!(NSString), stringWithUTF8String: c"NSScreenNumber".as_ptr()];
            for ix in 0..count {
                let screen: Id = msg_send![screens, objectAtIndex: ix];
                let frame: Rect = msg_send![screen, frame];
                let inside = pointer.x >= frame.origin.x
                    && pointer.x < frame.origin.x + frame.size.x
                    && pointer.y >= frame.origin.y
                    && pointer.y < frame.origin.y + frame.size.y;
                if !inside {
                    continue;
                }
                let description: Id = msg_send![screen, deviceDescription];
                let number: Id = msg_send![description, objectForKey: key];
                if number.is_null() {
                    return None;
                }
                let id: u32 = msg_send![number, unsignedIntValue];
                return Some(u64::from(id));
            }
            None
        }
    }

    /// Takes the window off screen without hiding the app, so BenCode's
    /// other windows stay. The Dock icon brings it back (`on_reopen`): a
    /// floating panel does not count as a visible window there. Once the
    /// update asking for it has returned (the module's doc comment).
    pub fn order_out(window: &gpui::Window, cx: &gpui::App) {
        let Some(native) = ns_window(window) else {
            log::warn!("no native window to take off screen");
            return;
        };
        // SAFETY: as in `ns_window`. Retained until the task has used it,
        // so a window closed meanwhile is still a valid object.
        unsafe {
            let _: Id = msg_send![native, retain];
        }
        cx.foreground_executor()
            .spawn(async move {
                // SAFETY: retained above; on the main thread, where GPUI's
                // foreground tasks run.
                unsafe {
                    let _: () = msg_send![native, orderOut: std::ptr::null_mut::<Object>()];
                    let _: () = msg_send![native, release];
                }
            })
            .detach();
    }

    /// Brings BenCode forward, over the other apps (`App::activate(true)`),
    /// once the update asking for it has returned.
    pub fn activate_app(cx: &gpui::App) {
        use objc::class;
        cx.foreground_executor()
            .spawn(async {
                // SAFETY: the shared application, on the main thread.
                unsafe {
                    let app: Id = msg_send![class!(NSApplication), sharedApplication];
                    let _: () = msg_send![app, activateIgnoringOtherApps: YES];
                }
            })
            .detach();
    }
}
