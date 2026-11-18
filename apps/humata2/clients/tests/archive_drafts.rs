//! D513: archiving a review keeps its pull request drafts never submitted;
//! a restore brings them back.

use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Daemon, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

#[test]
fn archived_reviews_keep_their_unsubmitted_drafts_through_a_restore() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    daemon.set_unsubmitted_drafts(&id, 2);
    block_on(daemon.archive(&id, false)).expect("archive");
    block_on(daemon.restore_session(&id)).expect("restore");
    let plan = block_on(daemon.plan_archive(&id)).expect("plan");
    assert_eq!(plan.unsubmitted_drafts, 2);
}
