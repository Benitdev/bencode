//! A site's HTTP sign-in (Basic, Digest, NTLM; a dev server behind
//! CloudFront's password, say). WebKit asks the navigation delegate for
//! credentials, and wry's delegate has no answer, so WebKit went on without
//! any and showed the server's empty 401 as a white page. A method added to
//! wry's delegate class takes the challenge and holds it while the app asks
//! the user (`app/browser_sign_in.rs`); the answer is kept by WebKit for
//! the session only, never written anywhere.

#![allow(unexpected_cfgs)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

use block::Block;
use objc::runtime::{BOOL, Class, NO, Object, Sel, class_addMethod};
use objc::{class, msg_send, sel, sel_impl};
use wry::WebViewExtMacOS;

use super::page::SignIn as Challenge;

type Id = *mut Object;
type Handler = Block<(isize, Id), ()>;

/// `NSURLSessionAuthChallengeDisposition`.
const USE_CREDENTIAL: isize = 0;
const DEFAULT_HANDLING: isize = 1;
/// `NSURLCredentialPersistenceForSession`.
const FOR_SESSION: usize = 1;
/// `NSUTF8StringEncoding`.
const UTF8: usize = 4;

/// The methods a person answers; server trust and client certificates
/// stay WebKit's.
const ASKED: [&str; 4] = [
    "NSURLAuthenticationMethodDefault",
    "NSURLAuthenticationMethodHTTPBasic",
    "NSURLAuthenticationMethodHTTPDigest",
    "NSURLAuthenticationMethodNTLM",
];

/// A challenge WebKit waits on: both retained until answered.
struct Waiting {
    challenge: Id,
    handler: Id,
    host: String,
}

struct Watched {
    ask: Box<dyn Fn(Challenge)>,
    waiting: Vec<Waiting>,
}

thread_local! {
    /// By the WKWebView's address; WebKit calls the delegate on the main
    /// thread, where pages are built.
    static VIEWS: RefCell<HashMap<usize, Watched>> = RefCell::new(HashMap::new());
}

fn key(view: &wry::WebView) -> (usize, Id) {
    let webview = view.webview();
    let webview: Id = &*webview as *const _ as Id;
    (webview as usize, webview)
}

/// Sends the page's sign-in requests to `ask`, until `forget`.
pub fn watch(view: &wry::WebView, ask: impl Fn(Challenge) + 'static) {
    let (key, webview) = key(view);
    unsafe {
        let delegate: Id = msg_send![webview, navigationDelegate];
        if delegate.is_null() {
            return log::warn!("browser: no navigation delegate, sign-ins stay blank");
        }
        let selector = sel!(webView:didReceiveAuthenticationChallenge:completionHandler:);
        let answers: BOOL = msg_send![delegate, respondsToSelector: selector];
        if answers == NO {
            let class: *const Class = msg_send![delegate, class];
            let imp: extern "C" fn(&Object, Sel, Id, Id, Id) = challenged;
            let added = class_addMethod(
                class as *mut Class,
                selector,
                std::mem::transmute::<extern "C" fn(&Object, Sel, Id, Id, Id), objc::runtime::Imp>(
                    imp,
                ),
                c"v@:@@@?".as_ptr(),
            );
            if added == NO {
                return log::warn!("browser: could not take sign-in requests");
            }
        }
        // WebKit reads which methods the delegate has when it is set.
        let _: () = msg_send![webview, setNavigationDelegate: delegate];
    }
    VIEWS.with_borrow_mut(|views| {
        views.insert(
            key,
            Watched {
                ask: Box::new(ask),
                waiting: Vec::new(),
            },
        )
    });
}

/// Answers what the page still waits on with nothing (the tab is going).
pub fn forget(view: &wry::WebView) {
    let (key, _) = key(view);
    let Some(watched) = VIEWS.with_borrow_mut(|views| views.remove(&key)) else {
        return;
    };
    for waiting in watched.waiting {
        unsafe { finish(waiting, None) };
    }
}

/// Answers the challenge the user was asked about, and every other one for
/// the same site; `None` goes on without (the server's own 401 page). A
/// challenge for another site is asked next.
pub fn answer(view: &wry::WebView, credential: Option<(&str, &str)>) {
    let (key, _) = key(view);
    let (answered, next) = VIEWS.with_borrow_mut(|views| {
        let Some(watched) = views.get_mut(&key) else {
            return (Vec::new(), None);
        };
        let Some(host) = watched.waiting.first().map(|w| w.host.clone()) else {
            return (Vec::new(), None);
        };
        let (answered, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut watched.waiting)
            .into_iter()
            .partition(|w| w.host == host);
        watched.waiting = rest;
        let next = watched
            .waiting
            .first()
            .map(|w| unsafe { describe(w.challenge) });
        (answered, next)
    });
    for waiting in answered {
        unsafe { finish(waiting, credential) };
    }
    if let Some(next) = next {
        VIEWS.with_borrow(|views| {
            if let Some(watched) = views.get(&key) {
                (watched.ask)(next);
            }
        });
    }
}

/// `webView:didReceiveAuthenticationChallenge:completionHandler:`.
extern "C" fn challenged(_: &Object, _: Sel, webview: Id, challenge: Id, handler: Id) {
    unsafe {
        let space: Id = msg_send![challenge, protectionSpace];
        let method = string(msg_send![space, authenticationMethod]);
        let watched = VIEWS.with_borrow(|views| views.contains_key(&(webview as usize)));
        if !watched || !ASKED.contains(&method.as_str()) {
            return call(handler, DEFAULT_HANDLING, std::ptr::null_mut());
        }
        // An address's `user:pass@`, or what this session already gave.
        let failures: isize = msg_send![challenge, previousFailureCount];
        let proposed: Id = msg_send![challenge, proposedCredential];
        if failures == 0 && !proposed.is_null() {
            let has_password: BOOL = msg_send![proposed, hasPassword];
            if has_password != NO {
                return call(handler, USE_CREDENTIAL, proposed);
            }
        }
        let asked = describe(challenge);
        let waiting = Waiting {
            challenge: msg_send![challenge, retain],
            handler: msg_send![handler, copy],
            host: asked.host.clone(),
        };
        let first = VIEWS.with_borrow_mut(|views| {
            let watched = views.get_mut(&(webview as usize))?;
            let first = watched.waiting.is_empty();
            watched.waiting.push(waiting);
            Some(first)
        });
        if first == Some(true) {
            log::info!("browser: {} asks for a sign-in", asked.host);
            VIEWS.with_borrow(|views| {
                if let Some(watched) = views.get(&(webview as usize)) {
                    (watched.ask)(asked);
                }
            });
        }
    }
}

unsafe fn describe(challenge: Id) -> Challenge {
    unsafe {
        let space: Id = msg_send![challenge, protectionSpace];
        let host = string(msg_send![space, host]);
        let port: isize = msg_send![space, port];
        let protocol = string(msg_send![space, protocol]);
        let secure: BOOL = msg_send![space, receivesCredentialSecurely];
        let failures: isize = msg_send![challenge, previousFailureCount];
        let usual = matches!(
            (protocol.as_str(), port),
            ("https", 443) | ("http", 80) | (_, 0)
        );
        Challenge {
            host: if usual {
                host
            } else {
                format!("{host}:{port}")
            },
            insecure: secure == NO,
            retry: failures > 0,
        }
    }
}

/// Hands WebKit its answer and lets go of the challenge.
unsafe fn finish(waiting: Waiting, credential: Option<(&str, &str)>) {
    unsafe {
        match credential {
            Some((user, password)) => {
                let (user, password) = (ns_string(user), ns_string(password));
                let credential: Id = msg_send![class!(NSURLCredential),
                    credentialWithUser: user password: password persistence: FOR_SESSION];
                call(waiting.handler, USE_CREDENTIAL, credential);
                let _: () = msg_send![password, release];
                let _: () = msg_send![user, release];
            }
            None => call(waiting.handler, DEFAULT_HANDLING, std::ptr::null_mut()),
        }
        let _: () = msg_send![waiting.handler, release];
        let _: () = msg_send![waiting.challenge, release];
    }
}

unsafe fn call(handler: Id, disposition: isize, credential: Id) {
    let handler = unsafe { &*(handler as *const Handler) };
    unsafe { handler.call((disposition, credential)) };
}

/// An owned `NSString` (released by the caller).
unsafe fn ns_string(text: &str) -> Id {
    unsafe {
        let string: Id = msg_send![class!(NSString), alloc];
        msg_send![string, initWithBytes: text.as_ptr() length: text.len() encoding: UTF8]
    }
}

unsafe fn string(ns: Id) -> String {
    if ns.is_null() {
        return String::new();
    }
    unsafe {
        let utf8: *const std::ffi::c_char = msg_send![ns, UTF8String];
        if utf8.is_null() {
            return String::new();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }
}
