//! DESKWATCH: a Desk event from the daemon changes the Desk in place, with
//! no reload.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState};
use humata2_clients::desk::{Closed, DeskChange};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Clients, Event, Fakes, Workspace, fixture};
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn launch(workspace: Workspace) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    (store, fakes)
}

fn loads(fakes: &Fakes) -> usize {
    fakes.daemon.calls().iter().filter(|call| *call == "load").count()
}

#[test]
fn a_channel_desk_upsert_changes_the_row_and_a_removal_drops_it_with_no_load() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (mut store, fakes) = launch(workspace);
    let triage = SessionId::new("cdesk-triage");
    let backfill = SessionId::new("cdesk-backfill");
    let before = loads(&fakes);

    let mut changed = store.state().workspace.desk.channel_sessions[&triage].clone();
    changed.desk.closed = Some(Closed {
        when: "now".to_owned(),
        days_left: None,
    });
    store.send(
        A::Event(Event::Desk(DeskChange::ChannelSession(changed))),
        |_| {},
    );
    store.settle();
    assert!(store.state().workspace.desk.channel_sessions[&triage].desk.closed.is_some());

    store.send(A::Event(Event::Desk(DeskChange::Gone(backfill.clone()))), |_| {});
    store.settle();
    let state = store.state();
    assert!(!state.workspace.desk.channel_sessions.contains_key(&backfill));
    assert!(state.workspace.desk.channel_sessions.contains_key(&triage));
    assert!(
        state.desk.channel_rows(&payments()).iter().all(|row| row.session.id != backfill),
        "the Desk tab no longer lists it"
    );
    assert_eq!(loads(&fakes), before, "the events were enough: no load");
}

#[test]
fn a_plain_desk_upsert_replaces_the_session_and_its_files_in_place() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (mut store, fakes) = launch(workspace);
    let id = SessionId::new("desk-release-checklist");
    let before = loads(&fakes);
    let others = store
        .state()
        .workspace
        .desk
        .documents
        .iter()
        .filter(|document| document.session != id)
        .count();

    let mut desk = store.state().workspace.desk.sessions[&id].clone();
    desk.closed = None;
    store.send(
        A::Event(Event::Desk(DeskChange::Session {
            desk,
            documents: vec!["new-note.md".to_owned()],
        })),
        |_| {},
    );
    store.settle();

    let desk = &store.state().workspace.desk;
    assert!(desk.sessions[&id].closed.is_none(), "it is open now");
    let mine: Vec<_> = desk
        .documents
        .iter()
        .filter(|document| document.session == id)
        .map(|document| document.path.as_str())
        .collect();
    assert_eq!(mine, ["new-note.md"]);
    assert_eq!(
        desk.documents.iter().filter(|document| document.session != id).count(),
        others,
        "other sessions' files are untouched"
    );
    assert_eq!(loads(&fakes), before, "no load");
}

#[test]
fn a_removal_of_a_session_the_app_never_had_changes_nothing() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (mut store, _fakes) = launch(workspace);
    let before = store.state().workspace.desk.clone();
    store.send(
        A::Event(Event::Desk(DeskChange::Gone(SessionId::new("never-listed")))),
        |_| {},
    );
    store.settle();
    assert_eq!(store.state().workspace.desk, before);
}
