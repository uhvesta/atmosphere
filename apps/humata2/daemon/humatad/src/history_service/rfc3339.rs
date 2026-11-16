//! RFC 3339 times for the mirror, which keeps them as UTC text to the
//! millisecond, such as `2026-11-15T10:20:30.123Z`, and compares them as
//! text.

use humata2_grpc::Timestamp;

/// `at` as the mirror writes times: UTC, to the millisecond.
pub(crate) fn format(at: &Timestamp) -> String {
    let millis = at.nanos.clamp(0, 999_999_999) / 1_000_000;
    let days = at.seconds.div_euclid(86_400);
    let rest = at.seconds.rem_euclid(86_400);
    // Civil date from days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The time `text` names, or `None` when it isn't `YYYY-MM-DDTHH:MM:SS`
/// with optional fraction followed by `Z` or `+hh:mm` / `-hh:mm`.
pub(crate) fn parse(text: &str) -> Option<Timestamp> {
    let bytes = text.as_bytes();
    let number = |from: usize, len: usize| -> Option<i64> {
        let digits = bytes.get(from..from + len)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(digits).ok()?.parse().ok()
    };
    let separator = |at: usize, wanted: &[u8]| bytes.get(at).filter(|b| wanted.contains(b));
    let year = number(0, 4)?;
    separator(4, b"-")?;
    let month = number(5, 2)?;
    separator(7, b"-")?;
    let day = number(8, 2)?;
    separator(10, b"Tt ")?;
    let hour = number(11, 2)?;
    separator(13, b":")?;
    let minute = number(14, 2)?;
    separator(16, b":")?;
    let second = number(17, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut nanos = 0_i32;
    if separator(at, b".").is_some() {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
        let kept: String = text[start..at].chars().take(9).collect();
        nanos = format!("{kept:0<9}").parse().ok()?;
    }
    let offset = match bytes.get(at)? {
        b'Z' | b'z' if at + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == bytes.len() => {
            separator(at + 3, b":")?;
            let minutes = number(at + 1, 2)? * 60 + number(at + 4, 2)?;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    // Days since 1970-01-01 from the civil date.
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(Timestamp {
        seconds: days * 86_400 + hour * 3600 + minute * 60 + second - offset * 60,
        nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_to_the_millisecond() {
        let at = parse("2026-11-15T10:20:30.123456Z").expect("a valid time");
        assert_eq!(at.nanos, 123_456_000);
        assert_eq!(format(&at), "2026-11-15T10:20:30.123Z");
    }

    #[test]
    fn an_offset_moves_the_time_to_utc() {
        let at = parse("2026-11-15T01:00:00+02:00").expect("a valid time");
        assert_eq!(format(&at), "2026-11-14T23:00:00.000Z");
    }

    #[test]
    fn the_epoch_and_leap_days_round_trip() {
        for text in ["1970-01-01T00:00:00.000Z", "2024-02-29T23:59:59.999Z", "2000-03-01T00:00:00.000Z"] {
            let at = parse(text).expect("a valid time");
            assert_eq!(format(&at), text);
        }
        assert_eq!(parse("1970-01-01T00:00:00Z").map(|at| at.seconds), Some(0));
    }

    #[test]
    fn text_that_is_not_a_time_is_none() {
        for text in ["", "yesterday", "2026-13-01T00:00:00Z", "2026-11-15T10:20:30", "2026-11-15"] {
            assert!(parse(text).is_none(), "{text}");
        }
    }
}
