//! ScreenCaptureKit frames and their mapping to Quartz desktop points.
//! The framework is loaded on demand so launch and macOS 11–13 keep working.

#![allow(unexpected_cfgs)]

use std::sync::{OnceLock, mpsc};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use block::ConcreteBlock;
use core_graphics::display::CGDisplay;
use core_graphics::geometry::{CGPoint, CGRect};
use core_graphics::sys::CGImageRef;
use core_graphics::window as quartz;
use objc::runtime::{Class, NO, Object};
use objc::{class, msg_send, sel, sel_impl};
use serde_json::json;

use crate::mcp::bridge::ToolReply;

type Id = *mut Object;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGImageRelease(image: CGImageRef);
    static CGRectNull: CGRect;
}

pub fn request_permission() {
    // SAFETY: prompts for the calling app's screen capture permission.
    unsafe { CGRequestScreenCaptureAccess() };
}

#[derive(Debug)]
pub struct Frame {
    pub id: u64,
    display: u32,
    bounds: CGRect,
    width: usize,
    height: usize,
}

impl Frame {
    fn point(&self, x: f64, y: f64) -> Result<CGPoint> {
        let display = CGDisplay::new(self.display);
        if !display.is_active() || !same_bounds(display.bounds(), self.bounds) {
            bail!("The screenshot is stale. Take a new screenshot of the intended display.");
        }
        self.map_pixels(x, y)
    }

    fn map_pixels(&self, x: f64, y: f64) -> Result<CGPoint> {
        if !(0.0..self.width as f64).contains(&x) || !(0.0..self.height as f64).contains(&y) {
            bail!("Coordinates fall outside the screenshot's pixel bounds.");
        }
        Ok(CGPoint::new(
            self.bounds.origin.x + x * self.bounds.size.width / self.width as f64,
            self.bounds.origin.y + y * self.bounds.size.height / self.height as f64,
        ))
    }
}

pub fn displays() -> Result<ToolReply> {
    let ids = CGDisplay::active_displays()
        .map_err(|code| anyhow::anyhow!("Could not list active displays (Quartz error {code})."))?;
    let displays: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            json!({
                "displayId": id,
                "main": display.is_main(),
                "x": bounds.origin.x,
                "y": bounds.origin.y,
                "width": bounds.size.width,
                "height": bounds.size.height
            })
        })
        .collect();
    Ok(ToolReply::text(json!({ "displays": displays }).to_string()))
}

/// Keep the most recent screenshot for each display in the current turn.
#[derive(Default)]
pub struct Frames {
    owner: Option<u64>,
    next: u64,
    displays: Vec<Frame>,
}

impl Frames {
    pub fn capture(&mut self, turn: u64, display: Option<u32>, max_size: u32) -> Result<ToolReply> {
        self.next += 1;
        let (frame, reply) = capture(self.next, display, max_size)?;
        if self.owner != Some(turn) {
            self.displays.clear();
            self.owner = Some(turn);
        }
        self.displays
            .retain(|old| old.display != frame.display && CGDisplay::new(old.display).is_active());
        self.displays.push(frame);
        Ok(reply)
    }

    pub fn point(&self, turn: u64, id: u64, x: f64, y: f64) -> Result<CGPoint> {
        let frame = self.find(turn, id)
            .context("That screenshot is stale or belongs to a previous turn. Capture the intended display again.")?;
        frame.point(x, y)
    }

    fn find(&self, turn: u64, id: u64) -> Option<&Frame> {
        (self.owner == Some(turn))
            .then(|| self.displays.iter().find(|frame| frame.id == id))
            .flatten()
    }
}

fn capture(id: u64, display: Option<u32>, max_size: u32) -> Result<(Frame, ToolReply)> {
    check_permission()?;
    let display = display.map_or_else(CGDisplay::main, CGDisplay::new);
    if !display.is_active() {
        bail!(
            "Display {} is inactive. Call computer_displays to find an active display.",
            display.id
        );
    }
    let bounds = display.bounds();
    let size = output_size(
        display.pixels_wide() as usize,
        display.pixels_high() as usize,
        max_size,
    );
    let (png, width, height) = match screenshot_manager() {
        Some(manager) => modern_capture(manager, Source::Display(display.id), size)?,
        None => legacy_capture(Source::Display(display.id), max_size)?,
    };
    if !display.is_active() || !same_bounds(display.bounds(), bounds) {
        bail!("The display changed during capture. Take another screenshot.");
    }
    let frame = Frame {
        id,
        display: display.id,
        bounds,
        width,
        height,
    };
    let reply = ToolReply {
        ok: true,
        text:
            json!({ "screenshotId": id, "displayId": display.id, "width": width, "height": height })
                .to_string(),
        image: Some(base64::engine::general_purpose::STANDARD.encode(png)),
    };
    Ok((frame, reply))
}

pub fn capture_window(id: u32, max_size: u32) -> Result<ToolReply> {
    check_permission()?;
    let window = super::windows::find(id)?;
    let source = Source::Window {
        id: window.id,
        owner: window.pid,
    };
    // ScreenCaptureKit expects pixel dimensions; window metadata uses points.
    // Request up to 2× and cap the longest side like display capture.
    let bounds = window.bounds;
    let size = output_size(
        (bounds.size.width * 2.0).ceil() as usize,
        (bounds.size.height * 2.0).ceil() as usize,
        max_size,
    );
    let (png, width, height) = match screenshot_manager() {
        Some(manager) => modern_capture(manager, source, size)?,
        None => legacy_capture(source, max_size)?,
    };
    Ok(ToolReply {
        ok: true,
        text: json!({
            "windowId": id, "app": window.app, "title": window.title,
            "width": width, "height": height,
            "coordinates": "Use a display screenshot for pointer coordinates or computer_ui for native controls."
        }).to_string(),
        image: Some(base64::engine::general_purpose::STANDARD.encode(png)),
    })
}

fn check_permission() -> Result<()> {
    // SAFETY: checks permission without prompting or reading any pixels.
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        bail!(
            "Grant BenCode Screen Recording in macOS Privacy & Security. Use Settings → Integrations → Computer Use to request it, then retry."
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Source {
    Display(u32),
    Window { id: u32, owner: i32 },
}

fn same_bounds(a: CGRect, b: CGRect) -> bool {
    a.origin.x == b.origin.x
        && a.origin.y == b.origin.y
        && a.size.width == b.size.width
        && a.size.height == b.size.height
}

fn output_size(width: usize, height: usize, max: u32) -> (usize, usize) {
    let scale = (f64::from(max) / width.max(height).max(1) as f64).min(1.0);
    (
        (width as f64 * scale).round().max(1.0) as usize,
        (height as f64 * scale).round().max(1.0) as usize,
    )
}

fn screenshot_manager() -> Option<&'static Class> {
    static MANAGER: OnceLock<Option<&'static Class>> = OnceLock::new();
    *MANAGER.get_or_init(|| {
        // SAFETY: this path is an Apple system framework. Keep it loaded for
        // the process lifetime because ObjC classes refer to its code.
        unsafe {
            libc::dlopen(
                c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_LOCAL,
            );
        }
        Class::get("SCScreenshotManager")
    })
}

type Png = (Vec<u8>, usize, usize);

fn modern_capture(
    manager: &'static Class,
    source: Source,
    (width, height): (usize, usize),
) -> Result<Png> {
    let (tx, rx) = mpsc::channel();
    let tx_errors = tx.clone();
    let encode = ConcreteBlock::new(move |image: CGImageRef, error: Id| {
        let result = objc::rc::autoreleasepool(|| {
            if image.is_null() {
                Err(capture_error(error))
            } else {
                encode_png(image)
            }
        });
        let _ = tx.send(result);
    })
    .copy();

    // SAFETY: these ObjC calls match ScreenCaptureKit's public headers. Objects
    // received by the callbacks are used within the callback's lifetime.
    unsafe {
        let content_class =
            Class::get("SCShareableContent").context("Missing SCShareableContent.")?;
        let filter_class = Class::get("SCContentFilter").context("Missing SCContentFilter.")?;
        let config_class =
            Class::get("SCStreamConfiguration").context("Missing SCStreamConfiguration.")?;
        let completion = ConcreteBlock::new(move |content: Id, error: Id| {
            objc::rc::autoreleasepool(|| {
                let begin = || -> Result<()> {
                    if content.is_null() {
                        return Err(capture_error(error));
                    }
                    let (items, expected): (Id, u32) = match source {
                        Source::Display(id) => (msg_send![content, displays], id),
                        Source::Window { id, .. } => (msg_send![content, windows], id),
                    };
                    let count: usize = msg_send![items, count];
                    let item = (0..count).find_map(|index| {
                        let item: Id = msg_send![items, objectAtIndex: index];
                        let id: u32 = match source {
                            Source::Display(_) => msg_send![item, displayID],
                            Source::Window { owner, .. } => {
                                let app: Id = msg_send![item, owningApplication];
                                let pid: i32 = msg_send![app, processID];
                                if pid != owner {
                                    return None;
                                }
                                msg_send![item, windowID]
                            }
                        };
                        (id == expected).then_some(item)
                    }).context("ScreenCaptureKit cannot capture this display or window.")?;
                    let filter: Id = msg_send![filter_class, alloc];
                    let filter: Id = match source {
                        Source::Display(_) => {
                            let excluding: Id = msg_send![class!(NSArray), array];
                            msg_send![filter, initWithDisplay:item excludingWindows:excluding]
                        }
                        Source::Window { .. } => msg_send![filter, initWithDesktopIndependentWindow:item],
                    };
                    if filter.is_null() {
                        bail!("ScreenCaptureKit could not create a capture filter.");
                    }
                    let config: Id = msg_send![config_class, new];
                    let _: () = msg_send![config, setWidth: width];
                    let _: () = msg_send![config, setHeight: height];
                    let _: () = msg_send![config, setShowsCursor: NO];
                    if matches!(source, Source::Window { .. }) {
                        let _: () = msg_send![config, setIgnoreShadowsSingleWindow: objc::runtime::YES];
                    }
                    let _: () = msg_send![manager, captureImageWithFilter:filter configuration:config completionHandler:&*encode];
                    let _: () = msg_send![filter, release];
                    let _: () = msg_send![config, release];
                    Ok(())
                };
                if let Err(err) = begin() {
                    let _ = tx_errors.send(Err(err));
                }
            });
        }).copy();
        objc::rc::autoreleasepool(|| {
            let _: () = msg_send![content_class,
                getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:NO completionHandler:&*completion];
        });
    }

    rx.recv_timeout(Duration::from_secs(12))
        .context("Timed out capturing the desktop")?
}

fn capture_error(error: Id) -> anyhow::Error {
    if error.is_null() {
        return anyhow::anyhow!("macOS returned no screenshot.");
    }
    // SAFETY: the NSError and its description stay alive in the callback.
    unsafe {
        let description: Id = msg_send![error, localizedDescription];
        let text: *const libc::c_char = msg_send![description, UTF8String];
        if text.is_null() {
            return anyhow::anyhow!("macOS returned no screenshot.");
        }
        let text = std::ffi::CStr::from_ptr(text).to_string_lossy();
        anyhow::anyhow!("Could not capture the desktop: {text}")
    }
}

fn encode_png(image: CGImageRef) -> Result<Png> {
    // SAFETY: a live CGImage is provided by the callback (or owned by the
    // caller). Copy encoded bytes before the autorelease pool drains.
    unsafe {
        let bitmap: Id = msg_send![class!(NSBitmapImageRep), alloc];
        let bitmap: Id = msg_send![bitmap, initWithCGImage: image];
        if bitmap.is_null() {
            bail!("Could not read the captured desktop image.");
        }
        let width: usize = msg_send![bitmap, pixelsWide];
        let height: usize = msg_send![bitmap, pixelsHigh];
        let properties: Id = msg_send![class!(NSDictionary), dictionary];
        let data: Id = msg_send![bitmap, representationUsingType:4usize properties:properties];
        let _: () = msg_send![bitmap, release];
        let length: usize = msg_send![data, length];
        let bytes: *const u8 = msg_send![data, bytes];
        if bytes.is_null() || length == 0 {
            bail!("Could not encode the desktop image.");
        }
        Ok((
            std::slice::from_raw_parts(bytes, length).to_vec(),
            width,
            height,
        ))
    }
}

fn legacy_capture(source: Source, max_size: u32) -> Result<Png> {
    // SAFETY: Quartz's display capture is used only when SCScreenshotManager
    // isn't available. CGDisplayCreateImage transfers ownership of a non-null image.
    let (png, width, height) = objc::rc::autoreleasepool(|| unsafe {
        let image = match source {
            Source::Display(id) => core_graphics::display::CGDisplayCreateImage(id),
            Source::Window { id, .. } => core_graphics::display::CGWindowListCreateImage(
                CGRectNull,
                quartz::kCGWindowListOptionIncludingWindow,
                id,
                quartz::kCGWindowImageBoundsIgnoreFraming,
            ),
        };
        if image.is_null() {
            bail!("macOS returned no image for this capture target.");
        }
        let png = encode_png(image);
        CGImageRelease(image);
        png
    })?;
    let (w, h) = output_size(width, height, max_size);
    if (w, h) == (width, height) {
        return Ok((png, width, height));
    }
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.resize_exact(
        w as u32,
        h as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png)?;
    Ok((out.into_inner(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::geometry::CGSize;

    #[test]
    fn retina_thumbnail_coordinates_follow_display_origin_and_scale() {
        let frame = Frame {
            id: 2,
            display: 3,
            bounds: CGRect::new(&CGPoint::new(-1440.0, -100.0), &CGSize::new(1440.0, 900.0)),
            width: 1600,
            height: 1000,
        };
        let point = frame.map_pixels(800.0, 500.0).unwrap();
        assert_eq!((point.x, point.y), (-720.0, 350.0));
        assert!(frame.map_pixels(1600.0, 0.0).is_err());
        assert!(frame.map_pixels(0.0, -1.0).is_err());
        assert_eq!(output_size(1800, 2880, 1600), (1000, 1600));
        assert_eq!(output_size(800, 600, 1600), (800, 600));
    }

    #[test]
    fn source_and_destination_screenshots_share_a_turn() {
        let frame = |id, x| Frame {
            id,
            display: id as u32,
            bounds: CGRect::new(&CGPoint::new(x, 0.0), &CGSize::new(1000.0, 700.0)),
            width: 2000,
            height: 1400,
        };
        let frames = Frames {
            owner: Some(12),
            next: 9,
            displays: vec![frame(8, -1000.0), frame(9, 0.0)],
        };
        let source = frames
            .find(12, 8)
            .unwrap()
            .map_pixels(1000.0, 700.0)
            .unwrap();
        let destination = frames
            .find(12, 9)
            .unwrap()
            .map_pixels(1000.0, 700.0)
            .unwrap();
        assert_eq!((source.x, destination.x), (-500.0, 500.0));
        assert!(frames.find(11, 8).is_none());
    }

    #[test]
    fn encodes_a_quartz_image_as_png_without_recording_the_desktop() {
        let source = include_bytes!("../../packaging/macos/icon-1024.png");
        let (png, width, height) = objc::rc::autoreleasepool(|| unsafe {
            let data: Id =
                msg_send![class!(NSData), dataWithBytes:source.as_ptr() length:source.len()];
            let bitmap: Id = msg_send![class!(NSBitmapImageRep), imageRepWithData:data];
            let image: CGImageRef = msg_send![bitmap, CGImage];
            assert!(!image.is_null());
            encode_png(image)
        })
        .unwrap();
        assert_eq!((width, height), (1024, 1024));
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 1024));
    }
}
