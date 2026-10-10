//! What wry does not pass on from a WKWebView. A PNG of what it shows
//! (`takeSnapshotWithConfiguration:`): the page draws in WebKit's own
//! process, so the view cannot be drawn into a bitmap from here. And
//! whether it is loading.

#![allow(unexpected_cfgs)]

use std::cell::RefCell;

use anyhow::{Result, anyhow};
use block::ConcreteBlock;
use objc::runtime::{BOOL, NO, Object};
use objc::{class, msg_send, sel, sel_impl};
use wry::WebViewExtMacOS;

type Id = *mut Object;

/// `NSBitmapImageFileTypePNG`.
const PNG: usize = 4;

/// The widest a snapshot may be, for a view `view_width` points wide.
pub struct Limit {
    pub view_width: f64,
    pub max_pixels: u32,
}

/// `WKWebView.isLoading`: a load asked for, under way, or neither.
pub fn is_loading(view: &wry::WebView) -> bool {
    let webview = view.webview();
    let webview: Id = &*webview as *const _ as Id;
    let loading: BOOL = unsafe { msg_send![webview, isLoading] };
    loading != NO
}

/// A configuration that keeps the snapshot within `limit`, or nil when it
/// already is: WebKit draws `snapshotWidth` points at the window's scale,
/// so the PNG made on the main thread is the small one.
unsafe fn configuration(webview: Id, limit: &Limit) -> Id {
    let nil: Id = std::ptr::null_mut();
    unsafe {
        let window: Id = msg_send![webview, window];
        if window.is_null() {
            return nil;
        }
        let scale: f64 = msg_send![window, backingScaleFactor];
        let max = f64::from(limit.max_pixels);
        if scale <= 0.0 || limit.view_width * scale <= max {
            return nil;
        }
        let width: Id = msg_send![class!(NSNumber), numberWithDouble: max / scale];
        let config: Id = msg_send![class!(WKSnapshotConfiguration), new];
        let _: () = msg_send![config, setSnapshotWidth: width];
        config
    }
}

/// Asks WebKit for a snapshot; `done` gets the PNG on the main thread.
pub fn take(
    view: &wry::WebView,
    limit: Option<Limit>,
    done: impl FnOnce(Result<Vec<u8>>) + 'static,
) {
    let webview = view.webview();
    let webview: Id = &*webview as *const _ as Id;
    let done = RefCell::new(Some(done));
    let handler = ConcreteBlock::new(move |image: Id, _error: Id| {
        let Some(done) = done.borrow_mut().take() else {
            return;
        };
        if image.is_null() {
            done(Err(anyhow!("WebKit took no snapshot (is the page shown?)")));
            return;
        }
        done(unsafe { png_bytes(image) });
    })
    .copy();
    unsafe {
        let config = match &limit {
            Some(limit) => configuration(webview, limit),
            None => std::ptr::null_mut(),
        };
        let _: () =
            msg_send![webview, takeSnapshotWithConfiguration: config completionHandler: &*handler];
        if !config.is_null() {
            // WebKit read it; `new` left this owning it.
            let _: () = msg_send![config, release];
        }
    }
}

/// An `NSImage` as PNG bytes, through its TIFF and a bitmap rep.
unsafe fn png_bytes(image: Id) -> Result<Vec<u8>> {
    unsafe {
        let tiff: Id = msg_send![image, TIFFRepresentation];
        if tiff.is_null() {
            return Err(anyhow!("the snapshot has no bitmap"));
        }
        let rep: Id = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        if rep.is_null() {
            return Err(anyhow!("the snapshot has no bitmap"));
        }
        let properties: Id = msg_send![class!(NSDictionary), dictionary];
        let png: Id = msg_send![rep, representationUsingType: PNG properties: properties];
        if png.is_null() {
            return Err(anyhow!("the snapshot could not be made a PNG"));
        }
        let length: usize = msg_send![png, length];
        let bytes: *const u8 = msg_send![png, bytes];
        if bytes.is_null() || length == 0 {
            return Err(anyhow!("the snapshot is empty"));
        }
        Ok(std::slice::from_raw_parts(bytes, length).to_vec())
    }
}
