//! The fake keeps a session's looks (D852) the way the host does: saving
//! replaces the whole set, reading returns it, an empty save forgets it.

use std::sync::Arc;

use humata2_clients::model::{LookedFile, SessionId};
use humata2_clients::{Daemon, DaemonError, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> FakeDaemon {
    FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()))
}

fn look(repo: &str, path: &str, fingerprint: u64) -> LookedFile {
    LookedFile {
        repo: repo.to_owned(),
        path: path.to_owned(),
        fingerprint,
    }
}

#[test]
fn saved_looks_read_back_and_a_new_save_replaces_them() {
    let daemon = daemon();
    let session = SessionId::new("s-auth");
    let first = vec![look("acme/api", "a.rs", 1), look("acme/web", "b.ts", u64::MAX)];
    block_on(daemon.save_looks(&session, first.clone())).expect("saved");
    assert_eq!(block_on(daemon.looks(&session)).expect("read"), first);
    let second = vec![look("acme/api", "a.rs", 2)];
    block_on(daemon.save_looks(&session, second.clone())).expect("saved");
    assert_eq!(block_on(daemon.looks(&session)).expect("read"), second);
    block_on(daemon.save_looks(&session, Vec::new())).expect("cleared");
    assert!(block_on(daemon.looks(&session)).expect("read").is_empty());
}

#[test]
fn a_session_with_no_saved_looks_reads_empty_and_an_unknown_one_is_refused() {
    let daemon = daemon();
    assert!(block_on(daemon.looks(&SessionId::new("s-auth"))).expect("read").is_empty());
    assert!(matches!(
        block_on(daemon.looks(&SessionId::new("s-nope"))),
        Err(DaemonError::NotFound { .. })
    ));
}
