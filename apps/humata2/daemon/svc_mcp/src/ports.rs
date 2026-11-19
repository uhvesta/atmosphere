//! Narrow constants and pure helpers decoupled from `daemon/humatad` so
//! `humata2_svc_mcp` compiles standalone.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Session token gRPC metadata header (matches `humatad::identity::TOKEN_HEADER`).
pub const TOKEN_HEADER: &str = "humata-session-token";

/// Protobuf type URL suffix for `humata.v1.Grant` (matches `humatad::inbox::GRANT_TYPE`).
pub const GRANT_TYPE: &str = "humata.v1.Grant";

/// Approved read-result marker (matches `humatad::status::READ_APPROVED`).
pub const READ_APPROVED: &str = "humata2.ReadApproved";

/// Stale-copy response header parser (matches `humatad::away::stale`).
pub mod stale {
    use super::{Duration, SystemTime, UNIX_EPOCH};

    /// The response metadata key.
    pub const HEADER: &str = "humata-stale-as-of";

    /// The time a `YYYY-MM-DDTHH:MM:SSZ` header value names; `None` for any other text.
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
}
