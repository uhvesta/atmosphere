//! SE-10/16: terminal bytes keep their ownership and replay offsets.
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, Workspace};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn replayed_and_overlapping_output_is_rendered_once_in_its_own_tab() {
    let (clients, _) = Clients::fake(Workspace::default(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SessionState::default(), Sessions::new(clients));
    let id = SessionId::new("session");
    let key = (id.clone(), 2);
    store.send(
        A::TerminalData {
            session: id.clone(),
            tab: 2,
            offset: 0,
            bytes: b"one".to_vec(),
        },
        |s| {
            s.terminal_bytes
                .insert(key.clone(), Arc::new(b"one".to_vec()));
        },
    );
    store.send(
        A::TerminalData {
            session: id.clone(),
            tab: 2,
            offset: 0,
            bytes: b"one".to_vec(),
        },
        |_| {},
    );
    store.send(
        A::TerminalData {
            session: id,
            tab: 2,
            offset: 2,
            bytes: b"e\r\ntwo".to_vec(),
        },
        |s| {
            s.terminal_bytes
                .insert(key.clone(), Arc::new(b"one\r\ntwo".to_vec()));
        },
    );
    assert_eq!(store.state().terminal_bytes.len(), 1);
    // A delayed full workspace snapshot cannot roll back newer streamed output.
    let mut workspace = Workspace::default();
    workspace.terminal_bytes.insert(key, b"one".to_vec());
    let workspace = Arc::new(workspace);
    store.send(A::Show(workspace.clone()), |s| s.workspace = workspace);
    store.finish();
}
