//! DESKWATCH: the fake daemon tells the Desk's changes as Desk events, from
//! whichever path changes them, so a client needs no reload.

use std::sync::Arc;

use humata2_clients::desk::DeskChange;
use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::{Daemon, DaemonError, DeskClient, Event, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;
use tokio::sync::mpsc::Receiver;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> FakeDaemon {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    // A move lands under the target's own root (PL-1); this Mac keeps its
    // Humata folder elsewhere, so a move here changes the Desk row's path.
    for machine in &mut workspace.machines {
        if machine.id.as_str() == "this-mac" {
            machine.root = "~/Work".to_owned();
        }
    }
    FakeDaemon::new(workspace, Arc::new(TestClock::new()))
}

fn desk_changes(events: &mut Receiver<Result<Event, DaemonError>>) -> Vec<DeskChange> {
    let mut found = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Ok(Event::Desk(change)) = event {
            found.push(change);
        }
    }
    found
}

#[test]
fn moving_a_session_that_has_a_desk_row_tells_its_new_folder() {
    let daemon = daemon();
    let id = SessionId::new("desk-brin-rfc");
    let before = daemon.workspace().desk.sessions[&id].folder.path().to_owned();
    let mut events = block_on(daemon.events()).expect("events open");

    block_on(daemon.move_to(&id, &MachineId::new("this-mac"))).expect("the session moves");

    let told: Vec<_> = desk_changes(&mut events)
        .into_iter()
        .filter_map(|change| match change {
            DeskChange::Session { desk, .. } if desk.session == id => Some(desk),
            _ => None,
        })
        .collect();
    assert_eq!(told.len(), 1, "one Desk event for the move");
    assert_ne!(told[0].folder.path(), before, "the folder is under the new machine's root");
}

#[test]
fn a_change_behind_the_back_tells_only_what_changed() {
    let daemon = daemon();
    let id = SessionId::new("desk-release-checklist");
    let mut events = block_on(daemon.events()).expect("events open");

    daemon.change_desk_elsewhere(|desk| {
        if let Some(found) = desk.sessions.get_mut(&id) {
            found.closed = None;
        }
    });

    let told = desk_changes(&mut events);
    assert_eq!(told.len(), 1);
    assert!(matches!(&told[0], DeskChange::Session { desk, .. } if desk.session == id && desk.closed.is_none()));
    assert!(events.try_recv().is_err(), "no reload was asked for");
}

#[test]
fn removing_a_session_tells_it_gone() {
    let daemon = daemon();
    let id = SessionId::new("desk-release-checklist");
    let mut events = block_on(daemon.events()).expect("events open");

    daemon.change_desk_elsewhere(|desk| {
        desk.sessions.remove(&id);
    });

    assert_eq!(desk_changes(&mut events), vec![DeskChange::Gone(id)]);
}
