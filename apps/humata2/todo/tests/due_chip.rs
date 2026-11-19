#![allow(clippy::expect_used)]
use humata2_todo::*;

fn d(m: u32, day: u32) -> Day {
    Day::from_ymd(2026, m, day).expect("valid date")
}
fn today() -> Day {
    d(11, 19)
}

#[test]
fn tones_follow_distance() {
    let t = today();
    assert_eq!(chip(t.add_days(-1), t).tone, Tone::Overdue);
    assert_eq!(chip(t.add_days(-1), t).text, "Overdue · 1 day");
    assert_eq!(chip(t.add_days(-2), t).text, "Overdue · 2 days");
    assert_eq!(chip(t, t).text, "Due today");
    assert_eq!(chip(t, t).tone, Tone::Soon);
    assert_eq!(chip(t.add_days(1), t).text, "Due tomorrow");
    let later = chip(d(11, 23), t);
    assert_eq!(later.text, "Mon 23 Nov");
    assert_eq!(later.tone, Tone::Plain);
}
