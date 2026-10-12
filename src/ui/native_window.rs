//! What GPUI's windows lack on macOS, asked of their `NSWindow`: the
//! floating chat's panel behaviour (MonoCode `make_panel`) and hiding one
//! window rather than the app (MonoCode `window.hide()`).

#[cfg(not(target_os = "macos"))]
pub fn make_floating_panel(_: &gpui::Window) {}

#[cfg(not(target_os = "macos"))]
pub fn order_out(_: &gpui::Window) {}

#[cfg(target_os = "macos")]
pub use macos::{make_floating_panel, order_out};

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

    /// Takes the window off screen without hiding the app, so BenCode's
    /// other windows stay. The Dock icon brings it back (`on_reopen`): a
    /// floating panel does not count as a visible window there.
    pub fn order_out(window: &gpui::Window) {
        let Some(native) = ns_window(window) else {
            log::warn!("no native window to take off screen");
            return;
        };
        // SAFETY: as in `ns_window`.
        unsafe {
            let _: () = msg_send![native, orderOut: std::ptr::null_mut::<Object>()];
        }
    }
}
