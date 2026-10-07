//! BenCode's own CPU time and memory, for the footer's readout. No MonoCode
//! counterpart: the footer shows it so the app's footprint stays in sight.

use std::time::Duration;

/// One reading of this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSample {
    /// User plus system CPU time since the process started.
    pub cpu_time: Duration,
    /// What Activity Monitor calls Memory (the physical footprint on macOS,
    /// the resident set elsewhere); `None` where it cannot be read.
    pub memory_bytes: Option<u64>,
}

/// Reads this process's CPU time and memory; a few syscalls, no IO on macOS.
pub fn sample() -> Option<ProcessSample> {
    Some(ProcessSample {
        cpu_time: cpu_time()?,
        memory_bytes: memory_bytes(),
    })
}

/// CPU used between two samples as a percent of one core, like Activity
/// Monitor (a busy multi-threaded process passes 100).
pub fn cpu_percent(previous: Duration, current: Duration, elapsed: Duration) -> f32 {
    if elapsed.is_zero() {
        return 0.0;
    }
    let used = current.saturating_sub(previous);
    (used.as_secs_f64() / elapsed.as_secs_f64() * 100.0) as f32
}

/// `3%`, `0.4%`, `112%`: one decimal only below 10, so the text barely moves.
pub fn format_percent(percent: f32) -> String {
    if percent < 10.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// `48 MB`, `1.2 GB`, in the binary units Activity Monitor uses.
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else {
        format!("{:.0} MB", bytes / MB)
    }
}

#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    // SAFETY: `getrusage` fills the zeroed struct it is given.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        log::debug!("getrusage: {}", std::io::Error::last_os_error());
        return None;
    }
    let time = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1_000);
    Some(time(usage.ru_utime) + time(usage.ru_stime))
}

#[cfg(not(unix))]
fn cpu_time() -> Option<Duration> {
    None
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> Option<u64> {
    // SAFETY: `proc_pid_rusage` fills the zeroed `rusage_info_v2` for the
    // flavor it is asked for.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        )
    };
    if status != 0 {
        log::debug!("proc_pid_rusage: {}", std::io::Error::last_os_error());
        return None;
    }
    Some(info.ri_phys_footprint)
}

#[cfg(target_os = "linux")]
fn memory_bytes() -> Option<u64> {
    // `statm`'s second field is the resident set, in pages.
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: `sysconf` has no preconditions.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * u64::try_from(page_size).ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_is_relative_to_one_core() {
        let ms = Duration::from_millis;
        assert_eq!(cpu_percent(ms(100), ms(600), ms(1000)), 50.0);
        assert_eq!(cpu_percent(ms(0), ms(3000), ms(2000)), 150.0);
        assert_eq!(cpu_percent(ms(500), ms(400), ms(1000)), 0.0);
        assert_eq!(cpu_percent(ms(0), ms(10), Duration::ZERO), 0.0);
    }

    #[test]
    fn formats_percent_and_bytes() {
        assert_eq!(format_percent(0.04), "0.0%");
        assert_eq!(format_percent(3.26), "3.3%");
        assert_eq!(format_percent(12.6), "13%");
        assert_eq!(format_bytes(48 * 1024 * 1024), "48 MB");
        assert_eq!(format_bytes(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn samples_this_process() {
        let sample = sample().expect("sample");
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(sample.memory_bytes.is_some_and(|bytes| bytes > 0));
        }
    }
}
