//! The footer's CPU and memory readout: BenCode's own process, sampled every
//! few seconds off the UI thread. The sampling loop and the drawing live in
//! `ui/footer/process_layer.rs`.

use std::time::{Duration, Instant};

use crate::process_stats::{self, ProcessSample};

/// How often the readout is sampled.
pub(crate) const SAMPLE_EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct ProcessMonitor {
    last: Option<(Instant, ProcessSample)>,
    /// `None` until two samples give a rate.
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

impl ProcessMonitor {
    /// Folds a sample in; true when the text on screen changed.
    pub(crate) fn record(&mut self, at: Instant, sample: ProcessSample) -> bool {
        let cpu = self.last.map(|(then, previous)| {
            let percent = process_stats::cpu_percent(
                previous.cpu_time,
                sample.cpu_time,
                at.duration_since(then),
            );
            process_stats::format_percent(percent)
        });
        let memory = sample.memory_bytes.map(process_stats::format_bytes);
        self.last = Some((at, sample));
        let changed = cpu != self.cpu || memory != self.memory;
        self.cpu = cpu;
        self.memory = memory;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_needs_two_samples() {
        let mut monitor = ProcessMonitor::default();
        let start = Instant::now();
        let sample = |cpu_ms, mb: u64| ProcessSample {
            cpu_time: Duration::from_millis(cpu_ms),
            memory_bytes: Some(mb * 1024 * 1024),
        };
        assert!(monitor.record(start, sample(100, 40)));
        assert_eq!(monitor.cpu, None);
        assert_eq!(monitor.memory.as_deref(), Some("40 MB"));

        assert!(monitor.record(start + Duration::from_secs(2), sample(200, 40)));
        assert_eq!(monitor.cpu.as_deref(), Some("5.0%"));

        assert!(!monitor.record(start + Duration::from_secs(4), sample(300, 40)));
    }
}
