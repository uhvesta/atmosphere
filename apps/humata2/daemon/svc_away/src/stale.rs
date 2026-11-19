//! How a read answered from the last copy says how old it is (AWAY T10d,
//! D227): the response header `humata-stale-as-of` holds the time the host
//! answered, as RFC 3339 in UTC to the second. The MCP bridge turns it into
//! `stale_as_of`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The response metadata key.
pub const HEADER: &str = "humata-stale-as-of";

/// `unix_ms` as `YYYY-MM-DDTHH:MM:SSZ`. A time before 1970 reads as 1970.
pub fn format(unix_ms: i64) -> String {
    let seconds = unix_ms.max(0) / 1000;
    let days = seconds / 86_400;
    let of_day = seconds % 86_400;
    // Civil date from a day count (proleptic Gregorian, from 0000-03-01).
    let shifted_days = days + 719_468;
    let era = shifted_days / 146_097;
    let day_of_era = shifted_days % 146_097;
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
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

/// The time a header value of [`format`]'s form names; `None` for any other
/// text.
pub fn parse(text: &str) -> Option<SystemTime> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || !text.is_ascii() || !text.ends_with('Z') {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let digits = text.get(from..to)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators
        .iter()
        .any(|(at, want)| bytes.get(*at) != Some(want))
    {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from the epoch of a civil date (Howard Hinnant's days_from_civil).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_written_in_utc_to_the_second() {
        assert_eq!(format(0), "1970-01-01T00:00:00Z");
        assert_eq!(format(1_794_657_600_999), "2026-11-14T12:00:00Z");
        assert_eq!(format(1_709_251_199_000), "2024-02-29T23:59:59Z");
        assert_eq!(format(-5), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn what_is_written_reads_back_to_the_same_second() {
        for seconds in [
            0_u64,
            951_782_400,
            1_709_251_199,
            1_794_657_600,
            4_107_542_400,
        ] {
            let at = UNIX_EPOCH + Duration::from_secs(seconds);
            let written = format(i64::try_from(seconds * 1000).unwrap());
            assert_eq!(parse(&written), Some(at), "{written}");
        }
    }

    #[test]
    fn text_that_is_not_a_written_time_reads_as_none() {
        for text in [
            "",
            "yesterday",
            "2026-11-14 12:00:00Z",
            "2026-13-14T12:00:00Z",
            "2026-11-14T12:00:00+01:00",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
