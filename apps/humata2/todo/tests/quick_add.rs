#![allow(clippy::expect_used)]
use humata2_todo::*;

fn d(m: u32, day: u32) -> Day {
    Day::from_ymd(2026, m, day).expect("valid date")
}
fn today() -> Day {
    d(11, 19)
}

fn channels() -> Vec<String> {
    ["payments-rewrite", "infra", "web", "pr-reviews"].iter().map(|s| s.to_string()).collect()
}

#[test]
fn reads_every_trailing_token() {
    let got = parse("fix login redirect loop fri #bug web s high", today(), &channels()).expect("draft");
    assert_eq!(got.title, "Fix login redirect loop");
    assert_eq!(got.due, Some(d(11, 20)));
    assert_eq!(got.tags, vec!["bug".to_owned()]);
    assert_eq!(got.channel.as_deref(), Some("web"));
    assert_eq!(got.effort, Some(Effort::S));
    assert_eq!(got.impact, Some(Impact::High));
}

#[test]
fn words_inside_the_title_stay() {
    let got = parse("raise high score tomorrow", today(), &channels()).expect("draft");
    assert_eq!(got.title, "Raise high score");
    assert_eq!(got.impact, None);
    assert_eq!(got.due, Some(d(11, 20)));
}

#[test]
fn next_week_is_next_monday_and_same_weekday_is_a_week_out() {
    let got = parse("plan roadmap next week", today(), &channels()).expect("draft");
    assert_eq!(got.title, "Plan roadmap");
    assert_eq!(got.due, Some(d(11, 23)));
    let thu = parse("write notes thu", today(), &channels()).expect("draft");
    assert_eq!(thu.due, Some(d(11, 26)));
}

#[test]
fn no_title_means_no_draft() {
    assert!(parse("", today(), &channels()).is_none());
    assert!(parse("  #bug high  ", today(), &channels()).is_none());
}

#[test]
fn plain_line_is_just_a_title() {
    let got = parse("call the bank", today(), &channels()).expect("draft");
    assert_eq!(got.title, "Call the bank");
    assert!(got.due.is_none() && got.tags.is_empty() && got.channel.is_none());
}
