//! MonoCode `MonoWorkTicker`: the running turn's one status line. A new
//! status rolls the one before it up and out (340ms), and each holds for
//! 850ms before the next may replace it, so a burst of steps reads as a
//! few calm ticks instead of a flicker. Steps that come in while one holds
//! coalesce into the latest. Pure, so its timing is tested without GPUI;
//! the roll itself is a one-shot transition in the view.

use std::time::{Duration, Instant};

/// MonoCode `TICK_MS`: one roll.
pub const TICK: Duration = Duration::from_millis(340);
/// MonoCode `HOLD_MS`: how long a status stays before the next replaces it.
pub const HOLD: Duration = Duration::from_millis(850);

#[derive(Debug, Default)]
pub struct WorkTicker {
    /// The status shown, and when it took its place.
    current: Option<(String, Instant)>,
    /// The status it replaced, rolling out while the tick lasts.
    previous: Option<String>,
    /// The latest status offered while the current one holds.
    pending: Option<String>,
    /// Bumped by each tick: names the roll's transition.
    sequence: u64,
}

/// What the line draws now.
#[derive(Debug, PartialEq, Eq)]
pub struct TickerFrame<'a> {
    pub current: &'a str,
    /// The status rolling out above it, while the tick lasts.
    pub previous: Option<&'a str>,
    pub sequence: u64,
}

impl WorkTicker {
    /// `status` is what the turn is doing now (`None` once it ended).
    /// Returns how long to wait before [`settle`](Self::settle) may show a
    /// status that has to hold back.
    pub fn offer(&mut self, status: Option<&str>, reduced: bool, now: Instant) -> Option<Duration> {
        let Some(status) = status else {
            *self = Self {
                sequence: self.sequence,
                ..Self::default()
            };
            return None;
        };
        let Some((current, at)) = &self.current else {
            self.current = Some((status.to_string(), now));
            return None;
        };
        if current == status {
            self.pending = None;
            return None;
        }
        let wait = if reduced {
            Duration::ZERO
        } else {
            HOLD.saturating_sub(now.saturating_duration_since(*at))
        };
        if !wait.is_zero() {
            self.pending = Some(status.to_string());
            return Some(wait);
        }
        self.pending = None;
        let replaced = self.current.replace((status.to_string(), now));
        // Reduced motion swaps the line in place.
        self.previous = replaced.map(|(text, _)| text).filter(|_| !reduced);
        self.sequence += 1;
        None
    }

    /// Shows the status that held back, if its time has come; else how
    /// long it still waits.
    pub fn settle(&mut self, reduced: bool, now: Instant) -> Option<Duration> {
        let pending = self.pending.take()?;
        self.offer(Some(&pending), reduced, now)
    }

    pub fn frame(&self, now: Instant) -> Option<TickerFrame<'_>> {
        let (current, at) = self.current.as_ref()?;
        let rolling = now.saturating_duration_since(*at) < TICK;
        Some(TickerFrame {
            current,
            previous: self.previous.as_deref().filter(|_| rolling),
            sequence: self.sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_status_shows_at_once_and_the_next_waits_its_turn() {
        let start = Instant::now();
        let mut ticker = WorkTicker::default();
        assert_eq!(ticker.offer(Some("Thinking…"), false, start), None);
        let frame = ticker.frame(start).unwrap();
        assert_eq!((frame.current, frame.previous), ("Thinking…", None));

        // A step 100ms in holds for the rest of the 850ms.
        let soon = start + Duration::from_millis(100);
        assert_eq!(
            ticker.offer(Some("Reading file…"), false, soon),
            Some(Duration::from_millis(750))
        );
        // Another one coalesces: the latest is what shows.
        let later = start + Duration::from_millis(300);
        assert!(
            ticker
                .offer(Some("Running command…"), false, later)
                .is_some()
        );
        assert_eq!(ticker.frame(later).unwrap().current, "Thinking…");

        let due = start + HOLD;
        assert_eq!(ticker.settle(false, due), None);
        let frame = ticker.frame(due).unwrap();
        assert_eq!(frame.current, "Running command…");
        assert_eq!(frame.previous, Some("Thinking…"));
        assert_eq!(frame.sequence, 1);
        // Once the roll is over only the new line is drawn.
        assert_eq!(ticker.frame(due + TICK).unwrap().previous, None);
    }

    #[test]
    fn the_same_status_again_cancels_one_waiting() {
        let start = Instant::now();
        let mut ticker = WorkTicker::default();
        ticker.offer(Some("Thinking…"), false, start);
        assert!(ticker.offer(Some("Reading file…"), false, start).is_some());
        assert_eq!(ticker.offer(Some("Thinking…"), false, start), None);
        assert_eq!(ticker.settle(false, start + HOLD), None);
        assert_eq!(ticker.frame(start + HOLD).unwrap().current, "Thinking…");
    }

    #[test]
    fn reduced_motion_swaps_in_place_and_an_ended_turn_clears_the_line() {
        let start = Instant::now();
        let mut ticker = WorkTicker::default();
        ticker.offer(Some("Thinking…"), true, start);
        assert_eq!(ticker.offer(Some("Reading file…"), true, start), None);
        let frame = ticker.frame(start).unwrap();
        assert_eq!((frame.current, frame.previous), ("Reading file…", None));
        ticker.offer(None, true, start);
        assert!(ticker.frame(start).is_none());
    }
}
