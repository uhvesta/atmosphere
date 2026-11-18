//! RL-8: the owners **+ Org** suggests are the library's owners first, then
//! the orgs the user's GitHub login belongs to, with none twice and none the
//! scope already lists. The fake daemon lists the orgs and tells the time.

use std::sync::Arc;

use humata2_clients::repos::{LibraryRepo, RepoPlace, suggest_owners};
use humata2_clients::{Daemon, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn held(repo: &str) -> LibraryRepo {
    LibraryRepo {
        repo: repo.to_owned(),
        remote: format!("git@github.com:{repo}.git"),
        default_branch: "main".to_owned(),
        added_at_ms: 1,
        last_used_at_ms: 1,
        last_used_in: RepoPlace::Desk,
    }
}

#[test]
fn library_owners_come_first_then_github_orgs_without_repeats() {
    let library = [held("acme/api"), held("octo/notes"), held("acme/web")];
    let github = strings(&["bazelbuild", "ACME", "octo", "zeta"]);
    assert_eq!(
        suggest_owners(&library, &github, &[]),
        strings(&["acme", "octo", "bazelbuild", "zeta"])
    );
}

#[test]
fn owners_already_in_the_scope_are_left_out_whatever_their_case() {
    let library = [held("acme/api"), held("octo/notes")];
    let github = strings(&["bazelbuild", "Acme"]);
    assert_eq!(
        suggest_owners(&library, &github, &strings(&["ACME", "bazelbuild"])),
        strings(&["octo"])
    );
}

#[test]
fn nothing_to_suggest_is_an_empty_list() {
    assert!(suggest_owners(&[], &[], &[]).is_empty());
    assert!(suggest_owners(&[held("acme/api")], &[], &strings(&["acme"])).is_empty());
}

#[test]
fn the_fake_daemon_lists_the_orgs_it_was_given() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    assert!(block_on(daemon.repo_owners()).is_ok_and(|owners| owners.is_empty()));
    daemon.set_repo_owners(strings(&["bazelbuild", "acme"]));
    assert_eq!(
        block_on(daemon.repo_owners()).ok(),
        Some(strings(&["bazelbuild", "acme"]))
    );
}

#[test]
fn the_time_a_search_reports_can_be_fixed_by_a_test() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    daemon.set_repo_now_ms(9_000);
    assert_eq!(daemon.now_ms(), 9_000);
}
