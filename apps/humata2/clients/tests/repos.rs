//! RL-1 to RL-12: the repo library and the one repo search, on the fake
//! daemon.

use std::sync::Arc;

use humata2_clients::repos::{
    LibraryRepo, RepoPlace, RepoScope, RepoSearchProblem, RepoSearchProblemKind,
};
use humata2_clients::{Daemon, DaemonError, FakeDaemon, fixture};
use humata2_clients::desk;
use humata2_clients::model::SessionId;
use humata2_clients::{DeskClient, Operation};
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

fn held(repo: &str, last_used_at_ms: i64) -> LibraryRepo {
    LibraryRepo {
        repo: repo.to_owned(),
        remote: format!("git@github.com:{repo}.git"),
        default_branch: "main".to_owned(),
        added_at_ms: 1,
        last_used_at_ms,
        last_used_in: RepoPlace::Desk,
    }
}

fn owners(names: &[&str], include_me: bool) -> RepoScope {
    RepoScope::Owners { owners: strings(names), include_me }
}

fn daemon() -> FakeDaemon {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    daemon.set_github_repos(strings(&[
        "acme/ledger",
        "acme/api",
        "octo/notes",
        "you/dotfiles",
        "other/ledger-tools",
    ]));
    daemon
}

fn library_names(search: &humata2_clients::repos::RepoSearch) -> Vec<String> {
    search.library.iter().map(|repo| repo.repo.clone()).collect()
}

fn github_names(search: &humata2_clients::repos::RepoSearch) -> Vec<String> {
    search.github.iter().map(|repo| repo.repo.clone()).collect()
}

#[test]
fn the_library_lists_most_recently_used_first() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/old", 10), held("acme/new", 30), held("acme/mid", 20)]);
    let listed = block_on(daemon.repo_library()).expect("the library is listed");
    let names: Vec<_> = listed.iter().map(|repo| repo.repo.as_str()).collect();
    assert_eq!(names, ["acme/new", "acme/mid", "acme/old"]);
}

#[test]
fn the_library_group_matches_whole_text_before_scattered_letters_ignoring_case() {
    let daemon = daemon();
    daemon.set_repo_library(vec![
        held("acme/ledger", 30),
        held("acme/lodge-red", 20),
        held("acme/LEDGER-two", 10),
        held("acme/api", 40),
    ]);
    let found = block_on(daemon.search_repos("ledger", None)).expect("searched");
    assert_eq!(library_names(&found), ["acme/ledger", "acme/LEDGER-two"]);

    let found = block_on(daemon.search_repos("ldgr", None)).expect("searched");
    assert_eq!(library_names(&found), ["acme/ledger", "acme/lodge-red", "acme/LEDGER-two"]);
}

#[test]
fn within_a_tier_the_library_group_is_most_recently_used_first() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/a-notes", 10), held("acme/b-notes", 20)]);
    let found = block_on(daemon.search_repos("notes", None)).expect("searched");
    assert_eq!(library_names(&found), ["acme/b-notes", "acme/a-notes"]);
}

#[test]
fn the_scope_never_filters_the_library_group() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("elsewhere/thing", 5)]);
    let found = block_on(daemon.search_repos("thing", Some(owners(&["acme"], false)))).expect("searched");
    assert_eq!(library_names(&found), ["elsewhere/thing"]);
}

#[test]
fn your_repos_searches_the_whole_catalogue() {
    let found = block_on(daemon().search_repos("ledger", None)).expect("searched");
    assert_eq!(github_names(&found), ["acme/ledger", "other/ledger-tools"]);
    assert_eq!(found.scope, RepoScope::YourRepos);
    assert_eq!(found.problem, None);
}

#[test]
fn owners_narrow_the_github_group_and_include_me_adds_the_login() {
    let daemon = daemon();
    let found = block_on(daemon.search_repos("le", Some(owners(&["ACME"], false)))).expect("searched");
    assert_eq!(github_names(&found), ["acme/ledger"]);

    let found = block_on(daemon.search_repos("do", Some(owners(&["acme"], true)))).expect("searched");
    assert_eq!(github_names(&found), ["you/dotfiles"]);

    daemon.set_repo_login("octo");
    let found = block_on(daemon.search_repos("no", Some(owners(&[], true)))).expect("searched");
    assert_eq!(github_names(&found), ["octo/notes"]);
}

#[test]
fn all_of_github_searches_every_owner() {
    let found = block_on(daemon().search_repos("ledger", Some(RepoScope::AllOfGitHub))).expect("searched");
    assert_eq!(github_names(&found), ["acme/ledger", "other/ledger-tools"]);
    assert_eq!(found.scope, RepoScope::AllOfGitHub);
}

#[test]
fn a_repo_in_the_library_group_is_dropped_from_the_github_group() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("ACME/Ledger", 1)]);
    let found = block_on(daemon.search_repos("ledger", None)).expect("searched");
    assert_eq!(library_names(&found), ["ACME/Ledger"]);
    assert_eq!(github_names(&found), ["other/ledger-tools"]);
}

#[test]
fn a_short_query_is_a_too_short_problem_and_no_github_group() {
    let daemon = daemon();
    let kind = |found: &humata2_clients::repos::RepoSearch| found.problem.as_ref().map(|problem| problem.kind);

    let found = block_on(daemon.search_repos("a", Some(owners(&["acme"], false)))).expect("searched");
    assert_eq!(kind(&found), Some(RepoSearchProblemKind::TooShort));
    assert!(found.github.is_empty());

    let found = block_on(daemon.search_repos("le", Some(owners(&["acme"], false)))).expect("searched");
    assert_eq!(kind(&found), None);

    let found = block_on(daemon.search_repos("le", Some(RepoScope::AllOfGitHub))).expect("searched");
    assert_eq!(kind(&found), Some(RepoSearchProblemKind::TooShort));
    assert!(found.github.is_empty());

    let found = block_on(daemon.search_repos("led", Some(RepoScope::AllOfGitHub))).expect("searched");
    assert_eq!(kind(&found), None);
    assert!(!found.github.is_empty());
}

#[test]
fn a_short_query_still_answers_the_library_group() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/ax", 1)]);
    let found = block_on(daemon.search_repos("a", Some(RepoScope::AllOfGitHub))).expect("searched");
    assert_eq!(library_names(&found), ["acme/ax"]);
}

#[test]
fn a_github_problem_empties_only_the_github_group() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/ledger", 1)]);
    let problem = RepoSearchProblem {
        kind: RepoSearchProblemKind::RateLimited { retry_at_ms: Some(9_000) },
        message: "wait".to_owned(),
    };
    daemon.set_repo_search_problem(Some(problem.clone()));
    let found = block_on(daemon.search_repos("ledger", None)).expect("the call itself works");
    assert_eq!(library_names(&found), ["acme/ledger"]);
    assert!(found.github.is_empty());
    assert_eq!(found.problem, Some(problem));

    daemon.set_repo_search_problem(None);
    let found = block_on(daemon.search_repos("ledger", None)).expect("searched");
    assert_eq!(found.problem, None);
}

#[test]
fn removing_a_repo_drops_it_and_an_unknown_one_is_not_an_error() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/a", 2), held("acme/b", 1)]);
    let left = block_on(daemon.remove_from_repo_library("ACME/a")).expect("removed");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].repo, "acme/b");
    let left = block_on(daemon.remove_from_repo_library("nobody/nothing")).expect("not an error");
    assert_eq!(left.len(), 1);
}

#[test]
fn forgetting_clears_the_library_and_counts_what_it_held() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/a", 2), held("acme/b", 1)]);
    assert_eq!(block_on(daemon.forget_repo_library()).expect("forgotten"), 2);
    assert!(block_on(daemon.repo_library()).expect("listed").is_empty());
    assert_eq!(block_on(daemon.forget_repo_library()).expect("forgotten"), 0);
}

#[test]
fn the_saved_scope_starts_as_your_repos_and_is_what_a_search_uses() {
    let daemon = daemon();
    assert_eq!(block_on(daemon.repo_scope()).expect("read"), RepoScope::YourRepos);
    let saved = block_on(daemon.save_repo_scope(owners(&["octo"], false))).expect("saved");
    assert_eq!(saved, owners(&["octo"], false));
    let found = block_on(daemon.search_repos("notes", None)).expect("searched");
    assert_eq!(found.scope, owners(&["octo"], false));
    assert_eq!(github_names(&found), ["octo/notes"]);
}

#[test]
fn a_per_search_scope_does_not_change_the_saved_one() {
    let daemon = daemon();
    block_on(daemon.save_repo_scope(owners(&["octo"], false))).expect("saved");
    block_on(daemon.search_repos("ledger", Some(RepoScope::AllOfGitHub))).expect("searched");
    assert_eq!(block_on(daemon.repo_scope()).expect("read"), owners(&["octo"], false));
}

#[test]
fn all_of_github_cannot_be_saved() {
    let daemon = daemon();
    let refused = block_on(daemon.save_repo_scope(RepoScope::AllOfGitHub));
    assert!(matches!(refused, Err(DaemonError::Refused { .. })));
    assert_eq!(block_on(daemon.repo_scope()).expect("read"), RepoScope::YourRepos);
}

#[test]
fn unknown_owners_in_the_scope_are_reported() {
    let daemon = daemon();
    daemon.set_unknown_owners(strings(&["Ghost"]));
    let found =
        block_on(daemon.search_repos("le", Some(owners(&["acme", "ghost"], false)))).expect("searched");
    assert_eq!(found.unknown_owners, ["ghost"]);
    let found = block_on(daemon.search_repos("le", Some(owners(&["acme"], false)))).expect("searched");
    assert!(found.unknown_owners.is_empty());
}

#[test]
fn a_scripted_search_failure_fails_only_the_next_search() {
    let daemon = daemon();
    daemon.fail_next(Operation::SearchRepos, "gh is busy");
    assert!(block_on(daemon.search_repos("ledger", None)).is_err());
    assert!(block_on(daemon.search_repos("ledger", None)).is_ok());
}

#[test]
fn a_channel_made_with_a_repo_is_first_in_the_library() {
    let daemon = daemon();
    daemon.set_repo_library(vec![held("acme/old", 10)]);
    let channel = block_on(daemon.create_channel("ledger work", strings(&["acme/ledger"]), Vec::new()))
        .expect("the channel is made");
    let listed = block_on(daemon.repo_library()).expect("the library is listed");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].repo, "acme/ledger");
    assert_eq!(listed[0].last_used_in, RepoPlace::Channel(channel.to_string()));
}

fn with_desk() -> FakeDaemon {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    FakeDaemon::new(workspace, Arc::new(TestClock::new()))
}

#[test]
fn a_repo_added_on_the_desk_is_recorded_as_used_there() {
    let daemon = with_desk();
    block_on(daemon.add_repo(&SessionId::new("desk-plan-q4"), "acme/web")).expect("it is added");
    let listed = block_on(daemon.repo_library()).expect("the library is listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].repo, "acme/web");
    assert_eq!(listed[0].last_used_in, RepoPlace::Desk);
}

#[test]
fn a_failed_desk_add_records_nothing() {
    let daemon = with_desk();
    daemon.fail_next(Operation::Desk(desk::Operation::AddRepo), "disk is full");
    assert!(block_on(daemon.add_repo(&SessionId::new("desk-plan-q4"), "acme/web")).is_err());
    assert!(block_on(daemon.repo_library()).expect("the library is listed").is_empty());
}
