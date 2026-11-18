//! The header's ⋯ session menu through the whole app (SE-7, DK-11): items the
//! app owns, and Escape.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_desk::DeskAction;
use humata2_session::{MenuItem, Panel, SessionAction as S, SessionView};
use tca::{TestClock, TestStore};

/// An app on `workspace` showing session `id`, the daemon holding the same
/// workspace.
fn on_session(workspace: Workspace, id: &SessionId) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let workspace = Arc::new(workspace);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.desk.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.session.open = Some(id.clone());
    state.session.views.insert(id.clone(), SessionView::default());
    state.route = Route::Session(id.clone());
    (TestStore::new(state, App::new(clients)).non_exhaustive(), fakes)
}

#[test]
fn archive_from_the_menu_closes_a_desk_session() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let id = SessionId::new("desk-plan-q4");
    let (mut store, fakes) = on_session(workspace, &id);
    store.send(A::Session(S::MenuPick(MenuItem::Archive)), |_| {});
    store.receive_matching(
        "the menu asks for Archive",
        |a| matches!(a, A::Session(S::Archive)),
        |_| {},
    );
    // Its unkept folder holds files, so Close session asks first (DV-4).
    let asked = store.state().desk.close.as_ref().map(|dialog| dialog.session.clone());
    assert_eq!(asked, Some(id.clone()));
    assert!(fakes.daemon.workspace().desk.sessions[&id].closed.is_none());
    store.send(A::Desk(DeskAction::Close(id.clone())), |_| {});
    store.receive_matching(
        "the close finished",
        |a| matches!(a, A::Desk(DeskAction::Changed { result: Ok(()), .. })),
        |_| {},
    );
    let record = fakes.daemon.workspace().desk.sessions[&id].clone();
    assert!(record.closed.is_some());
    assert!(!store.state().session.menu);
}

#[test]
fn push_and_pull_request_opens_the_changes_panel() {
    let id = SessionId::new("ledger");
    let (mut store, _) = on_session(fixture::design(), &id);
    store.send(A::Session(S::MenuPick(MenuItem::PushAndPullRequest)), |_| {});
    store.receive_matching(
        "the panel",
        |a| matches!(a, A::Session(S::SetPanel(Some(Panel::Changes)))),
        |_| {},
    );
    assert_eq!(store.state().panel(), Some(Panel::Changes));
}

#[test]
fn escape_closes_the_menu_then_the_rename_dialog() {
    let id = SessionId::new("ledger");
    let (mut store, _) = on_session(fixture::design(), &id);
    store.send(A::Session(S::Menu(true)), |_| {});
    store.send(A::Escape, |_| {});
    assert!(!store.state().session.menu);
    store.send(A::Session(S::StartRename), |_| {});
    assert!(store.state().session.renaming.is_some());
    store.send(A::Escape, |_| {});
    assert!(store.state().session.renaming.is_none());
}
