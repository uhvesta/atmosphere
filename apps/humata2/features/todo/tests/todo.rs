//! The To-do reducer on the fake host, with the list of frame b1-1.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::todo::{Call, FakeTodo, fake::notice};
use humata2_todo::{Day, State};
use humata2_todo_feature::{
    ChannelPick, Todo, TodoAction as A, TodoState, View, ViewFilter,
};
use tca::TestStore;

fn today() -> Day {
    Day::from_ymd(2026, 11, 19).expect("a date")
}

fn store(fake: &FakeTodo) -> TestStore<Todo> {
    TestStore::new(
        TodoState::new(View::default(), today()),
        Todo::new(Arc::new(fake.clone()), today),
    )
    .non_exhaustive()
}

/// Loads the list and waits for it.
fn loaded(store: &mut TestStore<Todo>) {
    store.send(A::Load, |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});
}

fn keys(rows: &[humata2_todo::Row]) -> Vec<&str> {
    rows.iter().map(|r| r.key.as_str()).collect()
}

fn channel(name: &str) -> ViewFilter {
    ViewFilter { channel: Some(ChannelPick::Channel(name.to_owned())), ..ViewFilter::default() }
}

fn item<'a>(state: &'a TodoState, key: &str) -> &'a humata2_todo::Todo {
    &state.items.iter().find(|e| e.todo.key == key).expect("the item is listed").todo
}

#[test]
fn load_shows_the_order_of_the_signed_off_frame() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    let ranked = store.state().ranked();
    assert_eq!(keys(&ranked.next), ["INF-6", "PAY-7", "REV-1", "PAY-5", "TD-3"]);
    assert_eq!(keys(&ranked.needs_attention), ["PAY-3", "INF-2", "SUG-1"]);
    assert_eq!(keys(&ranked.in_progress), ["WEB-8"]);
    assert_eq!(keys(&ranked.waiting), ["PAY-9", "WEB-13"]);
    assert_eq!(ranked.hidden.len(), 3);
    assert_eq!(store.state().header(), (10, 4));
    assert_eq!(ranked.do_next.as_deref(), Some("INF-6"));
}

#[test]
fn a_channel_filter_narrows_the_list_and_the_header() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::SetFilter(channel("payments-rewrite")), |_| {});
    let state = store.state();
    assert_eq!(state.header(), (4, 1));
    assert_eq!(keys(&state.ranked().next), ["PAY-7", "PAY-5"]);

    store.send(A::ClearFilter, |_| {});
    assert_eq!(store.state().header(), (10, 4));
}

#[test]
fn quick_add_reads_the_line_previews_the_place_and_adds_the_item() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::QuickAddChanged("fix login redirect loop fri #bug web s high".to_owned()), |_| {});
    let preview = store.state().quick_add.preview.clone().expect("a title is typed");
    assert_eq!(preview.draft.title, "Fix login redirect loop");
    assert_eq!(preview.draft.channel.as_deref(), Some("web"));
    assert_eq!(preview.draft.tags, ["bug"]);
    assert_eq!(preview.draft.due, Day::from_ymd(2026, 11, 20));
    // Behind the pin and the overdue refund retry, ahead of the review.
    assert_eq!(preview.position, Some(3));

    store.send(A::QuickAddSubmit, |_| {});
    store.receive_matching("Created", |a| matches!(a, A::Created(Ok(_))), |_| {});
    let state = store.state();
    assert!(state.quick_add.line.is_empty() && state.quick_add.preview.is_none());
    assert_eq!(state.header(), (11, 4));
    let new = state.items.last().expect("the new item").todo.clone();
    assert_eq!(new.title, "Fix login redirect loop");
    assert_eq!(new.channel.as_deref(), Some("web"));
    assert_eq!(keys(&state.ranked().next)[..3], ["INF-6", "PAY-7", new.key.as_str()]);
    assert!(fake.calls().iter().any(|c| matches!(c, Call::Create(d) if d.title == new.title)));
}

#[test]
fn an_empty_quick_add_line_adds_nothing() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);
    store.send(A::QuickAddChanged("   ".to_owned()), |_| {});
    assert!(store.state().quick_add.preview.is_none());
    store.send(A::QuickAddSubmit, |_| {});
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::Create(_))));
}

#[test]
fn close_then_reopen() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::ShowClosed(true), |_| {});
    store.send(A::SetState("PAY-5".to_owned(), State::Closed), |_| {});
    // Before the host answers, the item is already out of Next.
    let ranked = store.state().ranked();
    assert_eq!(keys(&ranked.closed), ["PAY-5"]);
    assert!(!keys(&ranked.next).contains(&"PAY-5"));
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").state, State::Closed);

    store.send(A::SetState("PAY-5".to_owned(), State::Open), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    let ranked = store.state().ranked();
    assert!(ranked.closed.is_empty());
    assert!(keys(&ranked.next).contains(&"PAY-5"));
    let held = fake.entries().into_iter().find(|e| e.todo.key == "PAY-5").expect("held");
    assert_eq!(held.todo.state, State::Open);
}

#[test]
fn pin_goes_after_the_pins_and_move_pin_puts_it_on_top() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::Pin("PAY-5".to_owned()), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(keys(&store.state().ranked().next)[..2], ["INF-6", "PAY-5"]);

    store.send(A::MovePin("PAY-5".to_owned(), 0), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(keys(&store.state().ranked().next)[..2], ["PAY-5", "INF-6"]);
    let held = |key: &str| {
        fake.entries().into_iter().find(|e| e.todo.key == key).expect("held").todo.pin
    };
    assert_eq!((held("PAY-5"), held("INF-6")), (Some(0), Some(1)));

    store.send(A::Unpin("PAY-5".to_owned()), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").pin, None);
}

#[test]
fn answering_yes_clears_needs_attention() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::Answer("req-pay-3".to_owned(), true), |_| {});
    assert!(item(store.state(), "PAY-3").ask.is_none());
    assert_eq!(keys(&store.state().ranked().needs_attention), ["INF-2", "SUG-1"]);
    store.receive_matching("Answered", |a| matches!(a, A::Answered { .. }), |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});

    // The host acted on the yes: the item is closed.
    assert_eq!(item(store.state(), "PAY-3").state, State::Closed);
    assert!(fake.calls().contains(&Call::Answer("req-pay-3".to_owned(), true)));
}

#[test]
fn a_failed_answer_puts_the_ask_back_and_says_so() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    fake.fail_next(DaemonError::NotFound { what: "This ask".to_owned() });
    store.send(A::Answer("req-pay-3".to_owned(), false), |_| {});
    store.receive_matching("Answered", |a| matches!(a, A::Answered { .. }), |_| {});
    assert!(item(store.state(), "PAY-3").ask.is_some());
    assert!(store.state().error.is_some());
}

#[test]
fn a_stale_revision_rolls_back_and_shows_an_error() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);
    let before = store.state().items.clone();

    // Another screen changed the item after this one listed it.
    fake.touch("PAY-5");
    store.send(A::SetState("PAY-5".to_owned(), State::Closed), |_| {});
    assert_eq!(item(store.state(), "PAY-5").state, State::Closed);
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});

    assert_eq!(store.state().items, before);
    assert!(matches!(store.state().error, Some(DaemonError::Conflict { .. })));
    let held = fake.entries().into_iter().find(|e| e.todo.key == "PAY-5").expect("held");
    assert_eq!(held.todo.state, State::Open);
}

#[test]
fn do_next_picks_the_first_next_row_among_those_shown() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::DoNext, |_| {});
    assert_eq!(store.state().do_next.as_deref(), Some("INF-6"));
    store.send(A::CloseDoNext, |_| {});
    assert_eq!(store.state().do_next, None);

    // INF-6 is in another channel, so it is not among the rows shown.
    store.send(A::SetFilter(channel("payments-rewrite")), |_| {});
    store.send(A::DoNext, |_| {});
    assert_eq!(store.state().do_next.as_deref(), Some("PAY-7"));
    store.send(A::SkipDoNext, |_| {});
    assert_eq!(store.state().do_next.as_deref(), Some("PAY-5"));
}

#[test]
fn do_next_with_nothing_to_do_opens_nothing() {
    let fake = FakeTodo::new();
    let mut store = store(&fake);
    loaded(&mut store);
    store.send(A::DoNext, |_| {});
    assert_eq!(store.state().do_next, None);
}

#[test]
fn dismissing_the_migration_banner_calls_the_client() {
    let fake = FakeTodo::frame_b1_1();
    fake.set_migration(Some(notice(8, 4, 2, 3)));
    let mut store = store(&fake);
    loaded(&mut store);
    assert!(store.state().migration.is_some());

    store.send(A::DismissMigration, |_| {});
    assert!(store.state().migration.is_none());
    store.receive_matching("Dismissed", |a| matches!(a, A::Dismissed { .. }), |_| {});
    assert!(fake.calls().contains(&Call::DismissMigration));
    assert!(store.state().migration.is_none());
}

#[test]
fn a_change_from_another_screen_arrives_through_the_watch() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);
    store.send(A::Watch, |_| {});
    store.settle();

    fake.touch("PAY-5");
    store.receive_matching("Updated", |a| matches!(a, A::Updated(_)), |_| {});
    let revision = |s: &TodoState| s.items.iter().find(|e| e.todo.key == "PAY-5").map(|e| e.revision);
    assert_eq!(revision(store.state()), Some(1));
}

#[test]
fn searching_keeps_the_rest_of_the_filter() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::SetFilter(channel("payments-rewrite")), |_| {});
    store.send(A::SetSearch("backoff".to_owned()), |_| {});
    let filter = &store.state().view.filter;
    assert_eq!(filter.search, "backoff");
    assert_eq!(filter.channel, Some(ChannelPick::Channel("payments-rewrite".to_owned())));
    assert_eq!(keys(&store.state().ranked().next), ["PAY-7"]);
}

#[test]
fn the_more_filters_narrow_by_only_effort_and_impact() {
    use humata2_todo_feature::OnlyPick;
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(
        A::SetFilter(ViewFilter { only: vec![OnlyPick::PullRequests], ..ViewFilter::default() }),
        |_| {},
    );
    assert_eq!(keys(&store.state().ranked().next), ["REV-1"]);
    store.send(
        A::SetFilter(ViewFilter { only: vec![OnlyPick::NeedsAttention], ..ViewFilter::default() }),
        |_| {},
    );
    assert_eq!(keys(&store.state().ranked().needs_attention), ["PAY-3", "INF-2", "SUG-1"]);
}

#[test]
fn what_changed_opens_and_got_it_closes_it() {
    let fake = FakeTodo::frame_b1_1();
    fake.set_migration(Some(notice(10, 2, 3, 2)));
    let mut store = store(&fake);
    loaded(&mut store);

    assert!(!store.state().what_changed);
    store.send(A::ShowWhatChanged, |_| {});
    assert!(store.state().what_changed);
    store.send(A::ShowWhatChanged, |_| {});
    assert!(!store.state().what_changed);
}

#[test]
fn a_description_is_edited_and_saved() {
    use humata2_todo_feature::EditField;
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::StartEdit("PAY-3".to_owned(), EditField::Description), |_| {});
    assert!(store.state().editing.as_ref().is_some_and(|e| e.text.starts_with("Split postings")));
    store.send(A::EditChanged("Shorter.".to_owned()), |_| {});
    store.send(A::CommitEdit, |_| {});
    assert_eq!(item(store.state(), "PAY-3").description, "Shorter.");
}

#[test]
fn asking_why_and_opening_the_session_change_nothing_here() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    let before = store.state().items.clone();
    store.send(A::AskWhy("req-pay-3".to_owned()), |_| {});
    store.send(A::OpenSession("Ledger entries".to_owned()), |_| {});
    store.send(A::ShowHistory("PAY-3".to_owned()), |_| {});
    assert_eq!(store.state().items, before);
}
