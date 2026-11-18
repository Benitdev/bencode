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
pub fn present_floating_panel(window: &gpui::Window, _: &gpui::App) {
    window.activate_window();
}

#[cfg(not(target_os = "macos"))]
pub fn order_out(_: &gpui::Window, _: &gpui::App) {}

#[cfg(not(target_os = "macos"))]
pub fn is_ordered_out(_: &gpui::Window) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn on_app_active(_: &gpui::App, _: impl Fn(&mut gpui::App) + 'static) {}

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
    activate_app, display_under_pointer, invalidate_shadow, is_ordered_out, make_floating_panel,
    on_app_active, order_out, present_floating_panel,
};

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod macos {
    use std::sync::Once;

    use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type Id = *mut Object;

    type Imp = extern "C" fn(&Object, Sel) -> BOOL;

    unsafe extern "C" {
        fn class_replaceMethod(
            cls: *const Class,
            name: Sel,
            imp: Imp,
            types: *const std::ffi::c_char,
        ) -> Option<Imp>;
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSApplicationDidBecomeActiveNotification: Id;
    }

    /// `NSWindowStyleMaskResizable`.
    const RESIZABLE: usize = 1 << 3;
    /// `NSWindowStyleMaskNonactivatingPanel`.
    const NONACTIVATING_PANEL: usize = 1 << 7;
    /// `NSWindowCollectionBehaviorCanJoinAllSpaces`.
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    /// `NSWindowCollectionBehaviorTransient`.
    const TRANSIENT: usize = 1 << 3;
    /// `NSWindowCollectionBehaviorIgnoresCycle`.
    const IGNORES_CYCLE: usize = 1 << 6;
    /// `NSWindowCollectionBehaviorFullScreenAuxiliary`.
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
    /// `NSWindowCloseButton`, `NSWindowMiniaturizeButton`, `NSWindowZoomButton`.
    const BUTTONS: [usize; 3] = [0, 1, 2];

    extern "C" fn can_become_main_window(_: &Object, _: Sel) -> BOOL {
        NO
    }

    /// Makes GPUI's `GPUIPanel` unable to become main window: becoming main
    /// (or activating BenCode) pulls BenCode's main window forward when the
    /// floating chat is clicked or closed. The class is changed, not the
    /// window's: AppKit observes a window through KVO, which re-classes it
    /// itself, and a window re-classed under KVO throws as it is freed. The
    /// floating chat is the only `GPUIPanel` BenCode opens.
    fn panels_never_main() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let Some(panel) = Class::get("GPUIPanel") else {
                log::warn!("GPUI's panel class is missing; the floating chat can become main");
                return;
            };
            // SAFETY: `c@:` is `-[NSWindow canBecomeMainWindow]`'s signature,
            // which `can_become_main_window` matches.
            unsafe {
                class_replaceMethod(
                    panel,
                    sel!(canBecomeMainWindow),
                    can_become_main_window,
                    c"c@:".as_ptr(),
                );
            }
        });
    }

    /// The window's `NSView` and `NSWindow`, if AppKit backs it.
    fn ns_view_and_window(window: &gpui::Window) -> Option<(Id, Id)> {
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
        (!native.is_null()).then_some((view, native))
    }

    /// The window's `NSWindow`, or nil.
    fn ns_window(window: &gpui::Window) -> Option<Id> {
        ns_view_and_window(window).map(|(_, native)| native)
    }

    /// GPUI gives a window without a title bar no resizable style, whatever
    /// `is_resizable` says, and its floating `NSPanel` activates BenCode and
    /// hides when BenCode loses focus. Making it a non-activating panel that
    /// never becomes main keeps other apps in front while it takes keys and when it closes.
    pub fn make_floating_panel(window: &gpui::Window) {
        let Some(native) = ns_window(window) else {
            log::warn!("the floating chat has no native window to set up");
            return;
        };
        panels_never_main();
        // SAFETY: as in `ns_window`; each button is borrowed and checked for
        // nil.
        unsafe {
            let mask: usize = msg_send![native, styleMask];
            let _: () = msg_send![native, setStyleMask: mask | RESIZABLE | NONACTIVATING_PANEL];
            // Setting `NonactivatingPanel` after creation does not update
            // WindowServer's activation tag on its own.
            let prevents = sel!(_setPreventsActivation:);
            let responds: BOOL = msg_send![native, respondsToSelector: prevents];
            if responds == YES {
                let _: () = msg_send![native, _setPreventsActivation: YES];
            }
            // A resizable window gets its traffic lights back; MonoCode's
            // panel has none.
            for button in BUTTONS {
                let button: Id = msg_send![native, standardWindowButton: button];
                if !button.is_null() {
                    let _: () = msg_send![button, setHidden: YES];
                }
            }
            let _: () = msg_send![native, setFloatingPanel: YES];
            let _: () = msg_send![native, setBecomesKeyOnlyIfNeeded: NO];
            let _: () = msg_send![native, setHidesOnDeactivate: NO];
            let _: () = msg_send![native, setCanHide: NO];
            let _: () = msg_send![
                native,
                setCollectionBehavior: CAN_JOIN_ALL_SPACES
                    | FULL_SCREEN_AUXILIARY
                    | TRANSIENT
                    | IGNORES_CYCLE
            ];
        }
    }

    /// Shows the floating panel over whichever app is active and hands it the
    /// keyboard without activating BenCode, once the update asking for it has
    /// returned (the module's doc comment).
    pub fn present_floating_panel(window: &gpui::Window, cx: &gpui::App) {
        let Some((view, native)) = ns_view_and_window(window) else {
            log::warn!("no native window to present floating panel");
            return;
        };
        // SAFETY: retained until the task has used them, so a window closed
        // meanwhile stays valid.
        unsafe {
            let _: Id = msg_send![view, retain];
            let _: Id = msg_send![native, retain];
        }
        cx.foreground_executor()
            .spawn(async move {
                // SAFETY: retained above; on the main thread, where GPUI's
                // foreground tasks run.
                unsafe {
                    let visible: BOOL = msg_send![native, isVisible];
                    if visible == NO {
                        let _: () = msg_send![view, setNeedsDisplay: YES];
                    }
                    let _: () = msg_send![native, orderFrontRegardless];
                    let _: () = msg_send![native, makeKeyWindow];
                    let _: () = msg_send![native, release];
                    let _: () = msg_send![view, release];
                }
            })
            .detach();
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

    /// Whether the window is off screen by `order_out`: neither shown nor
    /// minimized to the Dock.
    pub fn is_ordered_out(window: &gpui::Window) -> bool {
        let Some(native) = ns_window(window) else {
            return false;
        };
        // SAFETY: as in `ns_window`.
        unsafe {
            let visible: BOOL = msg_send![native, isVisible];
            let minimized: BOOL = msg_send![native, isMiniaturized];
            visible == NO && minimized == NO
        }
    }

    /// Calls `on_active` each time BenCode becomes the active app (⌘Tab,
    /// the Dock, `activate_app`), in an update of its own: AppKit's
    /// notification only wakes a task, which runs when GPUI is free.
    pub fn on_app_active(cx: &gpui::App, on_active: impl Fn(&mut gpui::App) + 'static) {
        use objc::class;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let handler = block::ConcreteBlock::new(move |_note: Id| {
            if tx.send(()).is_err() {
                log::debug!("BenCode became active after its watcher ended");
            }
        })
        .copy();
        // SAFETY: on the main thread; the centre copies the block and keeps
        // the observer it returns for the app's life (it is never removed).
        unsafe {
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let queue: Id = msg_send![class!(NSOperationQueue), mainQueue];
            let _observer: Id = msg_send![
                center,
                addObserverForName: NSApplicationDidBecomeActiveNotification
                object: std::ptr::null_mut::<Object>()
                queue: queue
                usingBlock: &*handler
            ];
        }
        cx.spawn(async move |cx| {
            while rx.recv().await.is_some() {
                cx.update(|cx| on_active(cx));
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
