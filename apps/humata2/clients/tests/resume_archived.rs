//! D784: the daemon refuses to resume a session that was ended for good.

use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Daemon, DaemonError, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

#[test]
fn resuming_an_archived_session_is_refused_and_leaves_it_archived() {
    let mut workspace = fixture::design();
    let id = SessionId::new("ledger");
    let Some(session) = workspace.sessions.iter_mut().find(|s| s.id == id) else {
        panic!("the design fixture has ledger");
    };
    session.archived = true;
    let before = session.status;
    let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
    let result = block_on(daemon.resume(&id));
    assert!(matches!(result, Err(DaemonError::Refused { .. })), "got {result:?}");
    let after = daemon.workspace();
    let Some(found) = after.session(&id) else { panic!("still there") };
    assert!(found.archived);
    assert_eq!(found.status, before);
}
