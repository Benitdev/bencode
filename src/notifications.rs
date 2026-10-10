//! MonoCode `notifications.rs`: desktop notifications through
//! `UNUserNotificationCenter`, which reports the real permission and hands a
//! click back through its delegate. The center only exists for a bundled
//! app: run from `cargo run`, BenCode has no bundle id and notifications
//! are `Unsupported`. Blocking calls; run them on a background executor.

use std::time::Duration;

/// MonoCode `NotificationPermission`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permission {
    /// Never asked, or not answered yet.
    #[default]
    Prompt,
    Granted,
    /// Declined at the prompt, or alerts switched off in System Settings.
    Denied,
    /// No notification center (not running as BenCode.app).
    Unsupported,
}

/// What a click on a notification asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    Session(String),
    Reminder { session_id: String, due_at: i64 },
}

const SESSION_PREFIX: &str = "session:";
const REMINDER_PREFIX: &str = "reminder:";

/// A thread's notification id. Each gets a fresh suffix: reusing one
/// replaces the previous banner, and macOS drops replacements that land
/// while the app is frontmost.
pub fn session_identifier(session_id: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{SESSION_PREFIX}{session_id}/{nanos}")
}

/// MonoCode's reminder id: `reminder:<session>:<due>`.
pub fn reminder_identifier(session_id: &str, due_at: i64) -> String {
    format!("{REMINDER_PREFIX}{session_id}:{due_at}")
}

pub fn parse_click(identifier: &str) -> Option<Click> {
    if let Some(rest) = identifier.strip_prefix(SESSION_PREFIX) {
        let id = rest.split('/').next().unwrap_or(rest);
        return (!id.is_empty()).then(|| Click::Session(id.to_string()));
    }
    let (session_id, due_at) = identifier.strip_prefix(REMINDER_PREFIX)?.rsplit_once(':')?;
    Some(Click::Reminder {
        session_id: session_id.to_string(),
        due_at: due_at.parse().ok()?,
    })
}

/// A dispatch still unanswered by now has lost to the in-app cue.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

pub use platform::{install_delegate, open_settings, permission, request_permission, show};

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Permission;

    pub fn install_delegate() -> Option<tokio::sync::mpsc::UnboundedReceiver<String>> {
        None
    }

    pub fn permission() -> Permission {
        Permission::Unsupported
    }

    pub fn request_permission() -> Permission {
        Permission::Unsupported
    }

    pub fn show(_: &str, _: &str, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
        anyhow::bail!("notifications are not supported on this platform")
    }

    pub fn open_settings() -> anyhow::Result<()> {
        anyhow::bail!("notifications are not supported on this platform")
    }
}

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod platform {
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};

    use anyhow::{Result, bail};
    use block::{Block, ConcreteBlock};
    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, Object, Protocol, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::{DISPATCH_TIMEOUT, Permission};

    #[link(name = "UserNotifications", kind = "framework")]
    unsafe extern "C" {}

    type Id = *mut Object;

    /// A banner category with one "Show" button, so the jump back is offered
    /// explicitly rather than only on a click on the body.
    const CATEGORY: &str = "bencode.session";
    const SHOW_ACTION: &str = "bencode.session.show";

    /// `UNAuthorizationOptionBadge | Sound | Alert`.
    const AUTHORIZATION_OPTIONS: usize = 1 | 2 | 4;
    /// `UNNotificationPresentationOptionSound | List | Banner`.
    const PRESENT_OPTIONS: usize = 2 | 8 | 16;
    /// `UNNotificationActionOptionForeground`.
    const ACTION_FOREGROUND: usize = 4;

    /// Runs `f` inside an autorelease pool: the calls below may run on a
    /// background thread, which has none of its own.
    fn pooled<T>(f: impl FnOnce() -> T) -> T {
        unsafe {
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            let out = f();
            let _: () = msg_send![pool, drain];
            out
        }
    }

    fn ns_string(text: &str) -> Id {
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    fn rust_string(string: Id) -> String {
        if string.is_null() {
            return String::new();
        }
        unsafe {
            let utf8: *const c_char = msg_send![string, UTF8String];
            if utf8.is_null() {
                return String::new();
            }
            CStr::from_ptr(utf8).to_string_lossy().into_owned()
        }
    }

    /// The center exists only for an app with a bundle id; asking for it
    /// anywhere else raises an Objective-C exception.
    fn supported() -> bool {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        *SUPPORTED.get_or_init(|| {
            pooled(|| unsafe {
                let bundle: Id = msg_send![class!(NSBundle), mainBundle];
                let id: Id = msg_send![bundle, bundleIdentifier];
                let path: Id = msg_send![bundle, bundlePath];
                !id.is_null() && rust_string(path).ends_with(".app")
            })
        })
    }

    fn center() -> Id {
        unsafe { msg_send![class!(UNUserNotificationCenter), currentNotificationCenter] }
    }

    fn map_permission(
        authorization: isize,
        alert_setting: isize,
        alert_style: isize,
    ) -> Permission {
        match authorization {
            // UNAuthorizationStatusNotDetermined, …Denied.
            0 => Permission::Prompt,
            1 => Permission::Denied,
            // Authorized does not promise a visible alert: alerts can be on
            // with no style picked (UNNotificationSettingDisabled = 1,
            // UNAlertStyleNone = 0).
            _ if alert_setting == 1 || alert_style == 0 => Permission::Denied,
            _ => Permission::Granted,
        }
    }

    /// `None` waits for the user, as the permission prompt needs.
    fn query_permission(timeout: Option<std::time::Duration>) -> Permission {
        if !supported() {
            return Permission::Unsupported;
        }
        let (tx, rx) = mpsc::channel();
        pooled(|| {
            let handler = ConcreteBlock::new(move |settings: Id| {
                let permission = unsafe {
                    let status: isize = msg_send![settings, authorizationStatus];
                    let alert: isize = msg_send![settings, alertSetting];
                    let style: isize = msg_send![settings, alertStyle];
                    map_permission(status, alert, style)
                };
                if tx.send(permission).is_err() {
                    log::debug!("notification permission answered after its caller left");
                }
            })
            .copy();
            unsafe {
                let _: () =
                    msg_send![center(), getNotificationSettingsWithCompletionHandler: &*handler];
            }
        });
        let answer = match timeout {
            Some(timeout) => rx.recv_timeout(timeout).ok(),
            None => rx.recv().ok(),
        };
        answer.unwrap_or(Permission::Denied)
    }

    pub fn permission() -> Permission {
        query_permission(None)
    }

    /// Shows the macOS prompt when undecided; otherwise reports the state.
    pub fn request_permission() -> Permission {
        if !supported() {
            return Permission::Unsupported;
        }
        let (tx, rx) = mpsc::channel();
        pooled(|| {
            let handler = ConcreteBlock::new(move |_granted: BOOL, _error: Id| {
                if tx.send(()).is_err() {
                    log::debug!("notification prompt answered after its caller left");
                }
            })
            .copy();
            unsafe {
                let _: () = msg_send![center(),
                    requestAuthorizationWithOptions: AUTHORIZATION_OPTIONS
                    completionHandler: &*handler];
            }
        });
        if rx.recv().is_err() {
            log::debug!("notification prompt dropped its handler");
        }
        permission()
    }

    /// Sends one banner and waits for the center to accept it. Asks for the
    /// permission first: one switched off in System Settings is refused
    /// here, so the caller's in-app cue stands in.
    pub fn show(
        identifier: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        sound: bool,
    ) -> Result<()> {
        match query_permission(Some(DISPATCH_TIMEOUT)) {
            Permission::Granted => {}
            Permission::Unsupported => bail!("notifications need BenCode.app"),
            _ => bail!("notifications are not authorized"),
        }
        let (tx, rx) = mpsc::channel();
        pooled(|| unsafe {
            let content: Id = msg_send![class!(UNMutableNotificationContent), new];
            let _: () = msg_send![content, setTitle: ns_string(title)];
            let _: () = msg_send![content, setSubtitle: ns_string(subtitle)];
            let _: () = msg_send![content, setBody: ns_string(body)];
            let _: () = msg_send![content, setCategoryIdentifier: ns_string(CATEGORY)];
            if sound {
                let default: Id = msg_send![class!(UNNotificationSound), defaultSound];
                let _: () = msg_send![content, setSound: default];
            }
            let request: Id = msg_send![class!(UNNotificationRequest),
                requestWithIdentifier: ns_string(identifier)
                content: content
                trigger: std::ptr::null_mut::<Object>()];
            let _: () = msg_send![content, release];
            let handler = ConcreteBlock::new(move |error: Id| {
                let result = if error.is_null() {
                    Ok(())
                } else {
                    let description: Id = msg_send![error, localizedDescription];
                    Err(rust_string(description))
                };
                if tx.send(result).is_err() {
                    log::debug!("notification answered after its caller left");
                }
            })
            .copy();
            let _: () = msg_send![center(), addNotificationRequest: request withCompletionHandler: &*handler];
        });
        match rx.recv_timeout(DISPATCH_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(reason)) => bail!("notification rejected: {reason}"),
            Err(_) => bail!("notification dispatch timed out"),
        }
    }

    /// BenCode's page in System Settings › Notifications.
    pub fn open_settings() -> Result<()> {
        let id = pooled(|| unsafe {
            let bundle: Id = msg_send![class!(NSBundle), mainBundle];
            rust_string(msg_send![bundle, bundleIdentifier])
        });
        let url =
            format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}");
        std::process::Command::new("open").arg(url).spawn()?;
        Ok(())
    }

    /// Where the delegate sends a clicked notification's identifier.
    fn clicks() -> &'static Mutex<Option<UnboundedSender<String>>> {
        static CLICKS: OnceLock<Mutex<Option<UnboundedSender<String>>>> = OnceLock::new();
        CLICKS.get_or_init(|| Mutex::new(None))
    }

    /// Without this macOS drops banners while the app is frontmost, and a
    /// finished thread in another pane deserves one either way.
    extern "C" fn will_present(
        _: &Object,
        _: Sel,
        _center: Id,
        _notification: Id,
        completion: *mut c_void,
    ) {
        let completion = completion as *mut Block<(usize,), ()>;
        unsafe { (*completion).call((PRESENT_OPTIONS,)) };
    }

    extern "C" fn did_receive(
        _: &Object,
        _: Sel,
        _center: Id,
        response: Id,
        completion: *mut c_void,
    ) {
        let identifier = unsafe {
            let notification: Id = msg_send![response, notification];
            let request: Id = msg_send![notification, request];
            rust_string(msg_send![request, identifier])
        };
        match clicks().lock() {
            Ok(sender) => {
                if let Some(sender) = sender.as_ref()
                    && sender.send(identifier).is_err()
                {
                    log::debug!("notification clicked after the app left");
                }
            }
            Err(err) => log::warn!("notification click: {err}"),
        }
        let completion = completion as *mut Block<(), ()>;
        unsafe { (*completion).call(()) };
    }

    fn delegate_class() -> &'static Class {
        static CLASS: OnceLock<usize> = OnceLock::new();
        let class = *CLASS.get_or_init(|| {
            let mut decl = ClassDecl::new("BenCodeNotificationDelegate", class!(NSObject))
                .expect("the delegate class is declared once");
            if let Some(protocol) = Protocol::get("UNUserNotificationCenterDelegate") {
                decl.add_protocol(protocol);
            }
            unsafe {
                decl.add_method(
                    sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                    will_present as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
                );
                decl.add_method(
                    sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                    did_receive as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
                );
            }
            decl.register() as *const Class as usize
        });
        unsafe { &*(class as *const Class) }
    }

    /// Becomes the center's delegate (it keeps a weak reference, so the
    /// delegate lives for the app) and registers the "Show" category. Call
    /// once, on the main thread. Clicks arrive on the returned channel.
    pub fn install_delegate() -> Option<UnboundedReceiver<String>> {
        if !supported() {
            return None;
        }
        let (tx, rx) = unbounded_channel();
        match clicks().lock() {
            Ok(mut slot) => *slot = Some(tx),
            Err(err) => {
                log::warn!("notification clicks: {err}");
                return None;
            }
        }
        pooled(|| unsafe {
            let delegate: Id = msg_send![delegate_class(), new];
            let center = center();
            let _: () = msg_send![center, setDelegate: delegate];
            let show: Id = msg_send![class!(UNNotificationAction),
                actionWithIdentifier: ns_string(SHOW_ACTION)
                title: ns_string("Show")
                options: ACTION_FOREGROUND];
            let actions: Id = msg_send![class!(NSArray), arrayWithObject: show];
            let empty: Id = msg_send![class!(NSArray), array];
            let category: Id = msg_send![class!(UNNotificationCategory),
                categoryWithIdentifier: ns_string(CATEGORY)
                actions: actions
                intentIdentifiers: empty
                options: 0usize];
            let categories: Id = msg_send![class!(NSSet), setWithObject: category];
            let _: () = msg_send![center, setNotificationCategories: categories];
        });
        Some(rx)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn permission_requires_a_visible_alert_style() {
            // Authorized, alerts enabled, no style.
            assert_eq!(map_permission(2, 2, 0), Permission::Denied);
            assert_eq!(map_permission(2, 2, 1), Permission::Granted);
            assert_eq!(map_permission(2, 1, 1), Permission::Denied);
            assert_eq!(map_permission(0, 0, 0), Permission::Prompt);
            assert_eq!(map_permission(1, 2, 1), Permission::Denied);
        }

        #[test]
        fn a_test_binary_has_no_notification_center() {
            assert!(!supported());
            assert_eq!(permission(), Permission::Unsupported);
            assert!(install_delegate().is_none());
        }

        #[test]
        fn the_delegate_answers_both_center_callbacks() {
            let class = delegate_class();
            assert!(
                class
                    .instance_method(
                        sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:)
                    )
                    .is_some()
            );
            assert!(
                class
                    .instance_method(sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))
                    .is_some()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_identifier_round_trips() {
        let id = session_identifier("549ae7ac");
        assert_eq!(parse_click(&id), Some(Click::Session("549ae7ac".into())));
    }

    #[test]
    fn a_reminder_identifier_round_trips() {
        let id = reminder_identifier("549ae7ac", 1_700_000_000_000);
        assert_eq!(
            parse_click(&id),
            Some(Click::Reminder {
                session_id: "549ae7ac".into(),
                due_at: 1_700_000_000_000
            })
        );
        assert_eq!(parse_click("other"), None);
        assert_eq!(parse_click("reminder:x:soon"), None);
    }
}
