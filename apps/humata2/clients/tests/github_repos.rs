//! CH-3: the repos `gh` sees, as the fake daemon lists them, and the fuzzy
//! rule the New channel sheet narrows them with.

use std::sync::Arc;

use humata2_clients::github::matching;
use humata2_clients::{Daemon, DaemonError, FakeDaemon, Operation, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn repos(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn daemon() -> FakeDaemon {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    daemon.set_github_repos(repos(&["acme/ledger", "acme/api", "octo/notes"]));
    daemon
}

#[test]
fn an_empty_query_lists_every_repo_in_ghs_order() {
    let listed = block_on(daemon().github_repos("")).expect("the repos are listed");
    assert_eq!(listed, repos(&["acme/ledger", "acme/api", "octo/notes"]));
}

#[test]
fn a_query_lists_only_the_repos_it_matches() {
    let listed = block_on(daemon().github_repos("acme")).expect("the repos are listed");
    assert_eq!(listed, repos(&["acme/ledger", "acme/api"]));
}

#[test]
fn a_scripted_failure_reaches_the_caller() {
    let daemon = daemon();
    daemon.fail_next(Operation::GitHubRepos, "gh is not signed in");
    let failed = block_on(daemon.github_repos(""));
    assert!(matches!(failed, Err(DaemonError::Scripted { .. })));
    assert!(block_on(daemon.github_repos("")).is_ok(), "only the next call fails");
}

#[test]
fn matching_ignores_case_and_allows_gaps() {
    let all = repos(&["acme/ledger", "acme/api", "octo/notes"]);
    assert_eq!(matching("LDGR", &all), repos(&["acme/ledger"]));
    assert_eq!(matching("", &all), all);
    assert!(matching("zzz", &all).is_empty());
}

#[test]
fn matching_puts_whole_matches_before_scattered_ones() {
    let all = repos(&["acme/docs", "octo/notes"]);
    assert_eq!(matching("es", &all), repos(&["octo/notes", "acme/docs"]));
}
