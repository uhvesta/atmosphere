//! When something happened, as the person reads it on this Mac (D503): the
//! local clock time, with the day when it wasn't today. The text is worked
//! out from a given "now" and UTC offset, so it is a pure function of them;
//! [`LocalNow::read`] is the one place that asks the system.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAY_S: i64 = 86_400;
/// How long the local UTC offset is kept before it is read again.
const OFFSET_KEPT: Duration = Duration::from_secs(600);
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// This Mac's clock and its offset from UTC, read once for a whole mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalNow {
    pub(crate) unix_ms: i64,
    /// Seconds east of UTC.
    pub(crate) utc_offset_s: i32,
}

impl LocalNow {
    /// The clock now, and the local offset from `date`, read at most every
    /// ten minutes. An offset that can't be read is 0, so times read as UTC.
    pub(crate) fn read() -> Self {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|since| i64::try_from(since.as_millis()).ok())
            .unwrap_or(0);
        Self {
            unix_ms,
            utc_offset_s: utc_offset(),
        }
    }

    /// `at_unix_ms` as clock text: `14:02` on today's local date, `Sun
    /// 14:02` within the six days before, `12 Nov 14:02` earlier in the
    /// year, `12 Nov 2025 14:02` before that. A time after now reads as
    /// today's.
    pub(crate) fn clock(&self, at_unix_ms: i64) -> String {
        let offset = i64::from(self.utc_offset_s);
        let local = at_unix_ms.div_euclid(1000).saturating_add(offset);
        let local_now = self.unix_ms.div_euclid(1000).saturating_add(offset);
        let day = local.div_euclid(DAY_S);
        let today = local_now.div_euclid(DAY_S);
        let seconds = local.rem_euclid(DAY_S);
        let time = format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60);
        let ago = today - day;
        if ago <= 0 {
            return time;
        }
        if ago <= 6 {
            return format!("{} {time}", weekday(day));
        }
        let date = civil_from_days(day);
        let month = MONTHS[date.month.min(11)];
        if date.year == civil_from_days(today).year {
            format!("{} {month} {time}", date.day)
        } else {
            format!("{} {month} {} {time}", date.day, date.year)
        }
    }
}

/// The local offset from UTC in seconds, kept for [`OFFSET_KEPT`].
fn utc_offset() -> i32 {
    static KEPT: Mutex<Option<(Instant, i32)>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, offset)) = *kept && at.elapsed() < OFFSET_KEPT {
        return offset;
    }
    let read = std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|output| {
            crate::file_manager::parse_offset(&String::from_utf8_lossy(&output.stdout))
        })
        .unwrap_or(0);
    *kept = Some((Instant::now(), read));
    read
}

/// A civil date.
struct Civil {
    year: i64,
    month: usize,
    day: i64,
}

/// The civil date of a day counted from 1970-01-01.
fn civil_from_days(days: i64) -> Civil {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month: usize::try_from(month - 1).unwrap_or(0),
        day,
    }
}

/// The weekday of a day counted from 1970-01-01, Monday first (1970-01-01
/// was a Thursday).
fn weekday(days: i64) -> &'static str {
    WEEKDAYS[usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2023-11-14 14:02:05 UTC, a Tuesday.
    const STOPPED: i64 = 1_699_970_525_000;
    const HOUR_MS: i64 = 3_600_000;
    const DAY_MS: i64 = 24 * HOUR_MS;

    fn now(unix_ms: i64, utc_offset_s: i32) -> LocalNow {
        LocalNow { unix_ms, utc_offset_s }
    }

    #[test]
    fn a_time_today_reads_as_the_local_clock() {
        assert_eq!(now(STOPPED + HOUR_MS, 0).clock(STOPPED), "14:02");
        assert_eq!(now(STOPPED + HOUR_MS, -8 * 3600).clock(STOPPED), "06:02");
        assert_eq!(now(STOPPED + HOUR_MS, 5 * 3600 + 1800).clock(STOPPED), "19:32");
    }

    #[test]
    fn today_is_the_local_day_not_utcs() {
        // 23:30 UTC on the 14th is 00:30 on the 15th an hour east of UTC.
        let late = STOPPED + 9 * HOUR_MS + 28 * 60_000;
        let morning = late + 8 * HOUR_MS;
        assert_eq!(now(morning, 3600).clock(late), "00:30");
        assert_eq!(now(morning, 0).clock(late), "Tue 23:30");
    }

    #[test]
    fn a_time_in_the_week_before_names_its_weekday() {
        assert_eq!(now(STOPPED + DAY_MS, 0).clock(STOPPED), "Tue 14:02");
        assert_eq!(now(STOPPED + 6 * DAY_MS, 0).clock(STOPPED), "Tue 14:02");
    }

    #[test]
    fn an_older_time_names_its_date_and_its_year_when_not_this_one() {
        assert_eq!(now(STOPPED + 16 * DAY_MS, 0).clock(STOPPED), "14 Nov 14:02");
        assert_eq!(now(STOPPED + 60 * DAY_MS, 0).clock(STOPPED), "14 Nov 2023 14:02");
    }

    #[test]
    fn a_time_after_now_reads_as_today() {
        assert_eq!(now(STOPPED - DAY_MS, 0).clock(STOPPED), "14:02");
    }
}
