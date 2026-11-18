//! VW-6 to VW-10, P7: the Journal lists every view of a session and its
//! forks in any state but Draft, newest first; Open it shows the view and
//! asks the app to open it; Revert, Keep and Close answer the view showing;
//! a watch that ends starts again.
use humata2_clients::model::{SessionId, ViewId, ViewState};
use humata2_clients::views::FakeViews;
use humata2_views::{Delegate, Views, ViewsAction as A, ViewsState};
use std::cell::Cell;
use std::sync::Arc;
use tca::TestStore;

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

/// An app on `views` that has read the offers and follows the watch.
fn started(views: &FakeViews) -> TestStore<Views> {
    let mut store =
        TestStore::new(ViewsState::default(), Views::new(Arc::new(views.clone()))).non_exhaustive();
    store.send(A::Start, |_| {});
    store.receive_matching(
        "the offers were read",
        |action| matches!(action, A::Listed(Ok(_))),
        |_| {},
    );
    store.settle();
    store
}

/// Waits for the daemon's answer to the last thing the person did.
fn answered(store: &mut TestStore<Views>) {
    store.receive_matching(
        "the daemon answered",
        |action| matches!(action, A::Answered { .. }),
        |_| {},
    );
}

fn state_of(store: &TestStore<Views>, id: &ViewId) -> Option<ViewState> {
    store.state().record(id).map(|record| record.state)
}

fn showing(store: &TestStore<Views>) -> Option<ViewId> {
    store
        .state()
        .showing
        .as_ref()
        .map(|showing| showing.id.clone())
}

fn journal(store: &TestStore<Views>, family: &[SessionId]) -> Vec<(ViewId, ViewState)> {
    store
        .state()
        .journal(family)
        .into_iter()
        .map(|record| (record.id.clone(), record.state))
        .collect()
}

/// Waits for the daemon's answer and for the app to be asked `delegate`,
/// in whichever order they come.
fn answered_and_asked(store: &mut TestStore<Views>, delegate: Delegate) {
    let asked = Cell::new(false);
    let answer = Cell::new(false);
    for _ in 0..2 {
        store.receive_matching(
            "the daemon's answer, or the app asked",
            |action| match action {
                A::Delegate(sent) if *sent == delegate && !asked.get() => {
                    asked.set(true);
                    true
                }
                A::Answered { .. } if !answer.get() => {
                    answer.set(true);
                    true
                }
                _ => false,
            },
            |_| {},
        );
    }
    assert!(asked.get() && answer.get());
}

/// Opens `id` and waits for the app to be asked to show it and for the
/// daemon's answer.
fn open(store: &mut TestStore<Views>, id: &ViewId) {
    store.send(A::Open(id.clone()), |_| {});
    answered_and_asked(store, Delegate::Opening(id.clone()));
}

#[test]
fn the_journal_lists_a_family_newest_first_in_every_state_but_draft() {
    let views = FakeViews::new();
    let parent = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let fork = views.offer(&session("s-1-fork"), "Benchmark", "BRIN vs B-tree");
    let other = views.offer(&session("s-2"), "Backfill plan", "Batches");
    let draft = views.offer(&session("s-1"), "Not ready", "Still writing");
    if let Err(error) = views.set_state_with_key(&draft, "draft-1", ViewState::Draft) {
        panic!("the fake takes the view back to a draft: {error}");
    }
    let mut store = started(&views);

    store.send(A::NotNow(parent.clone()), |_| {});
    answered(&mut store);
    store.send(A::Later(fork.clone()), |_| {});
    answered(&mut store);

    let family = [session("s-1"), session("s-1-fork")];
    assert_eq!(
        journal(&store, &family),
        vec![(fork, ViewState::Later), (parent, ViewState::NotNow)]
    );
    assert_eq!(
        journal(&store, &[session("s-2")]),
        vec![(other, ViewState::Waiting)]
    );
    store.finish();
}

#[test]
fn open_it_shows_the_view_and_asks_the_app_to_open_it() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);

    open(&mut store, &id);
    assert_eq!(showing(&store), Some(id.clone()));
    assert_eq!(state_of(&store, &id), Some(ViewState::Opened));
    assert!(store.state().offers(&session("s-1")).is_empty());
    assert_eq!(
        journal(&store, &[session("s-1")]),
        vec![(id, ViewState::Opened)]
    );
    store.finish();
}

#[test]
fn revert_answers_the_view_showing_and_it_can_be_opened_again() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);
    open(&mut store, &id);

    store.send(A::Revert, |_| {});
    assert_eq!(showing(&store), None);
    answered_and_asked(&mut store, Delegate::Reverting(id.clone()));
    assert_eq!(
        journal(&store, &[session("s-1")]),
        vec![(id.clone(), ViewState::Reverted)]
    );

    open(&mut store, &id);
    assert_eq!(showing(&store), Some(id.clone()));
    assert_eq!(state_of(&store, &id), Some(ViewState::Opened));
    store.finish();
}

#[test]
fn keep_leaves_the_view_and_ends_the_showing() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);
    open(&mut store, &id);

    store.send(A::Keep, |_| {});
    assert_eq!(showing(&store), None);
    answered(&mut store);
    assert_eq!(state_of(&store, &id), Some(ViewState::Kept));

    // Nothing is showing any more, so Revert has nothing to answer.
    store.send(A::Revert, |_| {});
    store.skip_received();
    assert_eq!(state_of(&store, &id), Some(ViewState::Kept));
    store.finish();
}

#[test]
fn close_answers_opened_and_closed() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);
    open(&mut store, &id);

    store.send(A::Close, |_| {});
    assert_eq!(showing(&store), None);
    answered(&mut store);
    assert_eq!(state_of(&store, &id), Some(ViewState::OpenedAndClosed));
    assert_eq!(
        journal(&store, &[session("s-1")]),
        vec![(id, ViewState::OpenedAndClosed)]
    );
    store.finish();
}

#[test]
fn a_watch_that_ends_starts_again_and_reads_the_offers() {
    let views = FakeViews::new();
    let mut store = started(&views);
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");

    store.send(A::WatchEnded, |_| {});
    store.receive_matching(
        "the offers are read again",
        |action| matches!(action, A::Listed(Ok(_))),
        |_| {},
    );
    store.skip_received();
    let offers: Vec<_> = store
        .state()
        .offers(&session("s-1"))
        .into_iter()
        .map(|record| record.id.clone())
        .collect();
    assert_eq!(offers, vec![id]);
    store.finish();
}
