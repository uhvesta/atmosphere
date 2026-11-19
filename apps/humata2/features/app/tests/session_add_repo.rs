//! CH-21: Add repo to this session. The sheet opens from the session menu and
//! the Diff and Files panels, searches the repo library and GitHub, and
//! clones the picked repo in the session through the daemon. Flows on the fake
//! daemon.
use std::sync::Arc;

use humata2_app::{
    App, AppAction as A, AppState, Overlay, RepoPicker, RepoSearchAction as S, Route, SessionAddRepo,
    SessionAddRepoAction as Add,
};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_session::SessionView;
use tca::{TestClock, TestStore};

fn on_session(workspace: Workspace, id: &SessionId) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.daemon.set_github_repos(vec!["acme/web".to_owned(), "octo/notes".to_owned()]);
    let workspace = Arc::new(workspace);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.desk.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.session.open = Some(id.clone());
    // The session menu is open, as when the sheet is picked from it.
    state.session.menu = true;
    state.session.views.insert(id.clone(), SessionView::default());
    state.route = Route::Session(id.clone());
    (TestStore::new(state, App::new(clients)).non_exhaustive(), fakes)
}

fn open(store: &mut TestStore<App>, id: &SessionId) {
    store.send(A::SessionAddRepo(Add::Open(id.clone())), |_| {});
    store.skip_received();
}

fn sheet(store: &TestStore<App>) -> Option<&SessionAddRepo> {
    store.state().overlays.iter().find_map(|overlay| match overlay {
        Overlay::SessionAddRepo(sheet) => Some(sheet),
        _ => None,
    })
}

fn clones_of(fakes: &Fakes, id: &SessionId) -> Vec<String> {
    fakes
        .daemon
        .workspace()
        .session(id)
        .map(|session| session.clones.iter().map(|clone| clone.repo.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn the_sheet_opens_for_the_session_and_closes_its_menu() {
    let id = SessionId::new("ledger");
    let (mut store, _) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    let Some(opened) = sheet(&store) else {
        panic!("the sheet is open");
    };
    assert_eq!(opened.session, id);
    assert!(!store.state().session.menu);
}

#[test]
fn the_sheet_searches_the_library_and_github_like_every_picker() {
    let id = SessionId::new("ledger");
    let (mut store, _) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    assert!(
        sheet(&store).is_some_and(|sheet| sheet.search.result.is_some()),
        "the search answered on open"
    );
    store.send(A::RepoSearch(RepoPicker::SessionAddRepo, S::QueryChanged("web".into())), |_| {});
    assert!(sheet(&store).is_some_and(|sheet| sheet.search.query == "web"));
}

#[test]
fn a_picked_repo_is_cloned_in_the_session_and_the_sheet_closes() {
    let id = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    store.send(A::SessionAddRepo(Add::Add(Some("acme/web".into()))), |_| {});
    store.receive_matching(
        "the daemon answered",
        |action| matches!(action, A::SessionAddRepo(Add::Added { result: Ok(()), .. })),
        |_| {},
    );
    assert!(clones_of(&fakes, &id).contains(&"acme/web".to_owned()));
    assert!(sheet(&store).is_none());
}

#[test]
fn what_was_typed_is_added_when_nothing_is_picked() {
    let id = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    store.send(A::RepoSearch(RepoPicker::SessionAddRepo, S::QueryChanged("acme/typed".into())), |_| {});
    store.send(A::SessionAddRepo(Add::Add(None)), |_| {});
    store.receive_matching(
        "the daemon answered",
        |action| matches!(action, A::SessionAddRepo(Add::Added { result: Ok(()), .. })),
        |_| {},
    );
    assert!(clones_of(&fakes, &id).contains(&"acme/typed".to_owned()));
}

#[test]
fn a_refused_repo_keeps_the_sheet_open_and_says_why() {
    let id = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    let before = clones_of(&fakes, &id);
    store.send(A::SessionAddRepo(Add::Add(Some("not a repo".into()))), |_| {});
    store.receive_matching(
        "the daemon refused",
        |action| matches!(action, A::SessionAddRepo(Add::Added { result: Err(_), .. })),
        |_| {},
    );
    let Some(shown) = sheet(&store) else {
        panic!("the sheet stays open");
    };
    assert!(shown.error.is_some());
    assert!(shown.adding.is_none(), "it can be tried again");
    assert_eq!(clones_of(&fakes, &id), before);
}

#[test]
fn adding_nothing_asks_the_daemon_nothing() {
    let id = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &id);
    open(&mut store, &id);
    let before = clones_of(&fakes, &id);
    store.send(A::SessionAddRepo(Add::Add(None)), |_| {});
    assert!(sheet(&store).is_some_and(|sheet| sheet.error.is_some() && sheet.adding.is_none()));
    assert_eq!(clones_of(&fakes, &id), before);
}

#[test]
fn a_desk_session_is_not_offered_the_sheet() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let id = SessionId::new("desk-plan-q4");
    let (mut store, _) = on_session(workspace, &id);
    open(&mut store, &id);
    assert!(sheet(&store).is_none());
}
