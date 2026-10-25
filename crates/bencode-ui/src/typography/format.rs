use std::sync::atomic::{AtomicBool, Ordering};

use jiff::{Timestamp, tz::TimeZone};

/// Typographic minus; aligns with plus in tabular figures.
pub const MINUS: char = '\u{2212}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Separators {
    pub group: char,
    pub decimal: char,
}

impl Separators {
    pub const EN: Self = Self {
        group: ',',
        decimal: '.',
    };
}

/// Groups thousands. Non-finite values print as Rust prints them.
pub fn number(value: f64, decimals: usize, separators: Separators) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let fixed = format!("{:.*}", decimals, value.abs());
    let (int, fraction) = match fixed.split_once('.') {
        Some((int, fraction)) => (int, Some(fraction)),
        None => (fixed.as_str(), None),
    };
    let mut out = String::new();
    if value < 0.0 && fixed.chars().any(|ch| ch.is_ascii_digit() && ch != '0') {
        out.push(MINUS);
    }
    out.push_str(&group_digits(int, separators.group));
    if let Some(fraction) = fraction {
        out.push(separators.decimal);
        out.push_str(fraction);
    }
    out
}

/// Inserts `group` every three digits from the right.
pub fn group_digits(digits: &str, group: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(group);
        }
        out.push(digit);
    }
    out
}

/// Decimal units (KB = 1000) or binary (KiB = 1024).
pub fn file_size(bytes: u64, binary: bool) -> String {
    let (base, units) = if binary {
        (1024.0, ["B", "KiB", "MiB", "GiB", "TiB", "PiB"])
    } else {
        (1000.0, ["B", "KB", "MB", "GB", "TB", "PB"])
    };
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= base && unit < units.len() - 1 {
        value /= base;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let decimals = if value < 10.0 { 1 } else { 0 };
    let shown = format!("{value:.decimals$}");
    if shown.parse::<f64>().is_ok_and(|rounded| rounded >= base) && unit < units.len() - 1 {
        return format!("1.0 {}", units[unit + 1]);
    }
    format!("{shown} {}", units[unit])
}

/// "1 file", "12,345 files".
pub fn plural(count: u64, one: &str, other: &str) -> String {
    let word = if count == 1 { one } else { other };
    format!(
        "{} {word}",
        group_digits(&count.to_string(), Separators::EN.group)
    )
}

/// "just now", "3 minutes ago", "in 2 days".
pub fn relative(then: Timestamp, now: Timestamp) -> String {
    let Some((count, unit, future)) = span(then, now) else {
        return "just now".to_string();
    };
    let label = plural(count, unit, &format!("{unit}s"));
    if future {
        format!("in {label}")
    } else {
        format!("{label} ago")
    }
}

/// The rounded distance: count, unit, and whether ahead; none under 45 s.
pub(crate) fn span(then: Timestamp, now: Timestamp) -> Option<(u64, &'static str, bool)> {
    let delta = now.as_second() - then.as_second();
    let seconds = delta.unsigned_abs();
    if seconds < 45 {
        return None;
    }
    let round = |unit: u64| (seconds + unit / 2) / unit;
    let (count, unit) = match seconds {
        s if s < 90 => (1, "minute"),
        s if s < 45 * 60 => (round(60), "minute"),
        s if s < 90 * 60 => (1, "hour"),
        s if s < 22 * 3_600 => (round(3_600), "hour"),
        s if s < 36 * 3_600 => (1, "day"),
        s if s < 26 * 86_400 => (round(86_400), "day"),
        s if s < 45 * 86_400 => (1, "month"),
        s if s < 320 * 86_400 => (round(30 * 86_400), "month"),
        s if s < 548 * 86_400 => (1, "year"),
        _ => (round(365 * 86_400), "year"),
    };
    Some((count, unit, delta < 0))
}

/// The system's zone; an unknown one logs once, naming `user`, and reads as jiff's unknown zone.
pub(crate) fn system_zone(user: &str) -> TimeZone {
    TimeZone::try_system().unwrap_or_else(|error| {
        static TOLD: AtomicBool = AtomicBool::new(false);
        if !TOLD.swap(true, Ordering::Relaxed) {
            log::error!("{user}: the system time zone is unknown ({error}); times show in UTC");
        }
        TimeZone::unknown()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_groups_and_rounds() {
        assert_eq!(number(1234567.891, 2, Separators::EN), "1,234,567.89");
        assert_eq!(number(999.5, 0, Separators::EN), "1,000");
        assert_eq!(number(12.0, 0, Separators::EN), "12");
        assert_eq!(number(-1234.5, 1, Separators::EN), "−1,234.5");
        assert_eq!(number(-0.001, 2, Separators::EN), "0.00");
        let de = Separators {
            group: '.',
            decimal: ',',
        };
        assert_eq!(number(1234.5, 2, de), "1.234,50");
        assert_eq!(number(f64::NAN, 2, Separators::EN), "NaN");
    }

    #[test]
    fn file_size_picks_units_and_carries() {
        assert_eq!(file_size(512, false), "512 B");
        assert_eq!(file_size(1536, false), "1.5 KB");
        assert_eq!(file_size(15_360, false), "15 KB");
        assert_eq!(file_size(999_999, false), "1.0 MB");
        assert_eq!(file_size(1536, true), "1.5 KiB");
        assert_eq!(file_size(3 * 1024 * 1024 * 1024, true), "3.0 GiB");
    }

    #[test]
    fn plural_counts() {
        assert_eq!(plural(1, "file", "files"), "1 file");
        assert_eq!(plural(0, "file", "files"), "0 files");
        assert_eq!(plural(12_345, "file", "files"), "12,345 files");
        let huge = plural(9_007_199_254_740_993, "file", "files");
        assert_eq!(huge, "9,007,199,254,740,993 files");
    }

    #[test]
    fn relative_reads_both_directions() {
        let now = Timestamp::from_second(1_000_000_000).unwrap();
        let ago = |s: i64| relative(Timestamp::from_second(1_000_000_000 - s).unwrap(), now);
        assert_eq!(ago(10), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(180), "3 minutes ago");
        assert_eq!(ago(7_200), "2 hours ago");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(ago(3 * 365 * 86_400), "3 years ago");
        assert_eq!(ago(-120), "in 2 minutes");
    }
}
