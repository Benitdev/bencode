//! Gives GPU memory back once BenCode goes quiet (MonoCode `gpu_trim.rs`).
//! The sprite atlases keep every glyph, icon and image any frame drew, and
//! the renderers the buffers of their busiest frame, so without this the
//! footprint only grows over a long session. A trim empties them and
//! redraws every window once: only what is on screen comes back. It runs
//! 10s after every window went to the background (the close button hides
//! BenCode), and again every 5 minutes while it stays there. Never in a
//! window in use: the first frame after a trim rasterizes what it shows
//! again, tens of milliseconds on a long thread and more now and then
//! (the frame bench's "repainted after a GPU trim"), a hitch the person
//! would see.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::App;

const CHECK: Duration = Duration::from_secs(5);
const BACKGROUND_AFTER: Duration = Duration::from_secs(10);
const AGAIN_AFTER: Duration = Duration::from_secs(300);

/// Watches the keyboard and the windows' focus; started once, after the
/// first frame.
pub fn start(cx: &mut App) {
    let typed = Rc::new(Cell::new(Instant::now()));
    cx.observe_keystrokes({
        let typed = typed.clone();
        move |_, _, _| typed.set(Instant::now())
    })
    .detach();
    cx.spawn(async move |cx| {
        let mut clock = Quiet::new(Instant::now());
        loop {
            cx.background_executor().timer(CHECK).await;
            let active = cx.update(|cx| {
                cx.windows().into_iter().any(|handle| {
                    handle
                        .update(cx, |_, window, _| window.is_window_active())
                        .unwrap_or(false)
                })
            });
            if clock.due(Instant::now(), active, typed.get()) {
                cx.update(trim);
            }
        }
    })
    .detach();
}

fn trim(cx: &mut App) {
    log::info!("gpu: quiet, giving back the atlases and buffers");
    #[cfg(target_os = "macos")]
    gpui_apple::metal_renderer::trim();
    // Each window paints afresh: no view replays a paint naming the
    // atlases' old tiles.
    cx.refresh_windows();
}

/// When a trim is due, from the windows' focus and the last key.
struct Quiet {
    /// The last time a window became active.
    activated: Instant,
    inactive_since: Option<Instant>,
    trimmed: Option<Instant>,
}

impl Quiet {
    fn new(now: Instant) -> Self {
        Self {
            activated: now,
            inactive_since: None,
            trimmed: None,
        }
    }

    fn due(&mut self, now: Instant, active: bool, typed: Instant) -> bool {
        if active {
            if self.inactive_since.take().is_some() {
                self.activated = now;
            }
        } else {
            self.inactive_since.get_or_insert(now);
        }
        let activity = typed.max(self.activated);
        let quiet = self
            .inactive_since
            .is_some_and(|since| now.duration_since(since.max(activity)) >= BACKGROUND_AFTER);
        let again = self
            .trimmed
            .is_none_or(|at| at < activity || now.duration_since(at) >= AGAIN_AFTER);
        if quiet && again {
            self.trimmed = Some(now);
        }
        quiet && again
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    #[test]
    fn a_background_app_trims_after_ten_seconds_then_once_until_used() {
        let start = Instant::now();
        let mut quiet = Quiet::new(start);
        assert!(!quiet.due(start + 5 * SEC, false, start));
        assert!(quiet.due(start + 15 * SEC, false, start));
        // Nothing new happened: not again until five minutes on.
        assert!(!quiet.due(start + 20 * SEC, false, start));
        assert!(quiet.due(start + 15 * SEC + AGAIN_AFTER, false, start));
    }

    #[test]
    fn a_window_in_use_never_trims() {
        let start = Instant::now();
        let mut quiet = Quiet::new(start);
        for minutes in [1, 5, 30] {
            assert!(!quiet.due(start + minutes * 60 * SEC, true, start));
        }
    }

    #[test]
    fn coming_back_to_the_app_counts_as_activity() {
        let start = Instant::now();
        let mut quiet = Quiet::new(start);
        // The check that sees it leave starts the count.
        assert!(!quiet.due(start + 5 * SEC, false, start));
        assert!(quiet.due(start + 15 * SEC, false, start));
        // Back for a moment, then away again: ten seconds from leaving.
        assert!(!quiet.due(start + 20 * SEC, true, start));
        assert!(!quiet.due(start + 25 * SEC, false, start));
        assert!(quiet.due(start + 40 * SEC, false, start));
    }
}
