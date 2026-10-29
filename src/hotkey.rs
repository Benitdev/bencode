//! A system-wide shortcut (BenCode's own): Carbon's `RegisterEventHotKey`,
//! which reaches BenCode from any app without an Accessibility grant (a
//! global key monitor needs one). One shortcut at a time; its presses
//! arrive on the channel [`listen`] returns.

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Carbon's modifier bits (`shiftKey`, `optionKey`, `controlKey`).
pub const SHIFT: u32 = 1 << 9;
pub const OPTION: u32 = 1 << 11;
pub const CONTROL: u32 = 1 << 12;
/// `kVK_Space`.
pub const SPACE: u32 = 49;

fn presses() -> &'static Mutex<Option<UnboundedSender<()>>> {
    static PRESSES: OnceLock<Mutex<Option<UnboundedSender<()>>>> = OnceLock::new();
    PRESSES.get_or_init(|| Mutex::new(None))
}

fn send() {
    match presses().lock() {
        Ok(sender) => {
            if let Some(sender) = sender.as_ref()
                && sender.send(()).is_err()
            {
                log::debug!("shortcut pressed after the app left");
            }
        }
        Err(err) => log::warn!("shortcut press: {err}"),
    }
}

pub use platform::{listen, register, unregister};

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn listen() -> Option<UnboundedReceiver<()>> {
        None
    }

    pub fn register(_code: u32, _modifiers: u32) -> Result<(), i32> {
        Err(-1)
    }

    pub fn unregister() {}
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyId {
        signature: u32,
        id: u32,
    }

    type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(
            target: *mut c_void,
            handler: Handler,
            count: usize,
            types: *const EventTypeSpec,
            user_data: *mut c_void,
            out: *mut *mut c_void,
        ) -> i32;
        fn RegisterEventHotKey(
            code: u32,
            modifiers: u32,
            id: EventHotKeyId,
            target: *mut c_void,
            options: u32,
            out: *mut *mut c_void,
        ) -> i32;
        fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
    }

    /// `kEventClassKeyboard` and `kEventHotKeyPressed`.
    const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
    const PRESSED: u32 = 5;
    /// The shortcut's signature: BenCode's.
    const SIGNATURE: u32 = u32::from_be_bytes(*b"BnCd");

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// The registered shortcut's `EventHotKeyRef`, or 0.
    static HOT_KEY: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn pressed(_next: *mut c_void, _event: *mut c_void, _data: *mut c_void) -> i32 {
        send();
        0
    }

    /// Starts listening for the shortcut's presses: once, on the main
    /// thread. Nothing is registered until [`register`].
    pub fn listen() -> Option<UnboundedReceiver<()>> {
        if INSTALLED.swap(true, Ordering::AcqRel) {
            return None;
        }
        let (tx, rx) = unbounded_channel();
        match presses().lock() {
            Ok(mut slot) => *slot = Some(tx),
            Err(err) => {
                log::warn!("shortcut presses: {err}");
                return None;
            }
        }
        let spec = EventTypeSpec {
            class: KEYBOARD,
            kind: PRESSED,
        };
        let mut handler = std::ptr::null_mut();
        // SAFETY: the application target lives as long as the app; the
        // spec is read during the call; `pressed` never unwinds.
        let status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                pressed,
                1,
                &spec,
                std::ptr::null_mut(),
                &mut handler,
            )
        };
        if status != 0 {
            log::warn!("could not listen for the summon shortcut: OSStatus {status}");
            return None;
        }
        Some(rx)
    }

    /// The shortcut `code` with `modifiers` in place of the one before; the
    /// OSStatus when macOS refuses it (another app holds it).
    pub fn register(code: u32, modifiers: u32) -> Result<(), i32> {
        unregister();
        let id = EventHotKeyId {
            signature: SIGNATURE,
            id: 1,
        };
        let mut hot_key = std::ptr::null_mut();
        // SAFETY: as in `listen`; `hot_key` is written before it returns.
        let status = unsafe {
            RegisterEventHotKey(
                code,
                modifiers,
                id,
                GetApplicationEventTarget(),
                0,
                &mut hot_key,
            )
        };
        if status != 0 || hot_key.is_null() {
            return Err(status);
        }
        HOT_KEY.store(hot_key as usize, Ordering::Release);
        Ok(())
    }

    /// No shortcut any more.
    pub fn unregister() {
        let hot_key = HOT_KEY.swap(0, Ordering::AcqRel);
        if hot_key != 0 {
            // SAFETY: the ref `register` stored, unregistered once.
            let status = unsafe { UnregisterEventHotKey(hot_key as *mut c_void) };
            if status != 0 {
                log::warn!("could not drop the summon shortcut: OSStatus {status}");
            }
        }
    }
}
