#![allow(clippy::expect_used)]
use humata2_todo::*;

fn d(m: u32, day: u32) -> Day {
    Day::from_ymd(2026, m, day).expect("valid date")
}
fn today() -> Day {
    d(11, 19)
}

fn item(key: &str, channel: Option<&str>, due: Option<Day>, tags: &[&str]) -> Todo {
    let mut t = Todo::new(key, &format!("Task {key}"), d(11, 1));
    t.channel = channel.map(str::to_owned);
    t.due = due;
    t.tags = tags.iter().map(|s| s.to_string()).collect();
    t
}

fn list() -> Vec<Todo> {
    vec![
        item("PAY-1", Some("payments"), Some(d(11, 18)), &["bug"]),
        item("PAY-2", Some("payments"), Some(d(11, 22)), &[]),
        item("PAY-3", Some("payments"), Some(d(12, 30)), &["Bug", "db"]),
        item("PAY-4", Some("payments"), None, &[]),
        item("INF-1", Some("infra"), Some(d(11, 19)), &["db"]),
        item("X-1", None, None, &[]),
    ]
}

fn keys(v: &[Todo]) -> Vec<&str> {
    v.iter().map(|t| t.key.as_str()).collect()
}

#[test]
fn channel_and_due_filters_and_what_they_leave_out() {
    let f = Filter {
        channel: Some(ChannelFilter::Channel("payments".to_owned())),
        due: vec![DueFilter::Overdue, DueFilter::Next7Days],
        ..Filter::default()
    };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-1", "PAY-2"]);
    assert_eq!(keys(&left_out(&list(), &f, today())), vec!["PAY-3", "PAY-4"]);
}

#[test]
fn due_filters_or_together() {
    let f = Filter { due: vec![DueFilter::Overdue], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-1"]);
    let f = Filter { due: vec![DueFilter::Next7Days], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-2", "INF-1"]);
}

#[test]
fn tags_or_and_search_and_no_channel() {
    let f = Filter { tags: vec!["bug".to_owned(), "db".to_owned()], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-1", "PAY-3", "INF-1"]);
    let f = Filter { search: "inf-1".to_owned(), ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["INF-1"]);
    let f = Filter { channel: Some(ChannelFilter::NoChannel), ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["X-1"]);
}

#[test]
fn session_filter_and_channel_counts() {
    let mut l = list();
    l[1].sessions.push("s1".to_owned());
    let f = Filter { session: Some(SessionFilter::Session("s1".to_owned())), ..Filter::default() };
    assert_eq!(keys(&apply(&l, &f, today())), vec!["PAY-2"]);
    l[3].state = State::Closed;
    let counts = channel_counts(&l, today());
    assert_eq!(counts[0], (Some("payments".to_owned()), 3));
    assert!(counts.contains(&(None, 1)));
}

#[test]
fn after_this_week_and_no_date_are_due_choices() {
    let f = Filter { due: vec![DueFilter::AfterThisWeek], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-3"]);
    let f = Filter { due: vec![DueFilter::NoDate], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-4", "X-1"]);
    let f = Filter { due: vec![DueFilter::NoDate, DueFilter::Overdue], ..Filter::default() };
    assert_eq!(keys(&apply(&list(), &f, today())), vec!["PAY-1", "PAY-4", "X-1"]);
}

#[test]
fn effort_impact_and_only_narrow_the_list() {
    let mut items = list();
    items[0].effort = Some(Effort::S);
    items[0].impact = Some(Impact::High);
    items[1].effort = Some(Effort::L);
    items[4].ask = Some(Ask {
        kind: AskKind::Close,
        by: "claude".to_owned(),
        reason: "done".to_owned(),
    });
    items[5].review = Some(Review { repo_prs: vec![], author: "a".to_owned(), waiting_days: 1 });
    let f = Filter { effort: vec![Effort::S, Effort::L], ..Filter::default() };
    assert_eq!(keys(&apply(&items, &f, today())), vec!["PAY-1", "PAY-2"]);
    let f = Filter { impact: vec![Impact::High], ..Filter::default() };
    assert_eq!(keys(&apply(&items, &f, today())), vec!["PAY-1"]);
    let f = Filter { only: vec![Only::NeedsAttention], ..Filter::default() };
    assert_eq!(keys(&apply(&items, &f, today())), vec!["INF-1"]);
    let f = Filter { only: vec![Only::PullRequests], ..Filter::default() };
    assert_eq!(keys(&apply(&items, &f, today())), vec!["X-1"]);
}

#[test]
fn session_filter_none_yet_keeps_only_items_with_no_session() {
    let mut l = list();
    l[0].sessions.push("s1".to_owned());
    l[2].sessions.push("s2".to_owned());
    let f = Filter { session: Some(SessionFilter::NoneYet), ..Filter::default() };
    assert_eq!(keys(&apply(&l, &f, today())), vec!["PAY-2", "PAY-4", "INF-1", "X-1"]);
    let f = Filter { session: Some(SessionFilter::Session("s1".to_owned())), ..Filter::default() };
    assert_eq!(keys(&apply(&l, &f, today())), vec!["PAY-1"]);
}
