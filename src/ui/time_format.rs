//! Pure timestamp formatting shared by the git panel's History tab (relative
//! time on each row) and the commit-view header (absolute date) — both
//! derived from a commit's Unix author-date timestamp. No date/time
//! dependency: the crate's stack has none (`docs/rust-best-practices.md`:
//! "every dependency is justified... the default answer is no"), and a
//! single fixed UTC format needs only plain civil-calendar arithmetic.

use std::time::{SystemTime, UNIX_EPOCH};

/// Formats `ts` (a Unix timestamp, seconds) relative to `now` (also Unix
/// seconds, seconds), GitHub-commit-list style: `"just now"`, `"Nm ago"`,
/// `"Nh ago"`, `"Nd ago"`, `"Nmo ago"`, `"Ny ago"`. A `ts` in the future
/// (clock skew between the reader and the commit's recorded author time)
/// clamps to `"just now"` rather than printing a negative duration.
pub(super) fn relative_time(now: i64, ts: i64) -> String {
    let secs = now.saturating_sub(ts).max(0);
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;
    if secs < MINUTE {
        "just now".to_string()
    } else if secs < HOUR {
        format!("{}m ago", secs / MINUTE)
    } else if secs < DAY {
        format!("{}h ago", secs / HOUR)
    } else if secs < MONTH {
        format!("{}d ago", secs / DAY)
    } else if secs < YEAR {
        format!("{}mo ago", secs / MONTH)
    } else {
        format!("{}y ago", secs / YEAR)
    }
}

/// The current wall-clock time as a Unix timestamp (seconds); `0` on a clock
/// error (a pre-1970 system clock) rather than panicking — a cosmetic
/// relative-time label is not worth a panic path over.
pub(super) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Formats `ts` (a Unix timestamp) as an absolute UTC date/time for the
/// commit-view header: `"2024-01-02 03:04 UTC"`. Pure civil-calendar math
/// (Howard Hinnant's `civil_from_days`, public domain) rather than a
/// date/time crate, since one fixed, unambiguous UTC format needs nothing
/// else.
pub(super) fn absolute_date(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs_of_day = ts.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

/// Converts a day count since the Unix epoch (1970-01-01) into a proleptic
/// Gregorian (year, month, day). Adapted from Howard Hinnant's
/// `civil_from_days` (public domain,
/// <https://howardhinnant.github.io/date_algorithms.html>).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_just_now_for_sub_minute_deltas() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now, now - 30), "just now");
    }

    #[test]
    fn relative_time_minutes_bucket() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now - 120), "2m ago");
    }

    #[test]
    fn relative_time_hours_bucket() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now - 3 * 3600), "3h ago");
    }

    #[test]
    fn relative_time_days_bucket() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now - 2 * 86_400), "2d ago");
    }

    #[test]
    fn relative_time_months_bucket() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now - 40 * 86_400), "1mo ago");
    }

    #[test]
    fn relative_time_years_bucket() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now - 400 * 86_400), "1y ago");
    }

    #[test]
    fn relative_time_clamps_future_timestamps_to_just_now() {
        // A commit's recorded author time slightly ahead of the reader's
        // clock (clock skew) must never print a negative duration.
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now + 1000), "just now");
    }

    #[test]
    fn absolute_date_formats_the_unix_epoch() {
        assert_eq!(absolute_date(0), "1970-01-01 00:00 UTC");
    }

    #[test]
    fn absolute_date_formats_a_known_instant() {
        // 2024-01-02 03:04:00 UTC.
        assert_eq!(absolute_date(1_704_164_640), "2024-01-02 03:04 UTC");
    }

    #[test]
    fn absolute_date_formats_a_leap_day() {
        // 2024-02-29 00:00:00 UTC (2024 is a leap year).
        assert_eq!(absolute_date(1_709_164_800), "2024-02-29 00:00 UTC");
    }

    #[test]
    fn now_unix_returns_a_plausible_current_timestamp() {
        // Sanity bound: any time after this crate was written and before a
        // date far enough out not to need updating.
        let now = now_unix();
        assert!(now > 1_700_000_000, "now_unix() = {now}, looks stale");
    }
}
