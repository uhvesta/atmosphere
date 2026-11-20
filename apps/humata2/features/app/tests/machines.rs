//! The Add machine dialog and a machine's page inside the app: how they open,
//! how Escape closes the dialog, and what their delegates do.
use std::sync::Arc;

use humata2_app::{App, AppAction, AppState, Overlay, Route};
use humata2_clients::machines::FakeMachines;
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{Clients, fixture};
use humata2_hosts::add::Delegate as AddDelegate;
use humata2_hosts::page::Delegate as PageDelegate;
use humata2_hosts::{AddMachineAction, HostsAction, MachinePageAction};
use humata2_settings::Section;
use tca::{TestClock, TestStore};

fn store(route: Route) -> TestStore<App> {
    let (mut clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    clients.machines = Arc::new(FakeMachines::new());
    let mut state = AppState::launch(&clients);
    state.route = route;
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn devbox() -> MachineId {
    MachineId::new("devbox-1")
}

fn page(store: &mut TestStore<App>, delegate: PageDelegate) {
    store.send(
        AppAction::MachinePage(MachinePageAction::Delegate(delegate)),
        |_| {},
    );
}

#[test]
fn add_a_machine_opens_the_dialog_on_the_hosts_page() {
    let mut store = store(Route::Home);
    let hosts = store.state().hosts.clone();
    store.send(AppAction::Hosts(HostsAction::StartAdd), |_| {});
    assert_eq!(store.state().route, Route::Hosts);
    assert!(store.state().add_machine.open);
    assert_eq!(store.state().hosts, hosts);
}

#[test]
fn escape_closes_the_open_dialog() {
    let mut store = store(Route::Hosts);
    store.send(AppAction::AddMachine(AddMachineAction::Open), |_| {});
    assert!(store.state().add_machine.open);
    store.send(AppAction::Escape, |_| {});
    assert!(!store.state().add_machine.open);
}

#[test]
fn an_added_machine_is_selected_on_hosts_and_toasted() {
    let mut store = store(Route::Home);
    store.send(
        AppAction::AddMachine(AddMachineAction::Delegate(AddDelegate::Added(
            MachineId::new("devbox-9"),
        ))),
        |_| {},
    );
    assert_eq!(store.state().route, Route::Hosts);
    assert_eq!(store.state().hosts.selected, Some(MachineId::new("devbox-9")));
    store.send(
        AppAction::AddMachine(AddMachineAction::Delegate(AddDelegate::Toast(
            "Added devbox-9.".to_owned(),
        ))),
        |_| {},
    );
    assert_eq!(store.state().toasts.len(), 1);
}

#[test]
fn opening_a_machine_route_shows_that_machine() {
    let mut store = store(Route::Home);
    store.send(AppAction::Switch(Route::Machine(devbox())), |_| {});
    assert_eq!(store.state().route, Route::Machine(devbox()));
    assert_eq!(store.state().machine_page.machine, Some(devbox()));
    assert!(store.state().machine_page.loading);
}

#[test]
fn the_page_reconnects_through_hosts() {
    let mut store = store(Route::Machine(devbox()));
    page(&mut store, PageDelegate::Reconnect(devbox()));
    assert!(store.state().hosts.connecting.contains(&devbox()));
}

#[test]
fn the_page_opens_a_session_in_its_own_screen() {
    let mut store = store(Route::Machine(devbox()));
    let session = SessionId::new("ledger");
    page(&mut store, PageDelegate::OpenSession(session.clone()));
    assert_eq!(store.state().route, Route::Session(session));
}

#[test]
fn the_page_starts_a_session_on_its_machine() {
    let mut store = store(Route::Machine(devbox()));
    page(&mut store, PageDelegate::NewSession(devbox()));
    assert!(matches!(store.state().overlay(), Some(Overlay::NewSession(_))));
}

#[test]
fn the_page_opens_a_folder_in_files() {
    let mut store = store(Route::Machine(devbox()));
    page(
        &mut store,
        PageDelegate::OpenFolder { machine: devbox(), channel: ChannelId::new("infra") },
    );
    assert_eq!(store.state().route, Route::Files);
}

#[test]
fn the_page_sends_agent_access_to_settings() {
    let mut store = store(Route::Machine(devbox()));
    page(&mut store, PageDelegate::ChangeAgentAccess(devbox()));
    assert_eq!(store.state().route, Route::Settings);
    assert_eq!(store.state().settings.section, Section::Access);
}

#[test]
fn archiving_a_session_from_the_machine_page_stays_on_the_machine_page() {
    let workspace = fixture::design();
    let regular = workspace
        .sessions
        .iter()
        .find(|s| !s.archived && !humata2_clients::desk::is_desk(s))
        .expect("fixture has a live regular session")
        .id
        .clone();
    let (mut clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    clients.machines = Arc::new(FakeMachines::new());
    let mut state = AppState::launch(&clients);
    let shared = Arc::new(workspace);
    state.workspace = shared.clone();
    state.session.workspace = shared;
    state.route = Route::Machine(devbox());
    state.trail = vec![Route::Home, Route::Session(regular.clone())];
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    page(
        &mut store,
        PageDelegate::Running(humata2_hosts::RunningDelegate::ArchiveSession(regular.clone())),
    );
    assert_eq!(
        store.state().route,
        Route::Machine(devbox()),
        "starting archive from the machine page must not jump to the session page"
    );
    assert_eq!(store.state().session.open, None);
    store.settle();
    store.skip_received();

    // If unpushed work opened the Archive confirm dialog, confirm it.
    if store.state().session.archiving.is_some() {
        store.send(
            AppAction::Session(humata2_session::SessionAction::ConfirmArchive(false)),
            |_| {},
        );
        store.settle();
        store.skip_received();
    }

    assert_eq!(
        store.state().route,
        Route::Machine(devbox()),
        "completing archive from the machine page must stay on the machine page"
    );
    assert!(
        !store.state().trail.contains(&Route::Session(regular)),
        "archived session is removed from Back trail"
    );
    assert!(
        !store.state().toasts.is_empty(),
        "archive shows the undo toast"
    );
}

#[test]
fn archiving_a_desk_session_from_the_machine_page_stays_on_the_machine_page() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let desk_session = workspace
        .sessions
        .iter()
        .find(|s| !s.archived && humata2_clients::desk::is_desk(s))
        .expect("fixture has a live desk session")
        .id
        .clone();
    let (mut clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    clients.machines = Arc::new(FakeMachines::new());
    let mut state = AppState::launch(&clients);
    let shared = Arc::new(workspace);
    state.workspace = shared.clone();
    state.session.workspace = shared.clone();
    state.desk.workspace = shared;
    state.route = Route::Machine(devbox());
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    page(
        &mut store,
        PageDelegate::Running(humata2_hosts::RunningDelegate::ArchiveSession(desk_session.clone())),
    );
    assert_eq!(
        store.state().route,
        Route::Machine(devbox()),
        "closing a desk session from the machine page must not jump to the session or desk page"
    );
    store.settle();
    store.skip_received();
    assert_eq!(store.state().route, Route::Machine(devbox()));
}
