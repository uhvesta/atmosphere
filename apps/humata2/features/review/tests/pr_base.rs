//! PR-10: a pull request review's Diff starts against each pull request's
//! base, not origin/main, and a base picked for it is kept as for any
//! session.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{DiffBase, RepoBase, SessionId, Status};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewSaved, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// The design workspace, with the session reviewing a pull request when
/// `reviews` says so.
fn workspace(reviews: bool) -> Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
            each.pull_request_review = reviews;
        }
    }
    workspace
}

fn page(reviews: bool, saved: Option<ReviewSaved>) -> TestStore<Review> {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(reviews), clock.clone());
    let mut state = ReviewState {
        workspace: Arc::new(workspace(reviews)),
        ..ReviewState::default()
    };
    if let Some(saved) = saved {
        state.load_saved(saved);
    }
    TestStore::new(state, Review::new(Arc::new(daemon), clock)).non_exhaustive()
}

/// Receives the next read of the session, which must be against `base`.
fn read_of(store: &mut TestStore<Review>, base: DiffBase) {
    store.receive_matching(
        "a read of the session's changes",
        move |action| matches!(action, A::Fetched { base: read, .. } if *read == base),
        |_| {},
    );
}

#[test]
fn a_pull_request_review_first_reads_against_each_pull_requests_base() {
    let mut store = page(true, None);
    store.send(A::Open(session()), |_| {});
    read_of(&mut store, DiffBase::PullRequestBase);
    assert_eq!(store.state().targets_of(&session()).preset, DiffBase::PullRequestBase);
    assert!(
        store.state().saved.targets.is_empty(),
        "the default is not a pick, so nothing is saved for it"
    );
}

#[test]
fn a_session_that_reviews_no_pull_request_still_reads_against_origin_main() {
    let mut store = page(false, None);
    store.send(A::Open(session()), |_| {});
    read_of(&mut store, DiffBase::OriginMain);
}

#[test]
fn origin_main_picked_for_a_pull_request_review_is_kept_across_a_restart() {
    let mut store = page(true, None);
    store.send(A::Open(session()), |_| {});
    store.skip_received();
    store.send(A::PickBase(DiffBase::OriginMain), |_| {});
    read_of(&mut store, DiffBase::OriginMain);
    let saved = store.state().saved.clone();

    let mut store = page(true, Some(saved));
    store.send(A::Open(session()), |_| {});
    read_of(&mut store, DiffBase::OriginMain);
}

#[test]
fn picking_one_repositorys_base_keeps_the_others_on_their_pull_requests_bases() {
    let mut store = page(true, None);
    store.send(A::Open(session()), |_| {});
    store.skip_received();
    store.send(
        A::PickRepoBase {
            session: session(),
            repo: "acme/api".to_owned(),
            base: RepoBase::Uncommitted,
        },
        |_| {},
    );
    let targets = store.state().targets_of(&session());
    assert_eq!(targets.preset, DiffBase::PullRequestBase);
    assert_eq!(targets.of("acme/api"), RepoBase::Uncommitted);
    assert_eq!(targets.of("acme/web"), RepoBase::PullRequestBase);
}
