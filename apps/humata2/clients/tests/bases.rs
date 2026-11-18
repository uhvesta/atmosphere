//! The branches one repository can be compared with, on the fake (VIEWS C3):
//! a repository the session has lists its branches with the default branch
//! first, a repository with no branches of its own lists main only, and a
//! repository the session doesn't have is refused.

use std::sync::Arc;

use humata2_clients::model::{RepoBases, SessionId};
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

fn bases(daemon: &FakeDaemon, session: &str, repo: &str) -> Result<RepoBases, DaemonError> {
    block_on(daemon.bases(&SessionId::new(session), repo))
}

#[test]
fn a_repository_lists_its_branches_with_the_default_branch_first() {
    let daemon = daemon();
    let listed = match bases(&daemon, "s-auth", "acme/api") {
        Ok(listed) => listed,
        Err(error) => panic!("the branches are listed: {error}"),
    };
    assert_eq!(listed.default_branch, "main");
    assert_eq!(
        listed.branches,
        ["main", "release/2.4", "release/2.3", "feat/ledger-v2"]
    );
}

#[test]
fn a_repository_with_no_branches_of_its_own_lists_main_only() {
    let daemon = daemon();
    let listed = match bases(&daemon, "s-passkey", "acme/web") {
        Ok(listed) => listed,
        Err(error) => panic!("the branches are listed: {error}"),
    };
    assert_eq!(listed.default_branch, "main");
    assert_eq!(listed.branches, ["main"]);
}

#[test]
fn a_repository_the_session_does_not_have_is_refused() {
    let daemon = daemon();
    let result = bases(&daemon, "s-auth", "acme/web");
    assert!(matches!(result, Err(DaemonError::Refused { .. })));
}
