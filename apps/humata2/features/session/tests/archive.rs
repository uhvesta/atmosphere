//! A failed archive keeps the session open and never reports success.
use humata2_clients::{Clients, DaemonError, Operation, fixture, model::SessionId};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn failed_archive_keeps_the_open_session() {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    fakes
        .daemon
        .fail_next(Operation::Archive, "link unavailable");
    let id = SessionId::new("ledger");
    let state = SessionState {
        workspace,
        open: Some(id.clone()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients));
    store.send(A::Archive, |_| {});
    store.receive_matching(
        "the plan",
        |a| matches!(a, A::ArchivePlanned { result: Ok(_), .. }),
        |_| {},
    );
    let error = DaemonError::Scripted {
        operation: Operation::Archive,
        message: "link unavailable".into(),
    };
    store.receive(
        A::Archived {
            session: id.clone(),
            title: "Ledger schema migration".into(),
            result: Err(error.clone()),
        },
        |s| s.error = Some(error.to_string()),
    );
    assert_eq!(store.state().open, Some(id.clone()));
    assert!(!fakes.daemon.workspace().session(&id).unwrap().archived);
    store.finish();
}
