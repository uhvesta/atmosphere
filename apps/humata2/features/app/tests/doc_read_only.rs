//! DV-6 in the app reducer: while a Desk file shows an old version, comment
//! actions on that file are refused (no state change); Back to current lets
//! them through; other files are not affected.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::comments::{Anchor, Side};
use humata2_clients::{Clients, fixture, model::SessionId};
use humata2_review::comments::CommentsAction as C;
use humata2_session::{HistoryAction as H, SessionAction};
use tca::{TestClock, TestStore};

const PATH: &str = "q4-plan.md";
const OTHER: &str = "notes.md";

fn desk() -> SessionId {
    SessionId::new("desk-plan-q4")
}

fn anchor(path: &str) -> Anchor {
    Anchor {
        repo: String::new(),
        path: path.to_owned(),
        side: Side::New,
        outdated: false,
        first: 2,
        last: 2,
        quoted: vec!["one".to_owned()],
        base: None,
        detached: false,
    }
}

fn setup() -> TestStore<App> {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    fakes.daemon.commit_as_agent(&desk(), PATH, "# Plan\none\n", "First draft");
    fakes.daemon.commit_as_agent(&desk(), PATH, "# Plan\none\ntwo\n", "Add two");
    let workspace = Arc::new(fakes.daemon.workspace());
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.session.workspace = workspace.clone();
    state.desk.workspace = workspace;
    state.session.open = Some(desk());
    state.route = Route::Session(desk());
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn history(store: &mut TestStore<App>, action: H) {
    store.send(A::Session(SessionAction::History(action)), |_| {});
    for _ in 0..6 {
        store.skip_received();
    }
}

fn viewing_old(store: &mut TestStore<App>) {
    history(
        store,
        H::Load {
            session: desk(),
            path: PATH.to_owned(),
        },
    );
    history(store, H::Back { path: PATH.to_owned() });
    assert!(store.state().session.doc_read_only(&desk(), PATH));
}

fn boxed(store: &TestStore<App>) -> Option<String> {
    store
        .state()
        .comments
        .sessions
        .get(&desk())
        .and_then(|held| held.editing.as_ref())
        .map(|editing| editing.text.clone())
}

#[test]
fn a_comment_on_an_old_version_is_refused() {
    let mut store = setup();
    viewing_old(&mut store);
    store.send(A::Comments(C::OpenBox(desk(), anchor(PATH))), |_| {});
    assert_eq!(boxed(&store), None);
}

#[test]
fn going_back_to_current_lets_the_comment_through() {
    let mut store = setup();
    viewing_old(&mut store);
    store.send(A::Comments(C::OpenBox(desk(), anchor(PATH))), |_| {});
    assert_eq!(boxed(&store), None);
    history(&mut store, H::BackToCurrent { path: PATH.to_owned() });
    assert!(!store.state().session.doc_read_only(&desk(), PATH));
    store.send(A::Comments(C::OpenBox(desk(), anchor(PATH))), |_| {});
    assert_eq!(boxed(&store), Some(String::new()));
}

#[test]
fn typing_in_a_box_left_open_is_refused_once_an_old_version_shows() {
    let mut store = setup();
    history(
        &mut store,
        H::Load {
            session: desk(),
            path: PATH.to_owned(),
        },
    );
    store.send(A::Comments(C::OpenBox(desk(), anchor(PATH))), |_| {});
    assert_eq!(boxed(&store), Some(String::new()));
    history(&mut store, H::Back { path: PATH.to_owned() });
    assert!(store.state().session.doc_read_only(&desk(), PATH));
    store.send(A::Comments(C::Typed(desk(), "late words".to_owned())), |_| {});
    assert_eq!(boxed(&store), Some(String::new()));
}

#[test]
fn comments_on_another_file_are_not_affected() {
    let mut store = setup();
    viewing_old(&mut store);
    store.send(A::Comments(C::OpenBox(desk(), anchor(OTHER))), |_| {});
    assert_eq!(boxed(&store), Some(String::new()));
}
