//! The upload time in history rows.

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in UTC, as `YYYY-MM-DD HH:MM`.
pub(crate) fn now_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format_utc(seconds)
}

/// Seconds since 1970-01-01 00:00 UTC as `YYYY-MM-DD HH:MM` (proleptic
/// Gregorian calendar).
fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (rem / 3_600, (rem % 3_600) / 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Days since 1970-01-01 as `(year, month, day)` in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
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
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::format_utc;

    #[test]
    fn the_epoch_is_midnight_on_1970_01_01() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
    }

    #[test]
    fn a_leap_day_is_formatted() {
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn the_time_of_day_is_formatted_to_the_minute() {
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
    }

    #[test]
    fn the_last_minute_of_2099_is_formatted() {
        assert_eq!(format_utc(4_102_444_799), "2099-12-31 23:59");
    }
}
