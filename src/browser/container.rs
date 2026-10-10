//! The plain NSView a page's web view sits in, between it and the window's
//! view. WebKit lays a docked Web Inspector out against the web view's
//! superview: were that the window's view, the page and the inspector would
//! take the whole window. In here they share the pane's box, and hiding the
//! box hides both.

#![allow(unexpected_cfgs)]

use std::ptr::NonNull;

use anyhow::{Result, anyhow};
use gpui::{Bounds, Pixels};
use objc::runtime::{BOOL, NO, Object, YES};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use wry::WebViewExtMacOS;

type Id = *mut Object;

/// A `CGRect` as AppKit passes it: an origin, then a size (cocoa's own
/// type is deprecated).
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// `NSViewMinYMargin`: under a view whose y runs up, the room below the
/// box gives, so it keeps its distance to the top as the window resizes.
const KEEP_TOP: usize = 8;
/// `NSViewWidthSizable | NSViewHeightSizable`.
const FILL: usize = 2 | 16;

pub struct Container {
    view: Id,
}

impl Container {
    /// A hidden box in `window`'s view.
    pub fn new(window: &gpui::Window) -> Result<Self> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|err| anyhow!("the window has no native view: {err}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(anyhow!("the window has no AppKit view"));
        };
        let parent = handle.ns_view.as_ptr() as Id;
        // wry sizes a child view from its parent, so not empty.
        let frame = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        unsafe {
            let view: Id = msg_send![class!(NSView), alloc];
            let view: Id = msg_send![view, initWithFrame: frame];
            if view.is_null() {
                return Err(anyhow!("no view for the page"));
            }
            let flipped: BOOL = msg_send![parent, isFlipped];
            let mask = if flipped == NO { KEEP_TOP } else { 0 };
            let _: () = msg_send![view, setAutoresizingMask: mask];
            let _: () = msg_send![view, setHidden: YES];
            let _: () = msg_send![parent, addSubview: view];
            Ok(Self { view })
        }
    }

    /// Makes `page` fill the box and follow its size. With an inspector
    /// docked, WebKit moves the page's edges itself and they still follow.
    pub fn fill(&self, page: &wry::WebView) {
        let page = page.webview();
        let page: Id = &*page as *const _ as Id;
        unsafe {
            let bounds: Rect = msg_send![self.view, bounds];
            let _: () = msg_send![page, setFrame: bounds];
            let _: () = msg_send![page, setAutoresizingMask: FILL];
        }
    }

    /// Puts the box over `bounds` (window coordinates, y running down).
    pub fn set_frame(&self, bounds: Bounds<Pixels>) {
        let (x, y) = (
            f64::from(f32::from(bounds.origin.x)),
            f64::from(f32::from(bounds.origin.y)),
        );
        let (width, height) = (
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        );
        unsafe {
            let parent: Id = msg_send![self.view, superview];
            if parent.is_null() {
                return;
            }
            let flipped: BOOL = msg_send![parent, isFlipped];
            let y = if flipped == NO {
                let parent: Rect = msg_send![parent, bounds];
                parent.height - y - height
            } else {
                y
            };
            let frame = Rect {
                x,
                y,
                width,
                height,
            };
            let _: () = msg_send![self.view, setFrame: frame];
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        let hidden = if hidden { YES } else { NO };
        unsafe {
            let _: () = msg_send![self.view, setHidden: hidden];
        }
    }

    /// Whether the page or its docked inspector is the window's first
    /// responder.
    pub fn has_keys(&self) -> bool {
        unsafe {
            let window: Id = msg_send![self.view, window];
            if window.is_null() {
                return false;
            }
            let responder: Id = msg_send![window, firstResponder];
            if responder.is_null() {
                return false;
            }
            // A window or a text field's editor is a responder but no view.
            let is_view: BOOL = msg_send![responder, isKindOfClass: class!(NSView)];
            if is_view == NO {
                return false;
            }
            let inside: BOOL = msg_send![responder, isDescendantOf: self.view];
            inside != NO
        }
    }

    /// Gives the keyboard to the window's view (GPUI's), when something in
    /// the box has it.
    pub fn release_keys(&self) {
        if !self.has_keys() {
            return;
        }
        unsafe {
            let window: Id = msg_send![self.view, window];
            let parent: Id = msg_send![self.view, superview];
            if window.is_null() || parent.is_null() {
                return;
            }
            let taken: BOOL = msg_send![window, makeFirstResponder: parent];
            if taken == NO {
                log::debug!("browser: keys stay with the page");
            }
        }
    }
}

/// What wry builds the web view in.
impl HasWindowHandle for Container {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let view = NonNull::new(self.view.cast()).ok_or(HandleError::Unavailable)?;
        let handle = RawWindowHandle::AppKit(AppKitWindowHandle::new(view));
        // SAFETY: the view lives as long as `self`, which the handle borrows.
        Ok(unsafe { WindowHandle::borrow_raw(handle) })
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![self.view, release];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_cross_to_appkit_and_back() {
        let rect = Rect {
            x: 1.5,
            y: -2.0,
            width: 640.0,
            height: 480.25,
        };
        // SAFETY: an NSValue only copies the rect in and out.
        let back: Rect = unsafe {
            let value: Id = msg_send![class!(NSValue), valueWithRect: rect];
            msg_send![value, rectValue]
        };
        assert_eq!(
            (back.x, back.y, back.width, back.height),
            (rect.x, rect.y, rect.width, rect.height)
        );
    }
}
