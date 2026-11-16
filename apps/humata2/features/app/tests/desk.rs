//! The Desk through the whole app (DK-1, DK-2, DK-5, DK-6, DK-11, DK-13,
//! DK-14, SE-26): flows on the fake daemon.
use std::sync::Arc;

use humata2_app::{
    App, AppAction as A, AppState, NewChannelAction, Overlay, Route,
};
use humata2_clients::desk::{self, Document};
use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_desk::{Delegate, DeskAction as D, NewAction, Sort};
use tca::{TestClock, TestStore};

fn desk_workspace() -> Workspace {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    workspace
}

/// An app on `workspace`, at `route`, with the daemon holding the same one.
fn setup(workspace: Workspace, route: Route) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let workspace = Arc::new(workspace);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.desk.workspace = workspace;
    state.route = route;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes)
}

fn toast_delegate(action: &A) -> bool {
    matches!(action, A::Desk(D::Delegate(Delegate::Toast(_))))
}

fn desk_sessions(workspace: &Workspace) -> Vec<SessionId> {
    workspace
        .sessions
        .iter()
        .filter(|session| desk::is_desk(session))
        .map(|session| session.id.clone())
        .collect()
}

#[test]
fn empty_home_starts_a_desk_session_and_opens_it() {
    let (mut store, fakes) = setup(fixture::empty(), Route::Home);
    store.send(A::StartNewDeskSession, |_| {});
    let default_agent = store.state().settings.preferences.default_agent;
    let Some(form) = store.state().desk.new.clone() else {
        panic!("the New desk session form is open")
    };
    assert_eq!(form.agent, default_agent);
    assert_eq!(form.machine, Some(MachineId::new("this-mac")));

    store.send(A::Desk(D::New(NewAction::Title("Plan Q4 board".into()))), |_| {});
    store.send(A::Desk(D::New(NewAction::Start)), |_| {});
    store.receive_matching(
        "the daemon started the session",
        |a| matches!(a, A::Desk(D::Started(Ok(_)))),
        |_| {},
    );
    store.receive_matching(
        "open the new session",
        |a| matches!(a, A::Desk(D::Delegate(Delegate::OpenSession(_)))),
        |_| {},
    );

    let workspace = fakes.daemon.workspace();
    let started = desk_sessions(&workspace);
    let [id] = started.as_slice() else {
        panic!("exactly one desk session started, got {started:?}")
    };
    let record = workspace.desk.sessions.get(id).expect("a desk record");
    assert!(record.folder.is_scratch());
    assert_eq!(record.folder.path(), desk::scratch_path("plan-q4-board"));
    assert_eq!(store.state().route, Route::Session(id.clone()));
    assert!(store.state().desk.new.is_none());
}

#[test]
fn escape_closes_the_form_and_starts_nothing() {
    let (mut store, fakes) = setup(fixture::empty(), Route::Home);
    store.send(A::StartNewDeskSession, |_| {});
    store.send(A::Desk(D::New(NewAction::Title("Draft".into()))), |_| {});
    assert!(store.state().desk.new.is_some());
    store.send(A::Escape, |_| {});
    store.skip_received();
    assert!(store.state().desk.new.is_none());
    assert_eq!(store.state().route, Route::Home);
    assert!(desk_sessions(&fakes.daemon.workspace()).is_empty());
}

#[test]
fn closing_from_the_session_page_returns_to_the_desk() {
    let id = SessionId::new("desk-plan-q4");
    let (mut store, fakes) = setup(desk_workspace(), Route::Session(id.clone()));
    store.send(A::Desk(D::Close(id.clone())), |_| {});
    store.receive_matching("the close finished", |a| {
        matches!(a, A::Desk(D::Changed { result: Ok(()), .. }))
    }, |_| {});
    store.receive_matching("a toast", toast_delegate, |_| {});
    assert_eq!(store.state().route, Route::Desk);
    assert!(!store.state().toasts.is_empty());
    let record = fakes.daemon.workspace().desk.sessions[&id].clone();
    assert!(record.closed.is_some());
}

#[test]
fn closing_from_the_desk_page_stays_on_the_desk() {
    let id = SessionId::new("desk-plan-q4");
    let (mut store, fakes) = setup(desk_workspace(), Route::Desk);
    store.send(A::Desk(D::Close(id.clone())), |_| {});
    store.receive_matching("a toast", toast_delegate, |_| {});
    assert_eq!(store.state().route, Route::Desk);
    assert!(fakes.daemon.workspace().desk.sessions[&id].closed.is_some());
}

#[test]
fn reopening_opens_the_session_and_it_is_open_on_the_daemon() {
    let id = SessionId::new("desk-release-checklist");
    let (mut store, fakes) = setup(desk_workspace(), Route::Desk);
    store.send(A::Desk(D::Reopen(id.clone())), |_| {});
    store.receive_matching(
        "open the reopened session",
        |a| matches!(a, A::Desk(D::Delegate(Delegate::OpenSession(_)))),
        |_| {},
    );
    assert_eq!(store.state().route, Route::Session(id.clone()));
    assert!(fakes.daemon.workspace().desk.sessions[&id].closed.is_none());
}

#[test]
fn keeping_a_closed_folder_toasts_and_leaves_no_days_left() {
    let id = SessionId::new("desk-release-checklist");
    let (mut store, fakes) = setup(desk_workspace(), Route::Desk);
    store.send(A::Desk(D::Keep(id.clone())), |_| {});
    store.receive_matching("a toast", toast_delegate, |_| {});
    assert!(!store.state().toasts.is_empty());
    let record = fakes.daemon.workspace().desk.sessions[&id].clone();
    let closed = record.closed.expect("still closed");
    assert_eq!(closed.days_left, None);
    assert!(!record.folder.expires());
}

#[test]
fn clean_up_deletes_the_ticked_closed_sessions() {
    let mut workspace = fixture::design();
    fixture::clean_up(&mut workspace);
    let (mut store, fakes) = setup(workspace, Route::Desk);
    store.send(A::Desk(D::OpenCleanUp), |_| {});
    store.receive_matching(
        "the closed sessions were listed",
        |a| matches!(a, A::Desk(D::CleanUpListed(Ok(_)))),
        |_| {},
    );
    let Some(dialog) = store.state().desk.clean_up.clone() else {
        panic!("the Clean up dialog is open")
    };
    assert!(!dialog.rows.is_empty());
    assert!(!dialog.ticked.is_empty());
    let ticked = dialog.ticked;

    store.send(A::Desk(D::ConfirmCleanUp), |_| {});
    store.receive_matching(
        "the clean-up finished",
        |a| matches!(a, A::Desk(D::CleanedUp(Ok(_)))),
        |_| {},
    );
    store.receive_matching("a toast", toast_delegate, |_| {});
    assert!(store.state().desk.clean_up.is_none());
    assert!(!store.state().toasts.is_empty());
    let after = fakes.daemon.workspace();
    for id in &ticked {
        assert!(after.session(id).is_none(), "{id} should be deleted");
        assert!(!after.desk.sessions.contains_key(id));
    }
}

#[test]
fn copying_a_document_adds_it_to_the_channels_shared_folder() {
    let mut workspace = desk_workspace();
    let channel = workspace
        .channels
        .iter_mut()
        .find(|c| c.id.as_str() == "infra")
        .expect("the infra channel");
    channel.machines = vec![MachineId::new("this-mac")];
    let channel_id = channel.id.clone();
    let (mut store, fakes) = setup(workspace, Route::Desk);
    let before = fakes.daemon.workspace().files.len();
    let document = Document {
        session: SessionId::new("desk-plan-q4"),
        path: "q4-plan.md".into(),
    };
    store.send(A::Desk(D::StartCopy(document)), |_| {});
    assert!(store.state().desk.copy.is_some());
    store.send(A::Desk(D::CopyChannel(channel_id)), |_| {});
    store.send(A::Desk(D::ConfirmCopy), |_| {});
    store.receive_matching(
        "the copy finished",
        |a| matches!(a, A::Desk(D::Copied(Ok(())))),
        |_| {},
    );
    store.receive_matching("a toast", toast_delegate, |_| {});
    assert!(store.state().desk.copy.is_none());
    assert!(!store.state().toasts.is_empty());
    let files = fakes.daemon.workspace().files;
    assert_eq!(files.len(), before + 1);
    assert!(
        files
            .iter()
            .any(|file| file.machine.as_str() == "this-mac" && file.path.ends_with("/shared/q4-plan.md"))
    );
}

#[test]
fn the_desk_lists_open_sessions_by_last_use() {
    let (mut store, fakes) = setup(desk_workspace(), Route::Desk);
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.skip_received();
    let first = |store: &TestStore<App>| {
        store
            .state()
            .desk
            .open()
            .first()
            .map(|row| row.session.id.clone())
    };
    assert_eq!(first(&store), Some(SessionId::new("desk-plan-q4")));

    fakes.daemon.use_desk_session(&SessionId::new("desk-tidy-infra"));
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.skip_received();
    assert_eq!(first(&store), Some(SessionId::new("desk-tidy-infra")));
}

#[test]
fn the_sidebar_lists_the_desk_on_its_page_and_its_sessions_only() {
    let (mut store, _fakes) = setup(desk_workspace(), Route::Desk);
    assert!(store.state().desk_in_sidebar());
    let route_of = |store: &mut TestStore<App>, route: Route| {
        store.send(A::Navigate(route), |_| {});
        store.skip_received();
        store.state().desk_in_sidebar()
    };
    assert!(route_of(&mut store, Route::Session(SessionId::new("desk-plan-q4"))));
    assert!(!route_of(&mut store, Route::Home));
    assert!(!route_of(&mut store, Route::Session(SessionId::new("ledger"))));
}

#[test]
fn the_desk_sort_is_saved() {
    let (mut store, _fakes) = setup(desk_workspace(), Route::Desk);
    assert_eq!(store.state().saved().desk_sort, Sort::LastUsed);
    store.send(A::Desk(D::SetSort(Sort::Title)), |_| {});
    assert_eq!(store.state().saved().desk_sort, Sort::Title);
}

#[test]
fn no_channel_is_named_desk() {
    for name in ["desk", "Desk"] {
        let (mut store, fakes) = setup(fixture::design(), Route::Home);
        let before = fakes.daemon.workspace().channels.len();
        store.send(A::StartNewChannel, |_| {});
        store.send(A::NewChannel(NewChannelAction::Name(name.into())), |_| {});
        store.send(A::NewChannel(NewChannelAction::Create), |_| {});
        store.skip_received();
        let Some(Overlay::NewChannel(sheet)) = store.state().overlay() else {
            panic!("the New channel sheet stays open for {name}")
        };
        assert!(sheet.error.is_some());
        assert_eq!(fakes.daemon.workspace().channels.len(), before);
    }
}

#[test]
fn a_failed_change_on_a_session_page_says_so_in_a_toast() {
    let id = SessionId::new("desk-plan-q4");
    let (mut store, fakes) = setup(desk_workspace(), Route::Session(id.clone()));
    fakes.daemon.fail_next(
        humata2_clients::Operation::Desk(desk::Operation::Keep),
        "the disk is full",
    );
    store.send(A::Desk(D::Keep(id.clone())), |_| {});
    store.receive_matching(
        "the keep failed",
        |a| matches!(a, A::Desk(D::Changed { result: Err(_), .. })),
        |_| {},
    );
    assert!(!store.state().toasts.is_empty());
    assert_eq!(store.state().route, Route::Session(id));
}
