//! MonoCode `src-tauri/src/mono_chat.rs` (its tray) and
//! `mono_chat/menu_bar.rs`: the Bots icon in the macOS menu bar. Its menu
//! lists every Bot with its portrait (a click opens its floating chat),
//! then Quit BenCode. MonoCode's Quick Composer item is left out: BenCode
//! has no quick composer. The icon is BenCode's own mark, where MonoCode
//! draws its logo: the prompt in BenCode's icon (`›_`) as a template image,
//! with a dot while a bot needs you.

use std::sync::{Mutex, OnceLock};

use crate::bots::figure::{self, Figure, Frame, Mood};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// What a click in the menu asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Pick {
    Bot(String),
    Quit,
}

/// One Bot's row.
pub struct Entry {
    pub id: String,
    pub name: String,
    /// MonoCode `portrait`: `PORTRAIT_PIXELS` square, RGBA.
    pub portrait: Vec<u8>,
}

const WIDTH: f64 = 260.0;
const PORTRAIT_SIZE: f64 = 28.0;
pub const PORTRAIT_PIXELS: usize = 56;
/// Pixels per figure cell: its 12 cells span the portrait's middle 48.
const CELL: usize = 4;
/// The menu bar icon: 18pt at 2×.
const ICON_PIXELS: usize = 36;
/// The icon's strokes, in its pixels: the chevron, then the underscore.
const STROKES: [((f32, f32), (f32, f32)); 3] = [
    ((9.5, 10.5), (17.0, 18.0)),
    ((17.0, 18.0), (9.5, 25.5)),
    ((20.5, 25.5), (28.0, 25.5)),
];
const STROKE_WIDTH: f32 = 3.5;
/// The dot while a bot needs you: its centre and radius.
const DOT: (f32, f32, f32) = (28.5, 8.5, 4.5);

fn picks() -> &'static Mutex<Option<UnboundedSender<Pick>>> {
    static PICKS: OnceLock<Mutex<Option<UnboundedSender<Pick>>>> = OnceLock::new();
    PICKS.get_or_init(|| Mutex::new(None))
}

fn send(pick: Pick) {
    match picks().lock() {
        Ok(sender) => {
            if let Some(sender) = sender.as_ref()
                && sender.send(pick).is_err()
            {
                log::debug!("menu bar pick after the app left");
            }
        }
        Err(err) => log::warn!("menu bar pick: {err}"),
    }
}

/// MonoCode `portrait`: a faint circle with the bot's figure centred in
/// it, in `color`.
pub fn portrait(figure: Figure, color: gpui::Hsla) -> Vec<u8> {
    let mut rgba = vec![0u8; PORTRAIT_PIXELS * PORTRAIT_PIXELS * 4];
    let center = PORTRAIT_PIXELS as f64 / 2.0;
    for y in 0..PORTRAIT_PIXELS {
        for x in 0..PORTRAIT_PIXELS {
            let distance =
                ((x as f64 + 0.5 - center).powi(2) + (y as f64 + 0.5 - center).powi(2)).sqrt();
            let alpha = (center - 2.0 - distance + 0.5).clamp(0.0, 1.0) * 0.12;
            let pixel = &mut rgba[(y * PORTRAIT_PIXELS + x) * 4..][..4];
            pixel.copy_from_slice(&[128, 128, 128, (alpha * 255.0).round() as u8]);
        }
    }
    let inset = ((PORTRAIT_PIXELS - figure::GRID as usize * CELL) / 2) as i32;
    let cells = figure::compose(figure, Frame::still(Mood::Idle, false));
    figure::paint_cells(
        &mut rgba,
        PORTRAIT_PIXELS,
        &cells,
        color,
        CELL,
        (inset, inset),
    );
    rgba
}

/// How far `p` is from the segment `a`–`b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// The menu bar icon: `›_` in black on clear, for a template image, and
/// the dot when `alert`. Edges are smoothed by how far each pixel's centre
/// is from a stroke.
pub fn icon(alert: bool) -> Vec<u8> {
    let mut rgba = vec![0u8; ICON_PIXELS * ICON_PIXELS * 4];
    for y in 0..ICON_PIXELS {
        for x in 0..ICON_PIXELS {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let near = STROKES
                .iter()
                .map(|&(a, b)| segment_distance(p, a, b))
                .fold(f32::MAX, f32::min);
            let mut alpha = (STROKE_WIDTH / 2.0 + 0.5 - near).clamp(0.0, 1.0);
            if alert {
                let off = ((p.0 - DOT.0).powi(2) + (p.1 - DOT.1).powi(2)).sqrt();
                alpha = alpha.max((DOT.2 + 0.5 - off).clamp(0.0, 1.0));
            }
            rgba[(y * ICON_PIXELS + x) * 4 + 3] = (alpha * 255.0).round() as u8;
        }
    }
    rgba
}

fn png(rgba: &[u8], side: usize) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(side as u32, side as u32, rgba.to_vec())?;
    let mut out = std::io::Cursor::new(Vec::new());
    match image.write_to(&mut out, image::ImageFormat::Png) {
        Ok(()) => Some(out.into_inner()),
        Err(err) => {
            log::warn!("menu bar image: {err}");
            None
        }
    }
}

pub use platform::{install, set_icon, set_menu, set_visible};

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn install(_icon: &[u8]) -> Option<UnboundedReceiver<Pick>> {
        None
    }

    pub fn set_menu(_entries: &[Entry]) {}

    pub fn set_icon(_icon: &[u8]) {}

    pub fn set_visible(_visible: bool) {}
}

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod platform {
    use std::ffi::CString;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
    use objc::{class, msg_send, sel, sel_impl};

    use super::*;

    type Id = *mut Object;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSSize {
        width: f64,
        height: f64,
    }

    /// `NSVariableStatusItemLength`.
    const VARIABLE_LENGTH: f64 = -1.0;
    /// `NSMenuItemImageVisibilityVisible`: macOS 27 hides menu item images
    /// unless the item asks to keep them.
    const IMAGE_VISIBLE: isize = 1;

    /// The status item and the menus' target, kept for the app's life.
    static STATUS_ITEM: AtomicUsize = AtomicUsize::new(0);
    static TARGET: AtomicUsize = AtomicUsize::new(0);

    fn ns_string(text: &str) -> Id {
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    fn rust_string(string: Id) -> Option<String> {
        if string.is_null() {
            return None;
        }
        let utf8: *const std::ffi::c_char = unsafe { msg_send![string, UTF8String] };
        if utf8.is_null() {
            return None;
        }
        Some(
            unsafe { std::ffi::CStr::from_ptr(utf8) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn image(rgba: &[u8], side: usize, points: f64, template: bool) -> Option<Id> {
        let bytes = png(rgba, side)?;
        unsafe {
            let data: Id = msg_send![class!(NSData),
                dataWithBytes: bytes.as_ptr() as *const std::ffi::c_void
                length: bytes.len()];
            let image: Id = msg_send![class!(NSImage), alloc];
            let image: Id = msg_send![image, initWithData: data];
            if image.is_null() {
                return None;
            }
            let _: () = msg_send![image, setSize: NSSize { width: points, height: points }];
            let _: () = msg_send![image, setTemplate: if template { YES } else { NO }];
            let _: Id = msg_send![image, autorelease];
            Some(image)
        }
    }

    fn symbol(name: &str) -> Option<Id> {
        let image: Id = unsafe {
            msg_send![class!(NSImage),
                imageWithSystemSymbolName: ns_string(name)
                accessibilityDescription: std::ptr::null_mut::<Object>()]
        };
        (!image.is_null()).then_some(image)
    }

    extern "C" fn pick(_: &Object, _: Sel, sender: Id) {
        let id: Id = unsafe { msg_send![sender, representedObject] };
        if let Some(id) = rust_string(id) {
            send(Pick::Bot(id));
        }
    }

    extern "C" fn quit(_: &Object, _: Sel, _sender: Id) {
        send(Pick::Quit);
    }

    fn target() -> Id {
        let existing = TARGET.load(Ordering::Acquire);
        if existing != 0 {
            return existing as Id;
        }
        let mut decl = ClassDecl::new("BenCodeBotMenuTarget", class!(NSObject))
            .expect("the menu target class is declared once");
        unsafe {
            decl.add_method(sel!(pick:), pick as extern "C" fn(&Object, Sel, Id));
            decl.add_method(sel!(quit:), quit as extern "C" fn(&Object, Sel, Id));
        }
        let class: &Class = decl.register();
        let target: Id = unsafe { msg_send![class, new] };
        TARGET.store(target as usize, Ordering::Release);
        target
    }

    fn status_item() -> Option<Id> {
        let item = STATUS_ITEM.load(Ordering::Acquire);
        (item != 0).then_some(item as Id)
    }

    fn show_image(item: Id) {
        unsafe {
            let responds: BOOL =
                msg_send![item, respondsToSelector: sel!(setPreferredImageVisibility:)];
            if responds == YES {
                let _: () = msg_send![item, setPreferredImageVisibility: IMAGE_VISIBLE];
            }
        }
    }

    fn item(title: &str, action: Option<Sel>) -> Id {
        unsafe {
            let item: Id = msg_send![class!(NSMenuItem), alloc];
            let item: Id = match action {
                Some(action) => msg_send![item,
                    initWithTitle: ns_string(title)
                    action: action
                    keyEquivalent: ns_string("")],
                None => msg_send![item,
                    initWithTitle: ns_string(title)
                    action: std::ptr::null::<std::ffi::c_void>()
                    keyEquivalent: ns_string("")],
            };
            let _: Id = msg_send![item, autorelease];
            if action.is_some() {
                let _: () = msg_send![item, setTarget: target()];
            } else {
                let _: () = msg_send![item, setEnabled: NO];
            }
            item
        }
    }

    fn separator() -> Id {
        unsafe { msg_send![class!(NSMenuItem), separatorItem] }
    }

    /// The status item, hidden until `set_visible`. Call once, on the main
    /// thread; picks arrive on the returned channel.
    pub fn install(icon: &[u8]) -> Option<UnboundedReceiver<Pick>> {
        if status_item().is_some() {
            return None;
        }
        let (tx, rx) = unbounded_channel();
        match picks().lock() {
            Ok(mut slot) => *slot = Some(tx),
            Err(err) => {
                log::warn!("menu bar picks: {err}");
                return None;
            }
        }
        unsafe {
            let bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
            let item: Id = msg_send![bar, statusItemWithLength: VARIABLE_LENGTH];
            if item.is_null() {
                return None;
            }
            let _: Id = msg_send![item, retain];
            let _: () = msg_send![item, setVisible: NO];
            let button: Id = msg_send![item, button];
            if !button.is_null() {
                if let Some(image) = image(icon, ICON_PIXELS, 18.0, true) {
                    let _: () = msg_send![button, setImage: image];
                }
                let _: () = msg_send![button, setToolTip: ns_string("Chat with a bot")];
            }
            STATUS_ITEM.store(item as usize, Ordering::Release);
        }
        Some(rx)
    }

    /// The status item's image, swapped for `icon`.
    pub fn set_icon(icon: &[u8]) {
        let Some(item) = status_item() else {
            return;
        };
        unsafe {
            let button: Id = msg_send![item, button];
            if !button.is_null()
                && let Some(image) = image(icon, ICON_PIXELS, 18.0, true)
            {
                let _: () = msg_send![button, setImage: image];
            }
        }
    }

    pub fn set_visible(visible: bool) {
        if let Some(item) = status_item() {
            unsafe { msg_send![item, setVisible: if visible { YES } else { NO }] }
        }
    }

    /// MonoCode `menu` and `style_menu`: the "Bots" header, the Bots (or
    /// how to get one), then Quit.
    pub fn set_menu(entries: &[Entry]) {
        let Some(status) = status_item() else {
            return;
        };
        unsafe {
            let menu: Id = msg_send![class!(NSMenu), alloc];
            let menu: Id = msg_send![menu, initWithTitle: ns_string("Bots")];
            let _: Id = msg_send![menu, autorelease];
            let _: () = msg_send![menu, setAutoenablesItems: NO];
            let _: () = msg_send![menu, setMinimumWidth: WIDTH];
            let font: Id = msg_send![class!(NSFont), systemFontOfSize: 14.0f64];
            let _: () = msg_send![menu, setFont: font];

            let header_class = class!(NSMenuItem);
            let section: BOOL =
                msg_send![header_class, respondsToSelector: sel!(sectionHeaderWithTitle:)];
            let header: Id = if section == YES {
                msg_send![header_class, sectionHeaderWithTitle: ns_string("Bots")]
            } else {
                item("Bots", None)
            };
            let _: () = msg_send![menu, addItem: header];
            let _: () = msg_send![menu, addItem: separator()];
            if entries.is_empty() {
                let _: () =
                    msg_send![menu, addItem: item("Create a bot in BenCode to chat here", None)];
            }
            for entry in entries {
                let row = item(&entry.name, Some(sel!(pick:)));
                let _: () = msg_send![row, setRepresentedObject: ns_string(&entry.id)];
                let _: () = msg_send![row, setToolTip: ns_string("Open floating chat")];
                if let Some(portrait) =
                    image(&entry.portrait, PORTRAIT_PIXELS, PORTRAIT_SIZE, false)
                {
                    let _: () = msg_send![row, setImage: portrait];
                    show_image(row);
                }
                let _: () = msg_send![menu, addItem: row];
            }
            let _: () = msg_send![menu, addItem: separator()];
            let quit = item("Quit BenCode", Some(sel!(quit:)));
            if let Some(power) = symbol("power") {
                let _: () = msg_send![quit, setImage: power];
                show_image(quit);
            }
            let _: () = msg_send![menu, addItem: quit];
            let _: () = msg_send![status, setMenu: menu];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portrait_centres_the_figure_in_a_faint_circle() {
        let figure = Figure::new(figure::Body::Block, figure::Eyes::Dot, figure::Gear::None);
        let image = portrait(figure, gpui::hsla(0.0, 1.0, 0.5, 1.0));
        assert_eq!(image.len(), PORTRAIT_PIXELS * PORTRAIT_PIXELS * 4);
        let pixel = |x: usize, y: usize| &image[(y * PORTRAIT_PIXELS + x) * 4..][..4];
        assert_eq!(pixel(0, 0)[3], 0, "corners stay clear");
        assert_eq!(pixel(28, 30)[3], 255, "the figure fills its middle");
        assert_eq!(pixel(28, 4)[3], 31, "a clear cell shows the circle");
    }

    #[test]
    fn the_icon_is_the_prompt_and_a_dot_when_a_bot_needs_you() {
        let alpha = |image: &[u8], x: usize, y: usize| image[(y * ICON_PIXELS + x) * 4 + 3];
        let calm = icon(false);
        assert_eq!(alpha(&calm, 0, 0), 0);
        assert_eq!(alpha(&calm, 13, 14), 255, "on the chevron");
        assert_eq!(alpha(&calm, 24, 25), 255, "on the underscore");
        assert_eq!(alpha(&calm, 28, 8), 0, "no dot");
        assert!(calm.chunks(4).all(|px| px[..3] == [0, 0, 0]), "black only");
        assert_eq!(alpha(&icon(true), 28, 8), 255, "the dot");
        assert!(png(&calm, ICON_PIXELS).is_some());
    }
}
