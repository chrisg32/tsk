//! Timestamps for `@done(...)`, `@started(...)`, `@lasted(...)` and `@due(...)`.

use chrono::{Local, NaiveDate, NaiveDateTime};

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn format(t: NaiveDateTime, fmt: &str) -> String {
    t.format(fmt).to_string()
}

/// Parse a tag value written by us (`fmt`) or by a human in one of the
/// common date shapes. Date-only values land at midnight.
pub fn parse(value: &str, fmt: &str) -> Option<NaiveDateTime> {
    let v = value.trim();
    if let Ok(t) = NaiveDateTime::parse_from_str(v, fmt) {
        return Some(t);
    }
    const DATETIME: &[&str] = &[
        "%Y-%m-%d %H:%M",
        "%y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ];
    const DATE: &[&str] = &["%Y-%m-%d", "%y-%m-%d", "%d.%m.%Y", "%m/%d/%Y", "%m/%d/%y"];
    for f in DATETIME {
        if let Ok(t) = NaiveDateTime::parse_from_str(v, f) {
            return Some(t);
        }
    }
    for f in DATE {
        if let Ok(d) = NaiveDate::parse_from_str(v, f) {
            return d.and_hms_opt(0, 0, 0);
        }
    }
    None
}

/// Compact duration like `35m`, `2h05m`, `3d4h`, matching PlainTasks' `@lasted`.
pub fn lasted(from: NaiveDateTime, to: NaiveDateTime) -> String {
    let mins = (to - from).num_minutes().max(0);
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        "<1m".to_string()
    }
}

/// A `@due` value is overdue once its moment has passed. Date-only values are
/// due at the end of that day, not at midnight at its start.
pub fn is_overdue(value: &str, fmt: &str, now: NaiveDateTime) -> bool {
    let Some(t) = parse(value, fmt) else {
        return false;
    };
    let date_only = t.time() == chrono::NaiveTime::MIN && !value.contains(':');
    if date_only {
        now.date() > t.date()
    } else {
        now > t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn lasted_formats() {
        assert_eq!(
            lasted(dt("2026-01-01 10:00"), dt("2026-01-01 10:35")),
            "35m"
        );
        assert_eq!(
            lasted(dt("2026-01-01 10:00"), dt("2026-01-01 12:05")),
            "2h05m"
        );
        assert_eq!(
            lasted(dt("2026-01-01 10:00"), dt("2026-01-04 14:00")),
            "3d4h"
        );
        assert_eq!(
            lasted(dt("2026-01-01 10:00"), dt("2026-01-01 10:00")),
            "<1m"
        );
    }

    #[test]
    fn parses_short_and_long_dates() {
        assert_eq!(
            parse("26-09-02 14:30", "%y-%m-%d %H:%M"),
            Some(dt("2026-09-02 14:30"))
        );
        assert_eq!(
            parse("2026-09-02", "%y-%m-%d %H:%M"),
            Some(dt("2026-09-02 00:00"))
        );
        assert_eq!(parse("soon", "%y-%m-%d %H:%M"), None);
    }

    #[test]
    fn overdue_respects_date_only_values() {
        let fmt = "%y-%m-%d %H:%M";
        assert!(!is_overdue("2026-09-02", fmt, dt("2026-09-02 23:00")));
        assert!(is_overdue("2026-09-02", fmt, dt("2026-09-03 00:01")));
        assert!(is_overdue("2026-09-02 09:00", fmt, dt("2026-09-02 09:01")));
        assert!(!is_overdue("whenever", fmt, dt("2026-09-02 09:01")));
    }
}
