use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    const ALL: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];

    pub(crate) fn from_index(i: i64) -> Weekday {
        Self::ALL[i.rem_euclid(7) as usize % 7]
    }

    pub(crate) fn index(self) -> i64 {
        Self::ALL.iter().position(|w| *w == self).map_or(0, |i| i as i64)
    }

    fn name(self) -> &'static str {
        match self {
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
            Weekday::Sun => "Sun",
        }
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A civil date, stored as days since 1970-01-01.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Day(i32);

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_len(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

impl Day {
    /// The day that is `days` after 1970-01-01.
    pub fn from_epoch_days(days: i32) -> Day {
        Day(days)
    }

    pub fn from_ymd(y: i32, m: u32, d: u32) -> Option<Day> {
        let (y, m, d) = (i64::from(y), i64::from(m), i64::from(d));
        if !(1..=12).contains(&m) || d < 1 || d > month_len(y, m) {
            return None;
        }
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        i32::try_from(days).ok().map(Day)
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        let z = i64::from(self.0) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        (y as i32, m as u32, d as u32)
    }

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday (index 3 with Monday as 0).
        Weekday::from_index(i64::from(self.0) + 3)
    }

    pub fn add_days(self, n: i32) -> Day {
        Day(self.0.saturating_add(n))
    }

    /// Whole days from `self` to `other`; negative when `other` is earlier.
    pub fn days_until(self, other: Day) -> i32 {
        other.0.saturating_sub(self.0)
    }

    /// The first day strictly after `self` that falls on `wd`.
    pub fn next_weekday(self, wd: Weekday) -> Day {
        let diff = (wd.index() - self.weekday().index()).rem_euclid(7);
        self.add_days(if diff == 0 { 7 } else { diff as i32 })
    }

    /// "Mon 23 Nov"
    pub fn short(self) -> String {
        let (_, m, d) = self.ymd();
        let month = MONTHS.get((m as usize).saturating_sub(1)).copied().unwrap_or("");
        format!("{} {} {}", self.weekday().name(), d, month)
    }
}

/// Why a typed due date couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseDayError {
    /// Nothing was typed.
    Empty,
    /// The date doesn't exist in the calendar (e.g. "2026-02-30").
    InvalidDate {
        /// What the person typed.
        input: String,
    },
    /// The text didn't match a date format we understand.
    Unrecognized {
        /// What the person typed.
        input: String,
    },
}

impl fmt::Display for ParseDayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "Type a date such as \"fri\", \"nov 28\", \"next week\", or \"2026-12-01\"."
            ),
            Self::InvalidDate { input } => write!(
                f,
                "\"{input}\" isn't a real calendar date; check the month and day."
            ),
            Self::Unrecognized { input } => write!(
                f,
                "Couldn't read \"{input}\" as a date; try \"fri\", \"nov 28\", \"next week\", or \"2026-12-01\"."
            ),
        }
    }
}

impl std::error::Error for ParseDayError {}

fn parse_weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_month(word: &str) -> Option<u32> {
    Some(match word {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    })
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn parse_iso(text: &str, raw: &str) -> Option<Result<Day, ParseDayError>> {
    let mut parts = text.split('-');
    let (Some(y), Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if y.len() != 4
        || !(1..=2).contains(&m.len())
        || !(1..=2).contains(&d.len())
        || !all_digits(y)
        || !all_digits(m)
        || !all_digits(d)
    {
        return None;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return None;
    };
    Some(Day::from_ymd(y, m, d).ok_or_else(|| ParseDayError::InvalidDate {
        input: raw.to_owned(),
    }))
}

fn parse_day_num(w: &str) -> Option<u32> {
    (all_digits(w) && (1..=2).contains(&w.len())).then(|| w.parse::<u32>().ok()).flatten()
}

fn parse_year_num(w: &str) -> Option<i32> {
    (all_digits(w) && w.len() == 4).then(|| w.parse::<i32>().ok()).flatten()
}

/// Parses a due date typed by hand relative to `today`:
/// `"today"`, `"tomorrow"`, `"next week"`, weekday names (`"fri"`),
/// month + day (`"nov 28"`, `"28 nov"`, optional 4-digit year), or ISO (`"2026-12-01"`).
///
/// # Errors
/// Returns [`ParseDayError::Empty`] for blank input,
/// [`ParseDayError::InvalidDate`] for an impossible calendar date, or
/// [`ParseDayError::Unrecognized`] when the text doesn't match any supported format.
pub fn parse_day(input: &str, today: Day) -> Result<Day, ParseDayError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseDayError::Empty);
    }
    let lower = trimmed.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() == 1 {
        let word = words[0];
        if word == "today" {
            return Ok(today);
        }
        if word == "tomorrow" {
            return Ok(today.add_days(1));
        }
        if let Some(wd) = parse_weekday(word) {
            return Ok(today.next_weekday(wd));
        }
        if let Some(iso) = parse_iso(word, trimmed) {
            return iso;
        }
    }
    if words.as_slice() == ["next", "week"] {
        return Ok(today.next_weekday(Weekday::Mon));
    }
    if words.len() == 2 || words.len() == 3 {
        let (month, day, explicit_year) = if let Some(m) = parse_month(words[0]) {
            let d = parse_day_num(words[1]);
            let y = words.get(2).and_then(|w| parse_year_num(w));
            if words.len() == 3 && y.is_none() {
                (None, None, None)
            } else {
                (Some(m), d, y)
            }
        } else if let Some(m) = parse_month(words[1]) {
            let d = parse_day_num(words[0]);
            let y = words.get(2).and_then(|w| parse_year_num(w));
            if words.len() == 3 && y.is_none() {
                (None, None, None)
            } else {
                (Some(m), d, y)
            }
        } else {
            (None, None, None)
        };
        if let (Some(m), Some(d)) = (month, day) {
            if let Some(y) = explicit_year {
                return Day::from_ymd(y, m, d).ok_or_else(|| ParseDayError::InvalidDate {
                    input: trimmed.to_owned(),
                });
            }
            let (ty, _, _) = today.ymd();
            let this_year = Day::from_ymd(ty, m, d);
            if let Some(candidate) = this_year {
                if candidate >= today {
                    return Ok(candidate);
                }
                return Day::from_ymd(ty + 1, m, d).ok_or_else(|| ParseDayError::InvalidDate {
                    input: trimmed.to_owned(),
                });
            }
            return Err(ParseDayError::InvalidDate {
                input: trimmed.to_owned(),
            });
        }
    }
    Err(ParseDayError::Unrecognized {
        input: trimmed.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_weekday() {
        let d = Day::from_ymd(2026, 11, 19).unwrap_or(Day(0));
        assert_eq!(d.ymd(), (2026, 11, 19));
        assert_eq!(d.weekday(), Weekday::Thu);
        assert_eq!(d.short(), "Thu 19 Nov");
        assert_eq!(Day::from_ymd(1970, 1, 1), Some(Day(0)));
        assert_eq!(Day(0).weekday(), Weekday::Thu);
    }

    #[test]
    fn rejects_bad_dates_and_handles_leap_years() {
        assert_eq!(Day::from_ymd(2026, 2, 29), None);
        assert!(Day::from_ymd(2028, 2, 29).is_some());
        assert_eq!(Day::from_ymd(2026, 13, 1), None);
        assert_eq!(Day::from_ymd(2026, 4, 31), None);
    }

    #[test]
    fn next_weekday_is_strictly_after() {
        let thu = Day::from_ymd(2026, 11, 19).unwrap_or(Day(0));
        assert_eq!(thu.next_weekday(Weekday::Thu).short(), "Thu 26 Nov");
        assert_eq!(thu.next_weekday(Weekday::Fri).short(), "Fri 20 Nov");
        assert_eq!(thu.next_weekday(Weekday::Mon).short(), "Mon 23 Nov");
        assert_eq!(thu.days_until(thu.add_days(-3)), -3);
    }
}
