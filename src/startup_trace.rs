//! Launch timings for perf work (BenCode's own). `BENCODE_STARTUP_TRACE=1`
//! prints each step of a launch with the milliseconds since `main` began,
//! up to the first frame on screen and the loads that fill it in;
//! `BENCODE_STARTUP_TRACE=quit` quits once those have landed, so a script
//! can launch it again and again (`scripts/startup-bench.py`).
//!
//! Like the frame bench, run it on a copy of the data folder (AGENTS.md).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

const ENV: &str = "BENCODE_STARTUP_TRACE";

static START: OnceLock<Instant> = OnceLock::new();

/// Called first thing in `main`.
pub fn init() {
    START.get_or_init(Instant::now);
}

fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| std::env::var(ENV).ok()).as_deref()
}

pub fn enabled() -> bool {
    mode().is_some()
}

fn quits() -> bool {
    mode() == Some("quit")
}

/// Prints `step` with the time since `main` began.
pub fn mark(step: &str) {
    if !enabled() {
        return;
    }
    let ms = START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0;
    println!("startup {ms:>9.2}ms  cpu {:>8.2}ms  {step}", cpu_ms());
}

/// The process's CPU time so far (every thread), which tells a step that
/// works from one that waits.
fn cpu_ms() -> f64 {
    // SAFETY: `getrusage` fills the struct it is given.
    let usage = unsafe {
        let mut usage = std::mem::zeroed::<libc::rusage>();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let ms = |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

/// What a launch waits for before it counts as done: the first frame, the
/// workspace's git snapshot and the open thread's transcript.
pub const FIRST_FRAME: u8 = 1;
pub const WORKSPACE: u8 = 2;
const ALL: u8 = FIRST_FRAME | WORKSPACE;

static LANDED: AtomicU8 = AtomicU8::new(0);

/// Marks `what` (one of the consts above) the first time it lands; once
/// all have, quits in `quit` mode.
pub fn landed(what: u8, step: &str, cx: &mut gpui::App) {
    if !enabled() {
        return;
    }
    let before = LANDED.fetch_or(what, Ordering::Relaxed);
    if before & what != 0 {
        return;
    }
    mark(step);
    if before | what == ALL {
        mark("launch settled");
        if quits() {
            cx.quit();
        }
    }
}
