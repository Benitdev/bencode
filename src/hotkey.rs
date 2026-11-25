//! System-wide shortcuts through Carbon's RegisterEventHotKey. The app
//! installs one handler and passes each shortcut's presses to its consumer.

use tokio::sync::mpsc::UnboundedReceiver;

/// Carbon's modifier bits.
#[cfg(target_os = "macos")]
pub const COMMAND: u32 = 1 << 8;
pub const SHIFT: u32 = 1 << 9;
pub const OPTION: u32 = 1 << 11;
pub const CONTROL: u32 = 1 << 12;
pub const SPACE: u32 = 49;
#[cfg(target_os = "macos")]
pub const PERIOD: u32 = 47;

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Shortcut {
    Summon,
    #[cfg(target_os = "macos")]
    StopComputerUse,
}

#[cfg(target_os = "macos")]
const SHORTCUT_COUNT: usize = 2;

pub use platform::{listen, register, unregister};

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn listen(_shortcut: Shortcut) -> Option<UnboundedReceiver<()>> {
        None
    }

    pub fn register(_shortcut: Shortcut, _code: u32, _modifiers: u32) -> Result<(), i32> {
        Err(-1)
    }

    pub fn unregister(_shortcut: Shortcut) {}
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

    use super::*;

    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
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
        fn GetEventParameter(
            event: *mut c_void,
            name: u32,
            desired_type: u32,
            actual_type: *mut u32,
            size: usize,
            actual_size: *mut usize,
            out: *mut c_void,
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

    const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
    const PRESSED: u32 = 5;
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    const HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
    const SIGNATURE: u32 = u32::from_be_bytes(*b"BnCd");
    const NOT_HANDLED: i32 = -9874;

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static HOT_KEYS: [AtomicUsize; SHORTCUT_COUNT] =
        [const { AtomicUsize::new(0) }; SHORTCUT_COUNT];

    fn presses() -> &'static Mutex<[Option<UnboundedSender<()>>; SHORTCUT_COUNT]> {
        static PRESSES: OnceLock<Mutex<[Option<UnboundedSender<()>>; SHORTCUT_COUNT]>> =
            OnceLock::new();
        PRESSES.get_or_init(|| Mutex::new(Default::default()))
    }

    extern "C" fn pressed(_next: *mut c_void, event: *mut c_void, _data: *mut c_void) -> i32 {
        let mut id = EventHotKeyId::default();
        // SAFETY: the event is valid in this callback; the parameter's
        // declared type and our C layout are EventHotKeyID.
        let status = unsafe {
            GetEventParameter(
                event,
                DIRECT_OBJECT,
                HOT_KEY_ID,
                std::ptr::null_mut(),
                std::mem::size_of::<EventHotKeyId>(),
                std::ptr::null_mut(),
                (&mut id as *mut EventHotKeyId).cast(),
            )
        };
        if status != 0 || id.signature != SIGNATURE {
            return NOT_HANDLED;
        }
        let Ok(slots) = presses().lock() else {
            return NOT_HANDLED;
        };
        let Some(Some(sender)) = id
            .id
            .checked_sub(1)
            .and_then(|index| slots.get(index as usize))
        else {
            return NOT_HANDLED;
        };
        if sender.send(()).is_err() {
            log::debug!("shortcut pressed after its consumer left");
        }
        0
    }

    /// Start listening on the main thread, before registering the key.
    pub fn listen(shortcut: Shortcut) -> Option<UnboundedReceiver<()>> {
        if !INSTALLED.load(Ordering::Acquire) {
            let spec = EventTypeSpec {
                class: KEYBOARD,
                kind: PRESSED,
            };
            let mut handler = std::ptr::null_mut();
            // SAFETY: this target lives as long as the application. Carbon
            // reads spec during installation and calls pressed on its thread.
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
                log::warn!("could not install global shortcuts: OSStatus {status}");
                return None;
            }
            INSTALLED.store(true, Ordering::Release);
        }
        let mut slots = presses().lock().ok()?;
        let slot = &mut slots[shortcut as usize];
        if slot.as_ref().is_some_and(|sender| !sender.is_closed()) {
            return None;
        }
        let (tx, rx) = unbounded_channel();
        *slot = Some(tx);
        Some(rx)
    }

    pub fn register(shortcut: Shortcut, code: u32, modifiers: u32) -> Result<(), i32> {
        unregister(shortcut);
        let id = EventHotKeyId {
            signature: SIGNATURE,
            id: shortcut as u32 + 1,
        };
        let mut hot_key = std::ptr::null_mut();
        // SAFETY: Carbon writes its managed hot-key ref to this output ref.
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
        HOT_KEYS[shortcut as usize].store(hot_key as usize, Ordering::Release);
        Ok(())
    }

    pub fn unregister(shortcut: Shortcut) {
        let hot_key = HOT_KEYS[shortcut as usize].swap(0, Ordering::AcqRel);
        if hot_key != 0 {
            // SAFETY: the ref register stored, unregistered once.
            let status = unsafe { UnregisterEventHotKey(hot_key as *mut c_void) };
            if status != 0 {
                log::warn!("could not unregister a global shortcut: OSStatus {status}");
            }
        }
    }
}
