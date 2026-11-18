//! DV-7: a repository's files can be collapsed in the open session. Toggling
//! a repository twice returns to where it was, a session's collapsed
//! repositories don't touch another's, and with no open session nothing
//! changes. Collapsing the repository that holds the selected file keeps the
//! selection.
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{Reducer, TestClock};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn other_session() -> SessionId {
    SessionId::new("brin")
}

/// The review page with `session` open and nothing collapsed.
fn page() -> ReviewState {
    ReviewState {
        session: Some(session()),
        ..ReviewState::default()
    }
}

/// Sends `action` to the review reducer, as the store would.
fn send(state: &mut ReviewState, action: A) {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(Workspace::default(), clock.clone());
    let review = Review::new(Arc::new(daemon), clock);
    let _effect = review.reduce(state, action);
}

fn toggle(repo: &str) -> A {
    A::ToggleRepo {
        repo: repo.to_owned(),
    }
}

#[test]
fn toggling_a_repository_twice_returns_to_the_first_state() {
    let mut state = page();
    let first = state.clone();

    send(&mut state, toggle("acme/api"));
    assert_ne!(state, first);

    send(&mut state, toggle("acme/api"));
    assert_eq!(state, first);
}

#[test]
fn a_collapse_in_one_session_leaves_another_sessions_collapsed_repositories_alone() {
    let mut state = page();
    send(&mut state, toggle("acme/api"));

    state.session = Some(other_session());
    send(&mut state, toggle("acme/api"));
    assert!(state.collapsed.contains(&(session(), "acme/api".to_owned())));
    assert!(state.collapsed.contains(&(other_session(), "acme/api".to_owned())));

    send(&mut state, toggle("acme/api"));
    assert_eq!(state.collapsed.len(), 1);
    assert!(state.collapsed.contains(&(session(), "acme/api".to_owned())));
}

#[test]
fn with_no_open_session_a_toggle_changes_nothing() {
    let mut state = page();
    state.session = None;
    let before = state.clone();

    send(&mut state, toggle("acme/api"));
    assert_eq!(state, before);
}

#[test]
fn collapsing_the_repository_of_the_selected_file_keeps_the_selection() {
    let mut state = page();
    let selected = ("acme/api".to_owned(), "src/lib.rs".to_owned());
    state.saved.selected.insert(session(), selected.clone());

    send(&mut state, toggle("acme/api"));
    assert_eq!(state.saved.selected.get(&session()), Some(&selected));
}
