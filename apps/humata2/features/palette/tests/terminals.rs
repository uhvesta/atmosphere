//! D784: Search (Command-K) lists every running terminal, hidden ones
//! included, and stopped sessions: a session's shell tabs are entries of
//! their own that open that tab, and a stopped session is still found.
use humata2_clients::{
    fixture,
    model::{SessionId, Status},
};
use humata2_palette::{Command, Group, Palette, PaletteAction as A, PaletteState};
use std::sync::Arc;
use tca::TestStore;

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn store(status: Status) -> TestStore<Palette> {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        if session.id == ledger() {
            session.title = "Ledger".into();
            session.shells = vec!["zsh".into(), "build-watch".into()];
            session.terminal_agents.clear();
            session.status = status;
        }
    }
    let state = PaletteState {
        workspace: Arc::new(workspace),
        open: true,
        ..Default::default()
    };
    TestStore::new(state, Palette).non_exhaustive()
}

fn query(store: &mut TestStore<Palette>, text: &str) {
    store.send(A::Query(text.into()), |_| {});
}

#[test]
fn each_shell_tab_of_a_session_is_found_and_opens_that_tab() {
    let mut store = store(Status::Idle);
    query(&mut store, "build-watch");
    let state = store.state();
    let found = state
        .entries
        .iter()
        .find(|entry| matches!(&entry.command, Command::Terminal { session, tab: 2 } if *session == ledger()));
    let Some(found) = found else {
        panic!("the second shell tab is listed: {:?}", state.entries);
    };
    assert_eq!(state.group(found), Group::Sessions);
}

#[test]
fn the_sessions_shells_are_found_by_the_sessions_title() {
    let mut store = store(Status::Idle);
    query(&mut store, "Ledger");
    let tabs: Vec<usize> = store
        .state()
        .entries
        .iter()
        .filter_map(|entry| match &entry.command {
            Command::Terminal { session, tab } if *session == ledger() => Some(*tab),
            _ => None,
        })
        .collect();
    assert_eq!(tabs, vec![1, 2]);
}

#[test]
fn a_stopped_session_is_still_found() {
    let mut store = store(Status::Stopped);
    query(&mut store, "Ledger");
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|entry| entry.command == Command::Session(ledger()))
    );
}

#[test]
fn a_terminal_is_not_found_through_its_sessions_status() {
    let mut store = store(Status::Working);
    // Only "working" holds these letters in this order.
    query(&mut store, "orki");
    assert!(
        !store
            .state()
            .entries
            .iter()
            .any(|entry| matches!(entry.command, Command::Terminal { .. })),
        "a terminal matched on its status: {:?}",
        store.state().entries
    );
}

#[test]
fn a_sessions_terminals_are_listed_below_the_session() {
    let mut store = store(Status::Working);
    query(&mut store, "Ledger");
    let entries = &store.state().entries;
    let at = |wanted: &dyn Fn(&Command) -> bool| entries.iter().position(|entry| wanted(&entry.command));
    let session = at(&|command| *command == Command::Session(ledger()));
    let shell = at(&|command| matches!(command, Command::Terminal { session, .. } if *session == ledger()));
    let (Some(session), Some(shell)) = (session, shell) else {
        panic!("both are listed: {entries:?}");
    };
    assert!(session < shell, "the shell came first: {entries:?}");
}

#[test]
fn a_terminal_names_the_machine_it_runs_on() {
    let mut store = store(Status::Working);
    query(&mut store, "build-watch");
    let state = store.state();
    let machine = state
        .workspace
        .session(&ledger())
        .and_then(|session| state.workspace.machines.iter().find(|machine| machine.id == session.machine))
        .map(|machine| machine.name.clone())
        .expect("the ledger's machine is in the workspace");
    let found = state
        .entries
        .iter()
        .find(|entry| matches!(&entry.command, Command::Terminal { session, tab: 2 } if *session == ledger()))
        .expect("the shell tab is listed");
    assert_eq!(found.hint, machine);
}
