//! Read marks on the fake daemon (CH-16, CH-17; D392), kept as the real
//! daemon keeps them: output on the agent tab of a session nobody views
//! marks it unread, viewing it clears that and keeps it read, and opening
//! a session that couldn't come back keeps its card.

use std::sync::Arc;

use humata2_clients::model::{MachineId, Recovery, SessionId};
use humata2_clients::{Daemon, FakeDaemon, Terminals, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn fake() -> FakeDaemon {
    FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()))
}

fn unread(daemon: &FakeDaemon, id: &SessionId) -> u32 {
    match daemon.workspace().session(id) {
        Some(session) => session.unread,
        None => panic!("{id} is in the workspace"),
    }
}

/// A session of the fixture with no unread mark.
fn read_session(daemon: &FakeDaemon) -> SessionId {
    match daemon.workspace().sessions.iter().find(|session| session.unread == 0) {
        Some(session) => session.id.clone(),
        None => panic!("the fixture has a read session"),
    }
}

#[test]
fn output_nobody_views_marks_a_session_unread() {
    let daemon = fake();
    let id = read_session(&daemon);
    block_on(daemon.write(&id, 0, "ls")).expect("tab 0 takes input");
    assert!(unread(&daemon, &id) > 0);
}

#[test]
fn a_viewed_session_stays_read_until_the_view_moves_on() {
    let daemon = fake();
    let id = read_session(&daemon);
    block_on(daemon.write(&id, 0, "ls")).expect("tab 0 takes input");
    block_on(daemon.view(Some(&id))).expect("view it");
    assert_eq!(unread(&daemon, &id), 0, "viewing it read it");
    block_on(daemon.write(&id, 0, "pwd")).expect("tab 0 takes input");
    assert_eq!(unread(&daemon, &id), 0, "output while it is viewed marks nothing");
    block_on(daemon.view(None)).expect("view nothing");
    block_on(daemon.write(&id, 0, "ls")).expect("tab 0 takes input");
    assert!(unread(&daemon, &id) > 0, "output after the view moved on marks it");
}

#[test]
fn opening_a_session_views_it() {
    let daemon = fake();
    let id = read_session(&daemon);
    block_on(daemon.mark_read(&id)).expect("it opens");
    block_on(daemon.write(&id, 0, "ls")).expect("tab 0 takes input");
    assert_eq!(unread(&daemon, &id), 0);
}

#[test]
fn opening_a_session_that_could_not_come_back_keeps_its_card() {
    let daemon = fake();
    let outcomes =
        block_on(daemon.reconnect(&MachineId::new("ci-runner"))).expect("the host reconnects");
    let Some((id, _)) = outcomes
        .iter()
        .find(|(_, recovery)| matches!(recovery, Recovery::Recoverable { .. }))
    else {
        panic!("a session couldn't come back");
    };
    block_on(daemon.mark_read(id)).expect("it opens");
    let workspace = daemon.workspace();
    let Some(session) = workspace.session(id) else {
        panic!("{id} is in the workspace");
    };
    assert_eq!(session.unread, 0);
    assert!(matches!(session.recovery, Some(Recovery::Recoverable { .. })));
}
