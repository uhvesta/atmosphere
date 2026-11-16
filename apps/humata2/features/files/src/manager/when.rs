//! Times as the file lists show them, worked out from a given "now" and
//! offset, never from the system clock (so a view is a pure function of
//! state). The calendar arithmetic is done by hand: days since the Unix
//! epoch to a civil date and a weekday.

const DAY: i64 = 86_400;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
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
    let index = usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0);
    WEEKDAYS[index]
}

/// The label of a file's modified time, as the Files page shows it: `Today
/// 09:12` on the same local day, `Sun 21:03` within the six days before,
/// `12 Nov` in the same year, `12 Nov 2025` otherwise. A time after `now`
/// reads as today.
#[must_use]
pub fn modified_label(modified_unix_s: i64, now_unix_s: i64, utc_offset_s: i32) -> String {
    let offset = i64::from(utc_offset_s);
    let local = modified_unix_s.saturating_add(offset);
    let local_now = now_unix_s.saturating_add(offset);
    let day = local.div_euclid(DAY);
    let today = local_now.div_euclid(DAY);
    let seconds = local.rem_euclid(DAY);
    let time = format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60);
    let ago = today - day;
    if ago <= 0 {
        return format!("Today {time}");
    }
    if ago <= 6 {
        return format!("{} {time}", weekday(day));
    }
    let date = civil_from_days(day);
    let month = MONTHS[date.month.min(11)];
    if date.year == civil_from_days(today).year {
        format!("{} {month}", date.day)
    } else {
        format!("{} {month} {}", date.day, date.year)
    }
}

/// How long ago `then_unix_s` was: `now` under a minute, then `5m`, `2h`,
/// `9d`. A time after `now` reads as `now`.
#[must_use]
pub fn age_label(then_unix_s: i64, now_unix_s: i64) -> String {
    let seconds = now_unix_s.saturating_sub(then_unix_s);
    match seconds {
        i64::MIN..=59 => "now".to_owned(),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / DAY),
    }
}

/// How many days are left until `until_unix_s`, rounded up; 0 when it has
/// passed.
#[must_use]
pub fn days_left(until_unix_s: i64, now_unix_s: i64) -> u32 {
    let seconds = until_unix_s.saturating_sub(now_unix_s);
    if seconds <= 0 {
        return 0;
    }
    u32::try_from(seconds / DAY + i64::from(seconds % DAY > 0)).unwrap_or(u32::MAX)
}
