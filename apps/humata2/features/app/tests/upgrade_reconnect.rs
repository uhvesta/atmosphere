//! D376: after the app migrates This Mac's old daemon and that daemon is
//! back at the staged release, the app reconnects (reloads and follows the
//! new daemon) without a reopen, and its own Migrate item ends Migrated.
//!
//! D377: the app hands the Inbox's Migrate items to the session, so a
//! session's refusal clears once the item that fixes it is migrated.
use humata2_app::{App, AppAction as A, AppState, Load, Route};
use humata2_clients::model::{
    Agent, Answer, ChannelId, MachineId, MigrateCard, MigratePhase, MigrateThing, Preview,
    Release, Request, RequestId, RequestKind, SessionId, Turn,
};
use humata2_clients::{Clients, DaemonError, Event, Fakes, Operation, Workspace, fixture};
use humata2_inbox::{InboxAction, MigrateStatus, OldLocalDaemon};
use humata2_session::{SessionAction, SessionRefused, SessionView};
use std::sync::Arc;
use std::time::Duration;
use tca::{TestClock, TestStore};

const STAGED: (&str, &str) = ("2.0.5", "a1b2c3d4e5f60718293a");

fn staged() -> Release {
    Release { version: STAGED.0.to_owned(), bundle: STAGED.1.to_owned() }
}

fn setup(
    edit: impl FnOnce(&mut Workspace),
    route: Route,
) -> (TestStore<App>, Fakes, Arc<TestClock>) {
    let mut workspace = fixture::design();
    edit(&mut workspace);
    let workspace = Arc::new(workspace);
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = Clients::fake((*workspace).clone(), clock.clone());
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace.clone();
    if let Route::Session(id) = &route {
        state.session.open = Some(id.clone());
        state.session.views.insert(id.clone(), SessionView::default());
    }
    state.route = route;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes, clock)
}

fn migrate_item(
    id: &str,
    thing: MigrateThing,
    machine: &str,
    phase: MigratePhase,
    answer: Option<Answer>,
) -> Request {
    Request {
        id: RequestId::new(id),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: format!("Migrate {thing:?} on {machine}"),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing,
            machine: MachineId::new(machine),
            from: Release { version: "2.0.4".to_owned(), bundle: "9f3e2d1c0b1a".to_owned() },
            to: staged(),
            changes: vec!["Daemon".to_owned()],
            keeps_running: Vec::new(),
            phase,
            waiting_sessions: Vec::new(),
            migrated: answer.as_ref().map(|_| "just now".to_owned()),
            steps: Vec::new(),
            holders: Vec::new(),
        }),
        doing: String::new(),
        rule: String::new(),
        undoable: false,
        asked: "1m".to_owned(),
        answer,
        operation: String::new(),
        steps: Vec::new(),
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: Vec::new(),
        on_screen: None,
        dismiss_only: false,
    }
}

// D376.

fn own_item(store: &TestStore<App>) -> RequestId {
    store
        .state()
        .inbox
        .shown()
        .into_iter()
        .find(|r| {
            r.migrate().is_some_and(|card| {
                card.thing == MigrateThing::Daemon && card.machine.as_str() == "this-mac"
            })
        })
        .map(|r| r.id.clone())
        .expect("the app's own Migrate item shows")
}

/// Shows the app's own item for an old daemon on This Mac and clicks
/// Migrate; the local migrate returns Ok.
fn migrate_this_mac(store: &mut TestStore<App>) -> RequestId {
    store.send(
        A::Inbox(InboxAction::OldLocalDaemon(Some(OldLocalDaemon {
            machine: MachineId::new("this-mac"),
            running: "2.0.4".to_owned(),
            staged: staged(),
        }))),
        |_| {},
    );
    let id = own_item(store);
    store.send(A::Inbox(InboxAction::Migrate(id.clone())), |_| {});
    store.receive_matching(
        "the local migrate returned",
        |a| matches!(a, A::Inbox(InboxAction::Answered(_, Ok(())))),
        |_| {},
    );
    store.settle();
    id
}

/// Something the new daemon holds that the app hasn't been told about: the
/// app only sees it by loading again.
fn only_after_a_reload(fakes: &Fakes) -> RequestId {
    let mut fresh = fakes.daemon.workspace().requests[0].clone();
    fresh.id = RequestId::new("held-by-the-new-daemon");
    fresh.answer = None;
    fakes.daemon.put_request(fresh.clone());
    fresh.id
}

fn holds(state: &AppState, id: &RequestId) -> bool {
    state.workspace.requests.iter().any(|r| &r.id == id)
}

#[test]
fn the_app_reconnects_once_this_macs_daemon_is_back_and_its_item_ends_migrated() {
    let (mut store, fakes, _clock) = setup(|_| {}, Route::Inbox);
    let id = migrate_this_mac(&mut store);
    let fresh = only_after_a_reload(&fakes);
    assert!(!holds(store.state(), &fresh));

    fakes.local_daemon.come_back(STAGED.1);
    store.receive_matching(
        "the app loads from the daemon it reconnected to",
        |a| matches!(a, A::Loaded(Ok(_))),
        |_| {},
    );
    store.settle();
    store.skip_received();

    let state = store.state();
    assert_eq!(state.load, Load::Ready);
    assert!(holds(state, &fresh), "the reload brought the new daemon's workspace");
    assert!(
        matches!(
            state.inbox.migrate_status(&id),
            Some(MigrateStatus::Migrated { ref to, .. }) if *to == staged()
        ),
        "{:?}",
        state.inbox.migrate_status(&id)
    );
    assert!(fakes.daemon.answers().is_empty(), "nothing was answered to a daemon");
}

#[test]
fn a_daemon_that_never_comes_back_fails_the_item_and_the_app_does_not_reload() {
    let (mut store, fakes, clock) = setup(|_| {}, Route::Inbox);
    let id = migrate_this_mac(&mut store);
    let fresh = only_after_a_reload(&fakes);

    clock.advance(Duration::from_secs(75));
    store.receive_matching(
        "the wait ran out",
        |a| matches!(a, A::Inbox(InboxAction::LocalDaemonLate)),
        |_| {},
    );
    store.settle();
    store.skip_received();

    let state = store.state();
    assert!(
        matches!(state.inbox.migrate_status(&id), Some(MigrateStatus::Failed(_))),
        "{:?}",
        state.inbox.migrate_status(&id)
    );
    assert!(!holds(state, &fresh), "no reconnect without a daemon to reconnect to");
}

// D377.

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn with_waiting_local_item(workspace: &mut Workspace) {
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == ledger())
        .expect("the design fixture has ledger");
    // Its agent takes a message only when told to send it.
    session.agent = Agent::Codex;
    session.turn = Turn::Ended;
    workspace.requests.insert(
        0,
        migrate_item("m-local", MigrateThing::Daemon, "this-mac", MigratePhase::Waiting, None),
    );
}

fn refused_send(store: &mut TestStore<App>, fakes: &Fakes) {
    fakes.daemon.fail_next_with(
        Operation::Send,
        DaemonError::MigrationNeeded {
            thing: MigrateThing::Daemon,
            machine: MachineId::new("this-mac"),
            request: Some(RequestId::new("m-local")),
        },
    );
    store.send(A::Session(SessionAction::Draft("run the tests".into())), |_| {});
    store.send(A::Session(SessionAction::Submit), |_| {});
    store.send(A::Session(SessionAction::SendQueued(0)), |_| {});
    store.receive_matching(
        "the send's result",
        |a| matches!(a, A::Session(SessionAction::Sent { .. })),
        |_| {},
    );
    store.settle();
    store.skip_received();
}

fn refusal() -> SessionRefused {
    SessionRefused::MigrationNeeded {
        thing: MigrateThing::Daemon,
        machine: MachineId::new("this-mac"),
        request: Some(RequestId::new("m-local")),
    }
}

fn daemon_says(store: &mut TestStore<App>, request: Request) {
    store.send(A::Event(Event::Request(request)), |_| {});
    store.settle();
    store.skip_received();
}

#[test]
fn a_session_refusal_clears_once_the_inbox_shows_its_item_migrated() {
    let (mut store, fakes, _clock) =
        setup(with_waiting_local_item, Route::Session(ledger()));
    refused_send(&mut store, &fakes);
    assert_eq!(store.state().session.refused, Some(refusal()));

    // Migrating isn't migrated.
    daemon_says(
        &mut store,
        migrate_item("m-local", MigrateThing::Daemon, "this-mac", MigratePhase::Migrating, None),
    );
    assert_eq!(store.state().session.refused, Some(refusal()));

    daemon_says(
        &mut store,
        migrate_item(
            "m-local",
            MigrateThing::Daemon,
            "this-mac",
            MigratePhase::Waiting,
            Some(Answer::Approved),
        ),
    );
    assert_eq!(store.state().session.refused, None);
}

#[test]
fn another_items_migration_leaves_the_session_refusal() {
    let (mut store, fakes, _clock) =
        setup(with_waiting_local_item, Route::Session(ledger()));
    refused_send(&mut store, &fakes);

    daemon_says(
        &mut store,
        migrate_item(
            "m-gpu",
            MigrateThing::Daemon,
            "gpu-box",
            MigratePhase::Waiting,
            Some(Answer::Approved),
        ),
    );
    assert_eq!(store.state().session.refused, Some(refusal()));
}
