//! Navigation bugs: the side panel's collapsed state is per session (NV-37,
//! SE-13), a session archived elsewhere closes its page, and the archive
//! toast lasts as long as the undo window (MV-15).
use std::sync::Arc;
use std::time::Duration;

use humata2_app::layout::PaneId;
use humata2_app::{App, AppAction as A, AppState, PaneAction, Route};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, Event, Workspace, fixture};
use humata2_session::{Delegate, Panel, SessionView};
use tca::{TestClock, TestStore};

fn with_panel() -> SessionView {
    SessionView {
        panel: Some(Panel::Changes),
        ..Default::default()
    }
}

fn two_sessions(workspace: &Workspace) -> (SessionId, SessionId) {
    let mut live = workspace.sessions.iter().filter(|s| !s.archived);
    let first = live.next().map(|s| s.id.clone());
    let second = live.next().map(|s| s.id.clone());
    match (first, second) {
        (Some(a), Some(b)) => (a, b),
        _ => panic!("the design fixture has at least two live sessions"),
    }
}

fn on_session(clock: Arc<TestClock>, id: Option<&SessionId>) -> (TestStore<App>, Workspace) {
    let workspace = fixture::design();
    let (clients, _fakes) = humata2_clients::fake_clients(workspace.clone(), clock);
    let mut state = AppState::launch(&clients);
    let shared = Arc::new(workspace.clone());
    state.workspace = shared.clone();
    state.session.workspace = shared;
    if let Some(id) = id {
        state.session.open = Some(id.clone());
        state.session.views.insert(id.clone(), with_panel());
        state.route = Route::Session(id.clone());
    }
    (TestStore::new(state, App::new(clients)).non_exhaustive(), workspace)
}

#[test]
fn collapsing_the_panel_in_one_session_leaves_another_session_open() {
    let (mut store, workspace) = on_session(Arc::new(TestClock::new()), None);
    let (a, b) = two_sessions(&workspace);
    store.send(A::Session(humata2_session::SessionAction::Delegate(Delegate::OpenSession(a.clone()))), |_| {});
    store.skip_received();
    store.send(
        A::Session(humata2_session::SessionAction::SetPanel(Some(Panel::Changes))),
        |_| {},
    );
    store.skip_received();
    store.send(A::Pane(PaneAction::Toggle(PaneId::Panel)), |_| {});
    assert!(store.state().layout.panel.collapsed, "collapsed in the first session");

    store.send(A::Session(humata2_session::SessionAction::Delegate(Delegate::OpenSession(b.clone()))), |_| {});
    store.skip_received();
    assert!(!store.state().layout.panel.collapsed, "the second session's panel is open");

    store.send(A::Session(humata2_session::SessionAction::Delegate(Delegate::OpenSession(a))), |_| {});
    store.skip_received();
    assert!(store.state().layout.panel.collapsed, "back in the first, still collapsed");
}

#[test]
fn a_session_archived_in_another_window_closes_its_page_here() {
    let probe = fixture::design();
    let (id, _) = two_sessions(&probe);
    let (mut store, workspace) = on_session(Arc::new(TestClock::new()), Some(&id));
    let mut gone = workspace
        .session(&id)
        .cloned()
        .unwrap_or_else(|| panic!("the fixture has {id:?}"));
    gone.archived = true;
    store.send(A::Event(Event::Session(gone)), |_| {});
    store.skip_received();
    assert_ne!(store.state().route, Route::Session(id));
}

#[test]
fn an_archived_toast_with_undo_stays_for_the_whole_undo_window() {
    let clock = Arc::new(TestClock::new());
    let (mut store, workspace) = on_session(clock.clone(), None);
    let (id, _) = two_sessions(&workspace);
    store.send(
        A::Session(humata2_session::SessionAction::Delegate(Delegate::Archived(
            id,
            "Ledger".to_owned(),
            None,
        ))),
        |_| {},
    );
    assert_eq!(store.state().toasts.len(), 1);
    clock.advance(Duration::from_secs(9));
    store.settle();
    store.skip_received();
    assert_eq!(store.state().toasts.len(), 1, "Undo is still offered after 9 s");

    clock.advance(Duration::from_secs(1));
    store.receive_matching(
        "the toast expiring",
        |a| matches!(a, A::ToastExpired(_)),
        |_| {},
    );
    assert!(store.state().toasts.is_empty());
}
