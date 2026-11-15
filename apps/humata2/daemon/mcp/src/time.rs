//! RFC 3339 times for `stale_as_of` and `retry.after`, in UTC to the second.

use std::time::{SystemTime, UNIX_EPOCH};

/// `time` as `YYYY-MM-DDTHH:MM:SSZ`. Times before 1970 read as 1970.
pub(crate) fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, for days on or after the epoch).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::rfc3339;

    fn at(seconds: u64) -> String {
        rfc3339(UNIX_EPOCH + Duration::from_secs(seconds))
    }

    /// MCP.md §3: `stale_as_of` and `retry.after` are RFC 3339 in UTC.
    #[test]
    fn times_are_written_in_utc_to_the_second() {
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(1_794_657_600), "2026-11-14T12:00:00Z");
        assert_eq!(at(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    /// MCP.md §3
    #[test]
    fn a_time_before_1970_reads_as_1970() {
        assert_eq!(
            rfc3339(UNIX_EPOCH - Duration::from_secs(10)),
            "1970-01-01T00:00:00Z"
        );
    }
}
