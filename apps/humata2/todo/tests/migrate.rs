#![allow(clippy::expect_used)]
use humata2_todo::*;

fn d(m: u32, day: u32) -> Day {
    Day::from_ymd(2026, m, day).expect("valid date")
}
fn today() -> Day {
    d(11, 19)
}

fn old(key: &str, column: OldColumn, board: Option<&str>, rank: &str) -> OldItem {
    OldItem {
        key: key.to_owned(),
        title: key.to_owned(),
        column,
        board: board.map(str::to_owned),
        rank: rank.to_owned(),
        vip: false,
        pinned: false,
        channel: board.map(str::to_owned),
        sessions: Vec::new(),
        session_archived: false,
        created: d(11, 1),
        done_at: None,
    }
}

#[test]
fn keeps_old_order_and_says_where_each_was() {
    let mut vip = old("V-1", OldColumn::Backlog, None, "a");
    vip.vip = true;
    let items = vec![
        old("B-2", OldColumn::Backlog, None, "b"),
        old("T-1", OldColumn::ToDo, None, "a"),
        old("P-1", OldColumn::InProgress, Some("infra"), "a"),
        old("B-1", OldColumn::Backlog, None, "a2"),
        vip,
        old("D-1", OldColumn::Done, None, "a"),
    ];
    let (todos, summary) = migrate_with_summary(items);
    assert_eq!((summary.items, summary.open, summary.in_progress, summary.closed), (6, 4, 1, 1));
    assert_eq!(summary.boards, 2);

    let r = rank(&todos, today());
    let keys: Vec<&str> = r.next.iter().map(|x| x.key.as_str()).collect();
    // VIP pinned first, then To do, then Backlog by position.
    assert_eq!(keys, vec!["V-1", "T-1", "B-1", "B-2"]);
    assert_eq!(r.next[0].reasons, vec![Reason::PinnedWasVip]);
    assert_eq!(
        r.next[1].reasons,
        vec![Reason::WasAt { position: 1, column: OldColumn::ToDo, board: None }]
    );
    assert_eq!(r.in_progress[0].key, "P-1");
    assert!(todos.iter().all(|t| t.due.is_none() && t.effort.is_none() && t.impact.is_none()));
}

#[test]
fn archived_session_items_come_over_closed_and_vip_pins_precede_pins() {
    let mut a = old("A-1", OldColumn::ToDo, None, "a");
    a.session_archived = true;
    let mut p = old("P-1", OldColumn::ToDo, None, "b");
    p.pinned = true;
    let mut v = old("V-1", OldColumn::Backlog, None, "a");
    v.vip = true;
    let todos = migrate(vec![a, p, v]);
    let by = |k: &str| todos.iter().find(|t| t.key == k).expect("item");
    assert_eq!(by("A-1").state, State::Closed);
    assert_eq!(by("A-1").pin, None);
    assert!(by("V-1").pin < by("P-1").pin);
}
