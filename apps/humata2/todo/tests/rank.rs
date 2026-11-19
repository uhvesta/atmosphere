#![allow(clippy::expect_used)]
use humata2_todo::*;

fn d(m: u32, day: u32) -> Day {
    Day::from_ymd(2026, m, day).expect("valid date")
}
fn today() -> Day {
    d(11, 19)
}

fn open(key: &str, created: Day) -> Todo {
    Todo::new(key, key, created)
}

fn hi_s(t: &mut Todo) {
    t.impact = Some(Impact::High);
    t.effort = Some(Effort::S);
}

fn keys(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.key.as_str()).collect()
}

fn frame_one() -> Vec<Todo> {
    let mut inf6 = open("INF-6", d(11, 2));
    inf6.channel = Some("infra".to_owned());
    inf6.pin = Some(0);
    let mut pay7 = open("PAY-7", d(11, 3));
    pay7.channel = Some("payments-rewrite".to_owned());
    pay7.due = Some(d(11, 18));
    hi_s(&mut pay7);
    let mut pay9 = open("PAY-9", d(11, 3));
    pay9.channel = Some("payments-rewrite".to_owned());
    pay9.depends_on = vec!["PAY-7".to_owned()];
    let mut rev = open("REV-1", d(11, 17));
    rev.channel = Some("pr-reviews".to_owned());
    rev.review = Some(Review {
        repo_prs: vec!["acme/api#483".to_owned()],
        author: "dana".to_owned(),
        waiting_days: 2,
    });
    let mut pay5 = open("PAY-5", d(11, 4));
    pay5.channel = Some("payments-rewrite".to_owned());
    pay5.due = Some(d(11, 23));
    pay5.impact = Some(Impact::High);
    pay5.effort = Some(Effort::M);
    let mut td3 = open("TD-3", d(11, 5));
    td3.channel = Some("web".to_owned());
    td3.due = Some(d(12, 14));
    hi_s(&mut td3);
    let mut run = open("WEB-2", d(11, 6));
    run.channel = Some("web".to_owned());
    run.state = State::InProgress;
    let mut ask = open("LED-1", d(11, 6));
    ask.ask = Some(Ask { kind: AskKind::Close, by: "claude in Ledger".to_owned(), reason: "done".to_owned() });
    let mut sug = open("", d(11, 18));
    sug.suggested = true;
    vec![td3, pay5, rev, pay9, pay7, inf6, run, ask, sug]
}

#[test]
fn first_frame_order_and_reasons() {
    let r = rank(&frame_one(), today());
    assert_eq!(keys(&r.next), vec!["INF-6", "PAY-7", "REV-1", "PAY-5", "TD-3"]);
    let pos: Vec<_> = r.next.iter().map(|x| x.position).collect();
    assert_eq!(pos, (1..=5).map(Some).collect::<Vec<_>>());
    assert_eq!(r.do_next.as_deref(), Some("INF-6"));
    assert_eq!(r.next[0].reasons, vec![Reason::Pinned]);
    assert_eq!(
        r.next[1].reasons,
        vec![Reason::HighImpactSmallJob, Reason::Blocks("PAY-9".to_owned())]
    );
    assert_eq!(r.next[2].reasons, vec![Reason::WaitingOnYou(2)]);
    assert_eq!(r.next[3].reasons, vec![Reason::HighImpact]);
    assert_eq!(keys(&r.waiting), vec!["PAY-9"]);
    assert_eq!(r.waiting[0].reasons, vec![Reason::WaitsOn("PAY-7".to_owned())]);
    assert_eq!(keys(&r.in_progress), vec!["WEB-2"]);
    assert_eq!(r.needs_attention.len(), 2);
    assert!(r.needs_attention.iter().all(|x| x.position.is_none()));
}

#[test]
fn header_counts_open_items_and_channels() {
    // Needs-attention asks count as open; suggested items do not.
    assert_eq!(header(&frame_one(), today()), (8, 4));
}

#[test]
fn filtered_list_has_its_own_top() {
    let only: Vec<Todo> = frame_one().into_iter().filter(|t| t.key == "TD-3" || t.key == "PAY-5").collect();
    assert_eq!(rank(&only, today()).do_next.as_deref(), Some("PAY-5"));
    assert_eq!(rank(&[], today()).do_next, None);
}

#[test]
fn overdue_stays_in_next_even_when_waiting_or_hidden() {
    let mut a = open("A-1", d(11, 1));
    a.due = Some(d(11, 10));
    a.depends_on = vec!["B-1".to_owned()];
    let mut b = open("B-1", d(11, 1));
    b.due = Some(d(12, 1));
    let mut c = open("C-1", d(11, 1));
    c.due = Some(d(11, 18));
    c.hide_until = Some(d(11, 25));
    let r = rank(&[a, b, c], today());
    assert_eq!(keys(&r.next), vec!["A-1", "C-1", "B-1"]);
    assert!(r.waiting.is_empty() && r.hidden.is_empty());
}

#[test]
fn hidden_comes_back_on_its_day() {
    let mut h = open("H-1", d(11, 1));
    h.hide_until = Some(d(11, 23));
    let r = rank(&[h.clone()], today());
    assert_eq!(keys(&r.hidden), vec!["H-1"]);
    assert_eq!(r.hidden[0].reasons, vec![Reason::ComesBack(d(11, 23))]);
    assert_eq!(header(&[h.clone()], today()), (0, 0));
    let later = rank(&[h], d(11, 23));
    assert_eq!(keys(&later.next), vec!["H-1"]);
}

#[test]
fn waiting_ends_when_the_blocker_closes() {
    let mut a = open("A-1", d(11, 1));
    a.depends_on = vec!["B-1".to_owned()];
    let mut b = open("B-1", d(11, 2));
    b.state = State::Closed;
    b.closed = Some(today());
    let r = rank(&[a, b], today());
    assert_eq!(keys(&r.next), vec!["A-1"]);
    assert_eq!(keys(&r.closed), vec!["B-1"]);
}

#[test]
fn closed_older_than_a_week_is_dropped() {
    let mut recent = open("R-1", d(11, 1));
    recent.state = State::Closed;
    recent.closed = Some(d(11, 13));
    let mut old = open("O-1", d(11, 1));
    old.state = State::Closed;
    old.closed = Some(d(11, 12));
    let r = rank(&[recent, old], today());
    assert_eq!(keys(&r.closed), vec!["R-1"]);
}

#[test]
fn pins_order_before_score() {
    let mut a = open("A-1", d(11, 1));
    a.pin = Some(2);
    let mut b = open("B-1", d(11, 2));
    b.pin = Some(1);
    let mut c = open("C-1", d(11, 3));
    c.due = Some(d(11, 18));
    assert_eq!(keys(&rank(&[a, b, c], today()).next), vec!["B-1", "A-1", "C-1"]);
}

#[test]
fn age_never_raises_rank_and_a_bare_list_runs_oldest_first() {
    let list = vec![open("N-3", d(11, 17)), open("N-1", d(11, 10)), open("N-2", d(11, 16))];
    let r = rank(&list, today());
    assert_eq!(keys(&r.next), vec!["N-1", "N-2", "N-3"]);
    assert_eq!(r.next[0].reasons, vec![Reason::Added(9)]);
    assert_eq!(r.next[2].reasons, vec![Reason::Added(2)]);
    // A scored item beats a much older bare one.
    let mut scored = open("S-1", d(11, 18));
    scored.impact = Some(Impact::Low);
    let r = rank(&[list[1].clone(), scored], today());
    assert_eq!(keys(&r.next), vec!["S-1", "N-1"]);
}

#[test]
fn reason_wording_is_product_copy() {
    assert_eq!(Reason::PinnedWasVip.text(), "Pinned · was VIP");
    assert_eq!(Reason::Added(0).text(), "Added today");
    assert_eq!(Reason::Added(3).text(), "Added 3 days ago");
    let was = Reason::WasAt { position: 1, column: OldColumn::ToDo, board: Some("infra".to_owned()) };
    assert_eq!(was.text(), "Was 1st in To do on infra");
    assert_eq!(
        Reason::WasAt { position: 2, column: OldColumn::Backlog, board: None }.text(),
        "Was 2nd in Backlog"
    );
    assert_eq!(Reason::WaitingOnYou(2).text(), "Waiting on you for 2 days");
    assert_eq!(Reason::Blocks("PAY-9".to_owned()).text(), "PAY-9 waits on it");
    assert_eq!(Reason::ComesBack(d(11, 23)).text(), "Comes back Mon 23 Nov");
}

#[test]
fn ask_labels() {
    let mk = |kind| Ask { kind, by: String::new(), reason: String::new() };
    assert_eq!(mk(AskKind::Close).label(), "close?");
    assert_eq!(mk(AskKind::Add).label(), "add?");
    let edit = |f: &str| AskKind::Edit { field: f.to_owned(), before: String::new(), after: String::new() };
    assert_eq!(mk(edit("due")).label(), "new due date?");
    assert_eq!(mk(edit("title")).label(), "edit?");
}
