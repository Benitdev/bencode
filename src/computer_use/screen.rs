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
use objc::runtime::{Class, NO, Object, YES};
use objc::{class, msg_send, sel, sel_impl};
use serde::Deserialize;
use serde_json::json;

use crate::mcp::bridge::ToolReply;

type Id = *mut Object;

/// Keep up to this many window screenshots clickable in one turn.
const MAX_WINDOW_FRAMES: usize = 8;
const JPEG_QUALITY: f64 = 0.80;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Jpeg,
    Png,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameTarget {
    Display(u32),
    Window(u32),
}

#[derive(Debug)]
pub struct Frame {
    pub id: u64,
    target: FrameTarget,
    bounds: CGRect,
    width: usize,
    height: usize,
}

impl Frame {
    fn point(&self, x: f64, y: f64) -> Result<CGPoint> {
        match self.target {
            FrameTarget::Display(id) => {
                let display = CGDisplay::new(id);
                if !display.is_active() || !same_bounds(display.bounds(), self.bounds) {
                    bail!(
                        "The screenshot is stale. Take a new screenshot of the intended display."
                    );
                }
            }
            FrameTarget::Window(id) => {
                let window = super::windows::find(id).map_err(|_| {
                    anyhow::anyhow!(
                        "The window is no longer available. Take a new screenshot."
                    )
                })?;
                if !window.on_screen || !super::windows::close_bounds(window.bounds, self.bounds) {
                    bail!(
                        "The window moved, resized, or is no longer on screen. Take a new screenshot."
                    );
                }
            }
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

/// Keep the most recent screenshot for each display and recent windows in the current turn.
#[derive(Default)]
pub struct Frames {
    owner: Option<u64>,
    next: u64,
    entries: Vec<Frame>,
}

impl Frames {
    fn reset_if_new_turn(&mut self, turn: u64) {
        if self.owner != Some(turn) {
            self.entries.clear();
            self.owner = Some(turn);
        }
    }

    fn store(&mut self, turn: u64, frame: Frame) {
        self.reset_if_new_turn(turn);
        self.entries.retain(|old| {
            if old.target == frame.target {
                return false;
            }
            match old.target {
                FrameTarget::Display(id) => CGDisplay::new(id).is_active(),
                FrameTarget::Window(_) => true,
            }
        });
        self.entries.push(frame);
        let window_count = self
            .entries
            .iter()
            .filter(|f| matches!(f.target, FrameTarget::Window(_)))
            .count();
        if window_count > MAX_WINDOW_FRAMES {
            let mut drop_count = window_count - MAX_WINDOW_FRAMES;
            self.entries.retain(|f| {
                if drop_count > 0 && matches!(f.target, FrameTarget::Window(_)) {
                    drop_count -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    pub fn capture(
        &mut self,
        turn: u64,
        display: Option<u32>,
        max_size: u32,
        format: ImageFormat,
    ) -> Result<ToolReply> {
        self.next += 1;
        let (frame, reply) = capture_display(self.next, display, max_size, format)?;
        self.store(turn, frame);
        Ok(reply)
    }

    pub fn capture_window(
        &mut self,
        turn: u64,
        id: u32,
        max_size: u32,
        format: ImageFormat,
    ) -> Result<ToolReply> {
        self.next += 1;
        let (frame, reply) = capture_window_frame(self.next, id, max_size, format)?;
        if let Some(frame) = frame {
            self.store(turn, frame);
        }
        Ok(reply)
    }

    pub fn point(&self, turn: u64, id: u64, x: f64, y: f64) -> Result<CGPoint> {
        let frame = self.find(turn, id).context(
            "That screenshot is stale or belongs to a previous turn. Capture the intended display or window again.",
        )?;
        frame.point(x, y)
    }

    fn find(&self, turn: u64, id: u64) -> Option<&Frame> {
        (self.owner == Some(turn))
            .then(|| self.entries.iter().find(|frame| frame.id == id))
            .flatten()
    }
}

fn capture_display(
    id: u64,
    display: Option<u32>,
    max_size: u32,
    format: ImageFormat,
) -> Result<(Frame, ToolReply)> {
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
    let (bytes, width, height) = match screenshot_manager() {
        Some(manager) => modern_capture(manager, Source::Display(display.id), size, format)?,
        None => legacy_capture(Source::Display(display.id), max_size, format)?,
    };
    if !display.is_active() || !same_bounds(display.bounds(), bounds) {
        bail!("The display changed during capture. Take another screenshot.");
    }
    let frame = Frame {
        id,
        target: FrameTarget::Display(display.id),
        bounds,
        width,
        height,
    };
    let reply = ToolReply::image(
        json!({
            "screenshotId": id,
            "displayId": display.id,
            "width": width,
            "height": height
        })
        .to_string(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
        format.mime(),
    );
    Ok((frame, reply))
}

fn capture_window_frame(
    id: u64,
    window_id: u32,
    max_size: u32,
    format: ImageFormat,
) -> Result<(Option<Frame>, ToolReply)> {
    check_permission()?;
    let window = super::windows::find(window_id)?;
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
    let (bytes, width, height) = match screenshot_manager() {
        Some(manager) => modern_capture(manager, source, size, format)?,
        None => legacy_capture(source, max_size, format)?,
    };
    let clickable = super::windows::find(window_id)
        .is_ok_and(|after| after.on_screen && super::windows::close_bounds(after.bounds, bounds));
    let (frame, meta) = if clickable {
        (
            Some(Frame {
                id,
                target: FrameTarget::Window(window_id),
                bounds,
                width,
                height,
            }),
            json!({
                "screenshotId": id,
                "windowId": window_id,
                "app": window.app,
                "title": window.title,
                "width": width,
                "height": height
            }),
        )
    } else {
        (
            None,
            json!({
                "windowId": window_id,
                "app": window.app,
                "title": window.title,
                "width": width,
                "height": height,
                "note": "Window is not on screen or moved during capture; focus it before clicking."
            }),
        )
    };
    let reply = ToolReply::image(
        meta.to_string(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
        format.mime(),
    );
    Ok((frame, reply))
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

type EncodedImage = (Vec<u8>, usize, usize);

fn modern_capture(
    manager: &'static Class,
    source: Source,
    (width, height): (usize, usize),
    format: ImageFormat,
) -> Result<EncodedImage> {
    let (tx, rx) = mpsc::channel();
    let tx_errors = tx.clone();
    let encode = ConcreteBlock::new(move |image: CGImageRef, error: Id| {
        let result = objc::rc::autoreleasepool(|| {
            if image.is_null() {
                Err(capture_error(error))
            } else {
                encode_image(image, format)
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
                    let _: () = msg_send![config, setShowsCursor: YES];
                    if matches!(source, Source::Window { .. }) {
                        let _: () = msg_send![config, setIgnoreShadowsSingleWindow: YES];
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
        let on_screen_only = match source {
            Source::Display(_) => YES,
            Source::Window { .. } => NO,
        };
        objc::rc::autoreleasepool(|| {
            let _: () = msg_send![content_class,
                getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:on_screen_only completionHandler:&*completion];
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

fn encode_image(image: CGImageRef, format: ImageFormat) -> Result<EncodedImage> {
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
        let (file_type, properties): (usize, Id) = match format {
            ImageFormat::Jpeg => {
                let key: Id = msg_send![
                    class!(NSString),
                    stringWithUTF8String: c"NSImageCompressionFactor".as_ptr()
                ];
                let factor: Id = msg_send![class!(NSNumber), numberWithDouble: JPEG_QUALITY];
                let dict: Id = msg_send![
                    class!(NSDictionary),
                    dictionaryWithObject: factor
                    forKey: key
                ];
                (3usize, dict)
            }
            ImageFormat::Png => (4usize, msg_send![class!(NSDictionary), dictionary]),
        };
        let data: Id = msg_send![bitmap, representationUsingType:file_type properties:properties];
        let _: () = msg_send![bitmap, release];
        if data.is_null() {
            bail!("Could not encode the desktop image.");
        }
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

fn legacy_capture(source: Source, max_size: u32, format: ImageFormat) -> Result<EncodedImage> {
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
        let encoded = encode_image(image, ImageFormat::Png);
        CGImageRelease(image);
        encoded
    })?;
    let (w, h) = output_size(width, height, max_size);
    if (w, h) == (width, height) && format == ImageFormat::Png {
        return Ok((png, width, height));
    }
    let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?;
    let resized = if (w, h) == (width, height) {
        decoded
    } else {
        decoded.resize_exact(w as u32, h as u32, image::imageops::FilterType::Triangle)
    };
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        ImageFormat::Png => resized.write_to(&mut out, image::ImageFormat::Png)?,
        ImageFormat::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out,
                (JPEG_QUALITY * 100.0) as u8,
            );
            encoder.encode_image(&resized.to_rgb8())?;
        }
    }
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
            target: FrameTarget::Display(3),
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
            target: FrameTarget::Display(id as u32),
            bounds: CGRect::new(&CGPoint::new(x, 0.0), &CGSize::new(1000.0, 700.0)),
            width: 2000,
            height: 1400,
        };
        let frames = Frames {
            owner: Some(12),
            next: 9,
            entries: vec![frame(8, -1000.0), frame(9, 0.0)],
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
    fn window_screenshots_map_into_window_bounds_and_evict_oldest() {
        let mut frames = Frames::default();
        for i in 1..=10u64 {
            frames.store(
                5,
                Frame {
                    id: i,
                    target: FrameTarget::Window(i as u32),
                    bounds: CGRect::new(&CGPoint::new(100.0, 50.0), &CGSize::new(800.0, 600.0)),
                    width: 1200,
                    height: 900,
                },
            );
        }
        assert!(frames.find(5, 1).is_none());
        assert!(frames.find(5, 2).is_none());
        let recent = frames.find(5, 10).unwrap();
        let center = recent.map_pixels(600.0, 450.0).unwrap();
        assert_eq!((center.x, center.y), (500.0, 350.0));
    }

    #[test]
    fn encodes_a_quartz_image_as_png_and_smaller_jpeg_without_recording_the_desktop() {
        let source = include_bytes!("../../packaging/macos/icon-1024.png");
        let (png, jpeg) = objc::rc::autoreleasepool(|| unsafe {
            let data: Id =
                msg_send![class!(NSData), dataWithBytes:source.as_ptr() length:source.len()];
            let bitmap: Id = msg_send![class!(NSBitmapImageRep), imageRepWithData:data];
            let image: CGImageRef = msg_send![bitmap, CGImage];
            assert!(!image.is_null());
            (
                encode_image(image, ImageFormat::Png).unwrap(),
                encode_image(image, ImageFormat::Jpeg).unwrap(),
            )
        });
        assert_eq!((png.1, png.2), (1024, 1024));
        assert_eq!((jpeg.1, jpeg.2), (1024, 1024));
        assert!(jpeg.0.len() < png.0.len());
        let decoded_png =
            image::load_from_memory_with_format(&png.0, image::ImageFormat::Png).unwrap();
        let decoded_jpeg =
            image::load_from_memory_with_format(&jpeg.0, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded_png.width(), decoded_png.height()), (1024, 1024));
        assert_eq!((decoded_jpeg.width(), decoded_jpeg.height()), (1024, 1024));
    }
}
