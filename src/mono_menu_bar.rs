//! MonoCode `src-tauri/src/mono_chat.rs` (its tray) and
//! `mono_chat/menu_bar.rs`: the Monos icon in the macOS menu bar. Its menu
//! lists every Mono with its portrait (a click opens its floating chat),
//! then Quit BenCode. MonoCode's Quick Composer item is left out: BenCode
//! has no quick composer. The icon is BenCode's own mark for it (the
//! invader mascot as a template image), where MonoCode draws its logo.

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// What a click in the menu asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Pick {
    Mono(String),
    Quit,
}

/// One Mono's row.
pub struct Entry {
    pub id: String,
    pub name: String,
    /// MonoCode `portrait`: `PORTRAIT_PIXELS` square, RGBA.
    pub portrait: Vec<u8>,
}

const WIDTH: f64 = 260.0;
const PORTRAIT_SIZE: f64 = 28.0;
pub const PORTRAIT_PIXELS: usize = 56;
/// Pixels per sprite cell: the 8-cell sprite spans the middle 32 pixels,
/// as MonoCode's 16-unit one at 2× does.
const CELL: usize = 4;
/// The menu bar icon: 18pt at 2×, the sprite inset by a cell's margin.
const ICON_PIXELS: usize = 36;

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

/// MonoCode `portrait`: a faint circle with the sprite (`rows`, `#` filled)
/// centred in it, in `rgb`.
pub fn portrait(rows: &[&str; 8], rgb: [u8; 3]) -> Vec<u8> {
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
    let inset = (PORTRAIT_PIXELS - 8 * CELL) / 2;
    paint(
        &mut rgba,
        PORTRAIT_PIXELS,
        rows,
        inset,
        [rgb[0], rgb[1], rgb[2], 255],
    );
    rgba
}

/// The menu bar icon: the sprite in black, for a template image.
pub fn icon(rows: &[&str; 8]) -> Vec<u8> {
    let mut rgba = vec![0u8; ICON_PIXELS * ICON_PIXELS * 4];
    paint(
        &mut rgba,
        ICON_PIXELS,
        rows,
        (ICON_PIXELS - 8 * CELL) / 2,
        [0, 0, 0, 255],
    );
    rgba
}

fn paint(rgba: &mut [u8], side: usize, rows: &[&str; 8], inset: usize, fill: [u8; 4]) {
    for (cy, row) in rows.iter().enumerate() {
        for (cx, cell) in row.bytes().enumerate() {
            if cell != b'#' {
                continue;
            }
            for y in inset + cy * CELL..inset + (cy + 1) * CELL {
                for x in inset + cx * CELL..inset + (cx + 1) * CELL {
                    rgba[(y * side + x) * 4..][..4].copy_from_slice(&fill);
                }
            }
        }
    }
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

pub use platform::{install, set_menu, set_visible};

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn install(_icon: &[u8]) -> Option<UnboundedReceiver<Pick>> {
        None
    }

    pub fn set_menu(_entries: &[Entry]) {}

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
            send(Pick::Mono(id));
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
        let mut decl = ClassDecl::new("BenCodeMonoMenuTarget", class!(NSObject))
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
                let _: () = msg_send![button, setToolTip: ns_string("Chat with a Mono")];
            }
            STATUS_ITEM.store(item as usize, Ordering::Release);
        }
        Some(rx)
    }

    pub fn set_visible(visible: bool) {
        if let Some(item) = status_item() {
            unsafe { msg_send![item, setVisible: if visible { YES } else { NO }] }
        }
    }

    /// MonoCode `menu` and `style_menu`: the "Monos" header, the Monos (or
    /// how to get one), then Quit.
    pub fn set_menu(entries: &[Entry]) {
        let Some(status) = status_item() else {
            return;
        };
        unsafe {
            let menu: Id = msg_send![class!(NSMenu), alloc];
            let menu: Id = msg_send![menu, initWithTitle: ns_string("Monos")];
            let _: Id = msg_send![menu, autorelease];
            let _: () = msg_send![menu, setAutoenablesItems: NO];
            let _: () = msg_send![menu, setMinimumWidth: WIDTH];
            let font: Id = msg_send![class!(NSFont), systemFontOfSize: 14.0f64];
            let _: () = msg_send![menu, setFont: font];

            let header_class = class!(NSMenuItem);
            let section: BOOL =
                msg_send![header_class, respondsToSelector: sel!(sectionHeaderWithTitle:)];
            let header: Id = if section == YES {
                msg_send![header_class, sectionHeaderWithTitle: ns_string("Monos")]
            } else {
                item("Monos", None)
            };
            let _: () = msg_send![menu, addItem: header];
            let _: () = msg_send![menu, addItem: separator()];
            if entries.is_empty() {
                let _: () =
                    msg_send![menu, addItem: item("Create a Mono in BenCode to chat here", None)];
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

    const BLOCK: [&str; 8] = [
        "########", "########", "########", "########", "########", "########", "########",
        "#######.",
    ];

    #[test]
    fn the_portrait_centres_the_sprite_in_a_faint_circle() {
        let image = portrait(&BLOCK, [255, 0, 0]);
        assert_eq!(image.len(), PORTRAIT_PIXELS * PORTRAIT_PIXELS * 4);
        let pixel = |x: usize, y: usize| &image[(y * PORTRAIT_PIXELS + x) * 4..][..4];
        assert_eq!(pixel(0, 0), [128, 128, 128, 0], "corners stay clear");
        assert_eq!(pixel(12, 12), [255, 0, 0, 255], "the sprite starts 12 in");
        assert_eq!(
            pixel(43, 43),
            [128, 128, 128, 31],
            "a clear cell shows the circle"
        );
        assert_eq!(pixel(8, 28)[3], 31, "the margin is the circle");
    }

    #[test]
    fn the_icon_is_black_on_clear() {
        let image = icon(&BLOCK);
        let pixel = |x: usize, y: usize| &image[(y * ICON_PIXELS + x) * 4..][..4];
        assert_eq!(pixel(0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(2, 2), [0, 0, 0, 255]);
        assert!(png(&image, ICON_PIXELS).is_some());
    }
}
