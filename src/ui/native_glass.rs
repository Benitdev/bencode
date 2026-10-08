//! The window's glass on macOS 26 and later: an `NSGlassEffectView` (Liquid
//! Glass) behind GPUI's view, in place of the `NSVisualEffectView` that
//! GPUI's `WindowBackgroundAppearance::Blurred` adds. Built against the
//! macOS 26 SDK, that view shows no blur under Tahoe's design; the glass
//! view is the public API for it. Older macOS has no such class, and the
//! window keeps GPUI's blur (`ui/glass.rs`).
//!
//! The glass is a sibling under GPUI's view in the window's content view,
//! where GPUI puts its own blur view; GPUI's window is then `Transparent`
//! and paints the tint (`Glass::root`) over it.

/// An `NSGlassEffectView` under GPUI's view; dropping it takes it out.
pub struct NativeGlass {
    #[cfg(target_os = "macos")]
    view: *mut objc::runtime::Object,
    /// Whether the corner radius was taken in full screen, where the
    /// window has square corners.
    #[cfg(target_os = "macos")]
    fullscreen: bool,
}

#[cfg(not(target_os = "macos"))]
impl NativeGlass {
    pub fn available() -> bool {
        false
    }

    pub fn install(_: &gpui::Window) -> Option<Self> {
        None
    }

    pub fn sync(&mut self, _: &gpui::Window) {}
}

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod macos {
    use cocoa::appkit::{NSView, NSViewHeightSizable, NSViewWidthSizable};
    use objc::runtime::{BOOL, Class, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::NativeGlass;

    type Id = *mut Object;

    const CLASS: &str = "NSGlassEffectView";
    /// `NSGlassEffectViewStyleRegular`; `Clear` (1) blurs too little to
    /// read text over.
    const STYLE_REGULAR: isize = 0;
    /// `NSWindowBelow`.
    const BELOW: isize = -1;

    impl NativeGlass {
        /// `NSGlassEffectView` exists: macOS 26 or later.
        pub fn available() -> bool {
            Class::get(CLASS).is_some()
        }

        /// Puts a glass under GPUI's view in `window`; `None` when the
        /// class or the view cannot be had.
        pub fn install(window: &gpui::Window) -> Option<Self> {
            let class = Class::get(CLASS)?;
            // `Window::window_handle` is GPUI's own handle; this is the
            // platform's.
            let handle = HasWindowHandle::window_handle(window).ok()?;
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return None;
            };
            let gpui_view = appkit.ns_view.as_ptr() as Id;
            // SAFETY: AppKit calls on the main thread, where GPUI renders.
            // `gpui_view` is GPUI's live NSView; `alloc` / `initWithFrame:`
            // give a +1 view this struct releases on drop, and the content
            // view retains it again as a subview.
            unsafe {
                let content = gpui_view.superview();
                if content.is_null() {
                    return None;
                }
                let view: Id = msg_send![class, alloc];
                let view = view.initWithFrame_(content.bounds());
                if view.is_null() {
                    return None;
                }
                view.setAutoresizingMask_(NSViewWidthSizable | NSViewHeightSizable);
                let _: () = msg_send![view, setStyle: STYLE_REGULAR];
                // The glass is on only in BenCode's dark theme, which need
                // not be the system's.
                let name: Id = msg_send![
                    class!(NSString),
                    stringWithUTF8String: c"NSAppearanceNameDarkAqua".as_ptr()
                ];
                let dark: Id = msg_send![class!(NSAppearance), appearanceNamed: name];
                if !dark.is_null() {
                    view.setAppearance(dark);
                }
                let _: () = msg_send![
                    content,
                    addSubview: view
                    positioned: BELOW
                    relativeTo: gpui_view
                ];
                let mut glass = NativeGlass {
                    view,
                    fullscreen: false,
                };
                glass.set_corner_radius(window.is_fullscreen());
                Some(glass)
            }
        }

        /// Follows the window into and out of full screen, where its
        /// corners are square.
        pub fn sync(&mut self, window: &gpui::Window) {
            let fullscreen = window.is_fullscreen();
            if fullscreen != self.fullscreen {
                self.set_corner_radius(fullscreen);
            }
        }

        /// The window's own corner radius, so the glass does not fill the
        /// corners outside it. `_cornerRadius` is private, so it is asked
        /// for only when the window answers to it; without it the glass
        /// keeps square corners.
        fn set_corner_radius(&mut self, fullscreen: bool) {
            self.fullscreen = fullscreen;
            // SAFETY: main-thread AppKit calls on the live glass view and
            // its window; `_cornerRadius` returns a CGFloat when present.
            unsafe {
                let window: Id = msg_send![self.view, window];
                if window.is_null() {
                    return;
                }
                let radius = if fullscreen {
                    0.0
                } else {
                    let answers: BOOL = msg_send![window, respondsToSelector: sel!(_cornerRadius)];
                    if answers == NO {
                        return;
                    }
                    let radius: f64 = msg_send![window, _cornerRadius];
                    radius
                };
                let _: () = msg_send![self.view, setCornerRadius: radius];
            }
        }
    }

    impl Drop for NativeGlass {
        fn drop(&mut self) {
            // SAFETY: main thread; the view is ours (+1 from `install`).
            unsafe {
                self.view.removeFromSuperview();
                let _: () = msg_send![self.view, release];
            }
        }
    }
}
