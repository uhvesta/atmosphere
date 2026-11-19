//! An add blocked behind This Mac's daemon: Migrate This Mac in the dialog
//! approves the same Migrate item the Inbox shows, and the add runs again
//! once that item is migrated (the daemon is back; nothing polls).
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState};
use humata2_clients::machines::{AddEvent, AddOutcome, Call, OperationId, StepKind};
use humata2_clients::model::{MachineId, Release};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_hosts::AddMachineAction;
use humata2_inbox::{InboxAction, OldLocalDaemon};
use tca::{TestClock, TestStore};

const BUNDLE: &str = "a1b2c3d4e5f60718293a";

fn setup() -> (TestStore<App>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    // The app knows which machine is This Mac once the workspace is loaded.
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    (store, fakes)
}

fn adds(fakes: &Fakes) -> usize {
    fakes.machines.calls().into_iter().filter(|call| matches!(call, Call::Add(_))).count()
}

#[test]
fn migrating_this_mac_from_a_blocked_add_runs_the_add_again_once_the_daemon_is_back() {
    let (mut store, fakes) = setup();
    fakes.machines.script_add(vec![
        AddEvent::Started { operation: OperationId::new("op-1"), steps: vec![StepKind::Connect] },
        AddEvent::Outcome(AddOutcome::NeedsMigrate {
            platform: "linux-x86_64".to_owned(),
            what: "Migrate This Mac first.".to_owned(),
        }),
    ]);
    fakes.machines.script_add(vec![
        AddEvent::Started { operation: OperationId::new("op-2"), steps: vec![StepKind::Connect] },
        AddEvent::Outcome(AddOutcome::Added(MachineId::new("devbox-9"))),
    ]);
    store.send(A::AddMachine(AddMachineAction::Open), |_| {});
    store.send(A::AddMachine(AddMachineAction::TypeHost("devbox-9".to_owned())), |_| {});
    store.send(A::AddMachine(AddMachineAction::Start), |_| {});
    store.receive_matching(
        "the add is blocked",
        |a| matches!(a, A::AddMachine(AddMachineAction::Progress(AddEvent::Outcome(AddOutcome::NeedsMigrate { .. })))),
        |_| {},
    );
    store.settle();
    assert!(store.state().add_machine.migrate.is_some());

    // The old daemon's Migrate item (the app's own) is what the Inbox shows.
    store.send(
        A::Inbox(InboxAction::OldLocalDaemon(Some(OldLocalDaemon {
            machine: MachineId::new("this-mac"),
            running: "2.0.4".to_owned(),
            staged: Release { version: "2.0.5".to_owned(), bundle: BUNDLE.to_owned() },
        }))),
        |_| {},
    );
    assert!(store.state().add_machine.migrate_offer().is_some());

    store.send(A::AddMachine(AddMachineAction::MigrateThisMac), |_| {});
    store.receive_matching(
        "the local migrate returned",
        |a| matches!(a, A::Inbox(InboxAction::Answered(_, Ok(())))),
        |_| {},
    );
    store.settle();
    assert_eq!(adds(&fakes), 1, "the add waits for the daemon");

    fakes.local_daemon.come_back(BUNDLE);
    store.receive_matching(
        "the add runs again",
        |a| matches!(a, A::AddMachine(AddMachineAction::Progress(AddEvent::Outcome(AddOutcome::Added(_))))),
        |_| {},
    );
    store.settle();
    store.skip_received();
    assert_eq!(adds(&fakes), 2);
}
