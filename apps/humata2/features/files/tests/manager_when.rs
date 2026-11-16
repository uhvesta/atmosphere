//! Time labels from a given "now" and offset: calendar arithmetic by hand.
use humata2_files::{age_label, days_left, modified_label};

const DAY: i64 = 86_400;
/// 2024-01-01 00:00 UTC, a Monday.
const NEW_YEAR_2024: i64 = 1_704_067_200;
/// 2024-02-29 12:00 UTC, a Thursday.
const LEAP_DAY_NOON: i64 = 1_709_208_000;

#[test]
fn the_same_day_reads_today_with_the_time() {
    let modified = NEW_YEAR_2024 + 9 * 3600 + 12 * 60;
    assert_eq!(modified_label(modified, NEW_YEAR_2024 + 20 * 3600, 0), "Today 09:12");
}

#[test]
fn midnight_exactly_belongs_to_the_new_day() {
    assert_eq!(modified_label(NEW_YEAR_2024, NEW_YEAR_2024 + 1, 0), "Today 00:00");
    assert_eq!(modified_label(NEW_YEAR_2024 - 1, NEW_YEAR_2024 + 5, 0), "Sun 23:59");
}

#[test]
fn the_six_days_before_read_as_a_weekday() {
    let modified = NEW_YEAR_2024 + 9 * 3600 + 12 * 60;
    let six_days_on = NEW_YEAR_2024 + 6 * DAY + 12 * 3600;
    assert_eq!(modified_label(modified, six_days_on, 0), "Mon 09:12");
    assert_eq!(modified_label(modified, six_days_on + DAY, 0), "1 Jan");
}

#[test]
fn a_leap_day_is_a_date_in_its_year_and_a_year_later_carries_the_year() {
    assert_eq!(modified_label(LEAP_DAY_NOON, LEAP_DAY_NOON + 3 * DAY + 3600, 0), "Thu 12:00");
    assert_eq!(modified_label(LEAP_DAY_NOON, LEAP_DAY_NOON + 10 * DAY, 0), "29 Feb");
    assert_eq!(modified_label(LEAP_DAY_NOON, LEAP_DAY_NOON + 400 * DAY, 0), "29 Feb 2024");
}

#[test]
fn a_year_boundary_separates_the_years() {
    let now = NEW_YEAR_2024 + 600;
    assert_eq!(modified_label(NEW_YEAR_2024 - 1800, now, 0), "Sun 23:30");
    assert_eq!(modified_label(NEW_YEAR_2024 - 12 * DAY + 43_200, now, 0), "20 Dec 2023");
}

#[test]
fn a_negative_offset_can_move_the_local_day() {
    let modified = NEW_YEAR_2024 + 2 * 3600;
    let now = NEW_YEAR_2024 + 10 * 3600;
    assert_eq!(modified_label(modified, now, 0), "Today 02:00");
    assert_eq!(modified_label(modified, now, -5 * 3600), "Sun 21:00");
}

#[test]
fn dates_around_the_epoch_work() {
    assert_eq!(modified_label(0, 0, 0), "Today 00:00");
    assert_eq!(modified_label(0, 1_700_000_000, 0), "1 Jan 1970");
    assert_eq!(modified_label(-DAY, NEW_YEAR_2024, 0), "31 Dec 1969");
}

#[test]
fn a_time_in_the_future_reads_as_today() {
    let label = modified_label(NEW_YEAR_2024 + 5 * DAY, NEW_YEAR_2024, 0);
    assert!(label.starts_with("Today "));
}

#[test]
fn ages_step_from_now_to_days() {
    let now = 10_000_000;
    assert_eq!(age_label(now, now), "now");
    assert_eq!(age_label(now - 59, now), "now");
    assert_eq!(age_label(now - 60, now), "1m");
    assert_eq!(age_label(now - 3599, now), "59m");
    assert_eq!(age_label(now - 3600, now), "1h");
    assert_eq!(age_label(now - 86_399, now), "23h");
    assert_eq!(age_label(now - 9 * DAY, now), "9d");
    assert_eq!(age_label(now + 500, now), "now");
}

#[test]
fn days_left_rounds_up_and_stops_at_zero() {
    let now = 5_000;
    assert_eq!(days_left(now - 1, now), 0);
    assert_eq!(days_left(now, now), 0);
    assert_eq!(days_left(now + 1, now), 1);
    assert_eq!(days_left(now + DAY, now), 1);
    assert_eq!(days_left(now + DAY + 1, now), 2);
    assert_eq!(days_left(now + 30 * DAY, now), 30);
}
