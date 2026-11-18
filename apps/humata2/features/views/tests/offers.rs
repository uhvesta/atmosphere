//! VW-2 to VW-5, P7: offers come from the daemon and are answered by view
//! id. Later hides an offer until the person comes back to its session, Not
//! now drops it for good, a second app sees each answer on the watch, an
//! answer the daemon refuses puts the offer back, an older revision never
//! replaces a newer one, and drafts never show.
use humata2_clients::model::{PreparedView, SessionId, ViewId, ViewState};
use humata2_clients::views::{FakeViews, ViewContent, ViewRecord};
use humata2_views::{Views, ViewsAction as A, ViewsState};
use std::sync::Arc;
use tca::TestStore;

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

/// The views page of an app on `views`, with nothing read yet.
fn app(views: &FakeViews) -> TestStore<Views> {
    TestStore::new(ViewsState::default(), Views::new(Arc::new(views.clone()))).non_exhaustive()
}

/// An app on `views` that has read the offers and follows the watch.
fn started(views: &FakeViews) -> TestStore<Views> {
    let mut store = app(views);
    store.send(A::Start, |_| {});
    store.receive_matching(
        "the offers were read",
        |action| matches!(action, A::Listed(Ok(_))),
        |_| {},
    );
    // Lets the watch, which starts after the read, open before the test
    // changes anything. The fakes answer at once; nothing waits on time.
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

/// Waits for the list of a session's views that coming back to it reads.
fn relisted(store: &mut TestStore<Views>) {
    store.receive_matching(
        "the session's views were read again",
        |action| matches!(action, A::Listed(_)),
        |_| {},
    );
}

fn offer_ids(store: &TestStore<Views>, of: &SessionId) -> Vec<ViewId> {
    store
        .state()
        .offers(of)
        .into_iter()
        .map(|record| record.id.clone())
        .collect()
}

fn state_of(store: &TestStore<Views>, id: &ViewId) -> Option<ViewState> {
    store.state().record(id).map(|record| record.state)
}

/// A view the app hears about without the fake knowing it.
fn record(id: &str, of: &str, state: ViewState, revision: u64) -> ViewRecord {
    ViewRecord {
        id: ViewId::new(id),
        view: PreparedView {
            title: "Walkthrough".to_owned(),
            session: session(of),
            body: "Three files".to_owned(),
            accepted: false,
            note: "Three files".to_owned(),
            prepared: "2m".to_owned(),
        },
        state,
        revision,
        prepared_by: "Claude Code".to_owned(),
        content: ViewContent::Summary,
    }
}

#[test]
fn offers_are_told_apart_by_id_even_when_titles_match() {
    let views = FakeViews::new();
    let first = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let second = views.offer(&session("s-1"), "Walkthrough", "Two more files");
    let elsewhere = views.offer(&session("s-2"), "Walkthrough", "Other files");
    let mut store = started(&views);

    // Newest first, and only the session's own.
    assert_eq!(
        offer_ids(&store, &session("s-1")),
        vec![second.clone(), first.clone()]
    );
    assert_eq!(offer_ids(&store, &session("s-2")), vec![elsewhere]);

    store.send(A::NotNow(first.clone()), |_| {});
    assert_eq!(offer_ids(&store, &session("s-1")), vec![second.clone()]);
    answered(&mut store);
    assert_eq!(offer_ids(&store, &session("s-1")), vec![second.clone()]);
    assert_eq!(state_of(&store, &first), Some(ViewState::NotNow));
    assert_eq!(state_of(&store, &second), Some(ViewState::Waiting));
    store.finish();
}

#[test]
fn later_hides_an_offer_until_the_person_comes_back_to_its_session() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);
    assert_eq!(offer_ids(&store, &session("s-1")), vec![id.clone()]);

    store.send(A::Later(id.clone()), |_| {});
    assert!(offer_ids(&store, &session("s-1")).is_empty());
    answered(&mut store);
    assert!(offer_ids(&store, &session("s-1")).is_empty());
    assert_eq!(state_of(&store, &id), Some(ViewState::Later));

    // Another session's return leaves it hidden.
    store.send(A::Return(session("s-2")), |_| {});
    relisted(&mut store);
    assert!(offer_ids(&store, &session("s-1")).is_empty());

    store.send(A::Return(session("s-1")), |_| {});
    relisted(&mut store);
    assert_eq!(offer_ids(&store, &session("s-1")), vec![id]);
    store.finish();
}

#[test]
fn not_now_drops_an_offer_for_good_but_the_journal_keeps_it() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut store = started(&views);

    store.send(A::NotNow(id.clone()), |_| {});
    assert!(offer_ids(&store, &session("s-1")).is_empty());
    answered(&mut store);

    store.send(A::Return(session("s-1")), |_| {});
    relisted(&mut store);
    assert!(offer_ids(&store, &session("s-1")).is_empty());
    let journal: Vec<_> = store
        .state()
        .journal(&[session("s-1")])
        .into_iter()
        .map(|record| (record.id.clone(), record.state))
        .collect();
    assert_eq!(journal, vec![(id, ViewState::NotNow)]);
    store.finish();
}

#[test]
fn an_app_opened_after_not_now_finds_the_view_in_the_journal_on_coming_back() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut first = started(&views);
    first.send(A::NotNow(id.clone()), |_| {});
    answered(&mut first);
    first.finish();

    let mut fresh = started(&views);
    fresh.send(A::Return(session("s-1")), |_| {});
    relisted(&mut fresh);
    assert!(offer_ids(&fresh, &session("s-1")).is_empty());
    let journal: Vec<_> = fresh
        .state()
        .journal(&[session("s-1")])
        .into_iter()
        .map(|record| (record.id.clone(), record.state))
        .collect();
    assert_eq!(journal, vec![(id, ViewState::NotNow)]);
    fresh.finish();
}

#[test]
fn a_second_app_sees_the_answer_on_its_watch() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut watching = started(&views);
    assert_eq!(offer_ids(&watching, &session("s-1")), vec![id.clone()]);

    let mut answering = started(&views);
    answering.send(A::NotNow(id.clone()), |_| {});
    answered(&mut answering);

    let changed = id.clone();
    watching.receive_matching(
        "the answer on the watch",
        move |action| {
            matches!(action, A::Changed(record) if record.id == changed && record.state == ViewState::NotNow)
        },
        |_| {},
    );
    assert!(offer_ids(&watching, &session("s-1")).is_empty());
    assert_eq!(state_of(&watching, &id), Some(ViewState::NotNow));
    answering.finish();
    watching.finish();
}

#[test]
fn an_offer_made_while_the_app_watches_shows_without_a_reload() {
    let views = FakeViews::new();
    let mut store = started(&views);
    assert!(offer_ids(&store, &session("s-1")).is_empty());

    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let offered = id.clone();
    store.receive_matching(
        "the offer on the watch",
        move |action| matches!(action, A::Changed(record) if record.id == offered),
        |_| {},
    );
    assert_eq!(offer_ids(&store, &session("s-1")), vec![id]);
    store.finish();
}

#[test]
fn an_answer_the_daemon_refuses_puts_the_offer_back_and_says_why() {
    // The fake has never heard of this view, so it refuses any answer.
    let views = FakeViews::new();
    let ghost = record("vw_ghost", "s-1", ViewState::Waiting, 1);
    let mut store = app(&views);
    store.send(A::Changed(ghost.clone()), |_| {});
    assert_eq!(offer_ids(&store, &session("s-1")), vec![ghost.id.clone()]);

    store.send(A::NotNow(ghost.id.clone()), |_| {});
    assert!(offer_ids(&store, &session("s-1")).is_empty());
    answered(&mut store);
    assert_eq!(offer_ids(&store, &session("s-1")), vec![ghost.id.clone()]);
    assert_eq!(store.state().record(&ghost.id), Some(&ghost));
    assert!(store.state().error.is_some());

    store.send(A::DismissError, |_| {});
    assert!(store.state().error.is_none());
    store.finish();
}

#[test]
fn an_older_revision_never_replaces_a_newer_one() {
    let views = FakeViews::new();
    let mut store = app(&views);
    let newer = record("vw_1", "s-1", ViewState::NotNow, 3);
    store.send(A::Changed(newer.clone()), |_| {});
    store.send(
        A::Changed(record("vw_1", "s-1", ViewState::Waiting, 2)),
        |_| {},
    );
    assert_eq!(store.state().record(&newer.id), Some(&newer));
    store.send(
        A::Listed(Ok(vec![record("vw_1", "s-1", ViewState::Waiting, 1)])),
        |_| {},
    );
    assert_eq!(store.state().record(&newer.id), Some(&newer));
    assert!(offer_ids(&store, &session("s-1")).is_empty());

    // A newer one does replace it.
    let newest = record("vw_1", "s-1", ViewState::Later, 4);
    store.send(A::Changed(newest.clone()), |_| {});
    assert_eq!(store.state().record(&newer.id), Some(&newest));
    store.finish();
}

#[test]
fn a_draft_never_shows_as_an_offer_or_in_the_journal() {
    let views = FakeViews::new();
    let draft = views.offer(&session("s-1"), "Walkthrough", "Not ready");
    if let Err(error) = views.set_state_with_key(&draft, "draft-1", ViewState::Draft) {
        panic!("the fake takes the view back to a draft: {error}");
    }
    let mut store = started(&views);
    store.send(A::Return(session("s-1")), |_| {});
    relisted(&mut store);
    store.send(
        A::Changed(record("vw_other", "s-1", ViewState::Draft, 1)),
        |_| {},
    );

    assert!(offer_ids(&store, &session("s-1")).is_empty());
    assert!(store.state().journal(&[session("s-1")]).is_empty());
    store.finish();
}
