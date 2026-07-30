//! Pure timestamp formatting shared by the git panel's History tab (relative
//! time on each row), the commit-view header (absolute date), the forge
//! thread overlay, and the Review launcher's Pull Requests tab (both parse a
//! forge-provided RFC 3339 string into relative time) — all derived from a
//! Unix timestamp in seconds. No date/time dependency: the crate's stack has
//! none (`docs/rust-best-practices.md`: "every dependency is justified... the
//! default answer is no"), and a single fixed UTC format plus one timestamp
//! parse need only plain civil-calendar arithmetic.

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

/// Days since the Unix epoch (1970-01-01) for a proleptic-Gregorian date —
/// the inverse of [`civil_from_days`] (Howard Hinnant's `days_from_civil`,
/// public domain, same source as above).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses an RFC 3339 timestamp (`YYYY-MM-DDThh:mm:ss…`, the shape both
/// GitHub's and GitLab's JSON timestamps use) into a Unix timestamp in
/// seconds. Timezone-naive: a trailing `Z`/offset is ignored, which is
/// accurate for the `Z`-suffixed UTC values these forges return and harmless
/// (a few hours' skew at worst) for the cosmetic relative-time label
/// otherwise. `None` on any shape it can't read, so the caller falls back to
/// the raw string.
pub(super) fn parse_rfc3339_to_unix(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    // Trim the timezone / fractional-second suffix off the time.
    let time = &time[..time.find(['Z', '+', '.']).unwrap_or(time.len())];
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_buckets() {
        let now = 1_700_000_000;
        // Last case: author time ahead of the reader's clock (clock skew)
        // must never print a negative duration.
        let cases: &[(i64, &str)] = &[
            (now, "just now"),
            (now - 30, "just now"),
            (now - 120, "2m ago"),
            (now - 3 * 3600, "3h ago"),
            (now - 2 * 86_400, "2d ago"),
            (now - 40 * 86_400, "1mo ago"),
            (now - 400 * 86_400, "1y ago"),
            (now + 1000, "just now"),
        ];
        for &(ts, expected) in cases {
            assert_eq!(relative_time(now, ts), expected, "ts delta {}", now - ts);
        }
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
    fn parse_rfc3339_reads_a_utc_timestamp_and_rejects_a_non_timestamp() {
        // Cross-checked against the module's own inverse.
        let base = days_from_civil(2026, 7, 1) * 86_400;
        assert_eq!(parse_rfc3339_to_unix("2026-07-01T00:00:00Z").unwrap(), base);
        assert_eq!(
            parse_rfc3339_to_unix("2026-07-01T01:02:03Z").unwrap(),
            base + 3_600 + 120 + 3
        );
        assert_eq!(parse_rfc3339_to_unix("not a date"), None);
    }
}
