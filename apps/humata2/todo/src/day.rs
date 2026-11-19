use serde::{Deserialize, Serialize};

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
