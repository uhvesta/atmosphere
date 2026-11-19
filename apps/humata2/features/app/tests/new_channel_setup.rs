//! CH-5: the New channel sheet shows the folders and clones while they run,
//! offers Retry on a failed clone and Cancel, and opens the channel when
//! every step is done. Flows on the fake daemon.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, NewChannelAction as N, Overlay, Route};
use humata2_clients::model::{ChannelId, ChannelSetup, SetupState};
use humata2_clients::{Clients, Event, Fakes, fixture};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Fakes) {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Home;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes)
}

/// Names the channel, ticks `repos` and presses Create; the fake clones the
/// repositories on This Mac and leaves the setup in its workspace.
fn make(store: &mut TestStore<App>, repos: &[&str]) {
    store.send(A::StartNewChannel, |_| {});
    store.skip_received();
    store.send(A::NewChannel(N::Name("Billing exports".into())), |_| {});
    for repo in repos {
        store.send(A::NewChannel(N::ToggleRepo((*repo).into())), |_| {});
    }
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
}

fn shown(store: &TestStore<App>) -> Option<ChannelSetup> {
    store.state().overlays.iter().find_map(|overlay| match overlay {
        Overlay::NewChannel(sheet) => sheet.setup.clone(),
        _ => None,
    })
}

fn failed(setup: &ChannelSetup) -> usize {
    setup
        .steps
        .iter()
        .filter(|step| matches!(step.state, SetupState::Failed { .. }))
        .count()
}

#[test]
fn progress_events_land_in_the_open_sheet_and_in_the_workspace() {
    let (mut store, _) = setup();
    store.send(A::StartNewChannel, |_| {});
    store.send(A::NewChannel(N::Name("Billing exports".into())), |_| {});
    let running = ChannelSetup {
        id: humata2_clients::model::SetupId("setup-1".into()),
        name: "billing-exports".into(),
        steps: vec![humata2_clients::model::SetupStep {
            machine: humata2_clients::model::MachineId::new("this-mac"),
            repo: Some("acme/ledger".into()),
            state: SetupState::Running {
                percent: 64,
                detail: "Receiving objects".into(),
            },
        }],
        cancellable: true,
    };
    store.send(A::Event(Event::ChannelSetup(running.clone())), |_| {});
    assert_eq!(shown(&store), Some(running.clone()));
    assert_eq!(store.state().workspace.setups, vec![running]);
}

#[test]
fn a_failed_clone_shows_failed_and_retry_runs_it_again_then_the_channel_opens() {
    let (mut store, fakes) = setup();
    make(&mut store, &["acme/ledger", "acme/broken-api"]);
    let pending = fakes.daemon.workspace().setups;
    assert_eq!(pending.len(), 1);
    store.send(A::Event(Event::ChannelSetup(pending[0].clone())), |_| {});
    let sheet_setup = shown(&store).expect("the sheet shows the setup");
    assert_eq!(failed(&sheet_setup), 1);
    assert!(store.state().overlay().is_some(), "the sheet stays open");

    store.send(A::NewChannel(N::RetrySetup), |_| {});
    store.skip_received();
    assert!(
        fakes
            .daemon
            .calls()
            .iter()
            .any(|call| call.starts_with("retry setup"))
    );
    let after = fakes.daemon.workspace();
    assert!(after.channels.iter().any(|c| c.name == "billing-exports"));

    let mut done = sheet_setup;
    for step in &mut done.steps {
        step.state = SetupState::Done;
    }
    store.send(A::Event(Event::ChannelSetup(done)), |_| {});
    assert!(store.state().overlay().is_none(), "the sheet closes");
    assert_eq!(
        store.state().route,
        Route::Channel(ChannelId::new("billing-exports"))
    );
}

#[test]
fn cancel_stops_the_setup_and_closes_the_sheet() {
    let (mut store, fakes) = setup();
    make(&mut store, &["acme/broken-api"]);
    let pending = fakes.daemon.workspace().setups;
    store.send(A::Event(Event::ChannelSetup(pending[0].clone())), |_| {});
    assert!(shown(&store).is_some());

    store.send(A::NewChannel(N::CancelSetup), |_| {});
    store.skip_received();
    assert!(
        fakes
            .daemon
            .calls()
            .iter()
            .any(|call| call.starts_with("cancel setup"))
    );
    assert!(fakes.daemon.workspace().setups.is_empty());
    assert!(store.state().overlay().is_none());
    assert!(store.state().workspace.setups.is_empty());
}

#[test]
fn a_clone_that_works_opens_the_channel_without_a_failed_row() {
    let (mut store, fakes) = setup();
    make(&mut store, &["acme/ledger"]);
    assert!(fakes.daemon.workspace().setups.is_empty());
    assert!(store.state().overlay().is_none());
    assert_eq!(
        store.state().route,
        Route::Channel(ChannelId::new("billing-exports"))
    );
}

/// The create's reply can come after the setup's last step opened the
/// channel, when the person has already opened a fresh New channel sheet:
/// that sheet stays open and the page stays where it is.
#[test]
fn a_late_create_reply_leaves_a_fresh_new_channel_sheet_open() {
    let (mut store, _) = setup();
    make(&mut store, &["acme/ledger"]);
    let opened = Route::Channel(ChannelId::new("billing-exports"));
    assert_eq!(store.state().route, opened);

    store.send(A::StartNewChannel, |_| {});
    store.skip_received();
    store.send(A::ChannelCreated(Ok(ChannelId::new("billing-exports"))), |_| {});
    store.skip_received();
    assert!(
        matches!(store.state().overlays.last(), Some(Overlay::NewChannel(_))),
        "the fresh sheet stays open"
    );
    assert_eq!(store.state().route, opened);
}
