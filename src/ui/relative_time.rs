//! MonoCode `formatRelativeTime` (`Intl.RelativeTimeFormat`, `numeric:
//! "auto"`), in English: "now", "5 minutes ago", "yesterday", "2 weeks ago".

/// How long before `now_ms` the time `then_ms` was, both in epoch ms.
pub fn since(then_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - then_ms) / 1000;
    let ago = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match secs {
        i64::MIN..60 => "now".to_string(),
        60..3600 => ago(secs / 60, "minute"),
        3600..86_400 => ago(secs / 3600, "hour"),
        86_400..172_800 => "yesterday".to_string(),
        172_800..604_800 => ago(secs / 86_400, "day"),
        604_800..2_629_800 => ago(secs / 604_800, "week"),
        2_629_800..31_557_600 => ago(secs / 2_629_800, "month"),
        _ => ago(secs / 31_557_600, "year"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_like_intl() {
        const HOUR: i64 = 3_600_000;
        let now = 400 * 24 * HOUR;
        assert_eq!(since(now + 5_000, now), "now");
        assert_eq!(since(now - 30_000, now), "now");
        assert_eq!(since(now - 60_000, now), "1 minute ago");
        assert_eq!(since(now - 3 * HOUR, now), "3 hours ago");
        assert_eq!(since(now - 30 * HOUR, now), "yesterday");
        assert_eq!(since(now - 3 * 24 * HOUR, now), "3 days ago");
        assert_eq!(since(now - 15 * 24 * HOUR, now), "2 weeks ago");
        assert_eq!(since(now - 399 * 24 * HOUR, now), "1 year ago");
    }
}
