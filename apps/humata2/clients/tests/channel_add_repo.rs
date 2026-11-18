//! CH-6, CH-18, CH-19: adding a GitHub repo to a channel on the fake daemon,
//! and reading a pasted GitHub address.

use std::sync::Arc;

use humata2_clients::github::{RepoTextError, parse_repo_text};
use humata2_clients::model::ChannelId;
use humata2_clients::repos::RepoPlace;
use humata2_clients::{Daemon, DaemonError, FakeDaemon, Operation, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> (FakeDaemon, ChannelId) {
    let workspace = fixture::design();
    let Some(channel) = workspace.channels.first().map(|channel| channel.id.clone()) else {
        panic!("the design workspace has a channel");
    };
    (FakeDaemon::new(workspace, Arc::new(TestClock::new())), channel)
}

fn repos_of(daemon: &FakeDaemon, channel: &ChannelId) -> Vec<String> {
    let loaded = block_on(daemon.load()).expect("the workspace loads");
    loaded
        .channel(channel)
        .map(|found| found.repos.clone())
        .unwrap_or_default()
}

#[test]
fn an_added_repo_is_in_the_channel_and_in_the_library_used_there() {
    let (daemon, channel) = daemon();
    block_on(daemon.add_repo_to_channel(&channel, "acme/newrepo")).expect("the repo is added");
    assert!(repos_of(&daemon, &channel).contains(&"acme/newrepo".to_owned()));
    let library = block_on(daemon.repo_library()).expect("the library lists");
    let held = library
        .iter()
        .find(|repo| repo.repo == "acme/newrepo")
        .expect("the add is recorded");
    assert_eq!(held.last_used_in, RepoPlace::Channel(channel.to_string()));
}

#[test]
fn adding_a_repo_the_channel_has_changes_nothing_in_the_channel() {
    let (daemon, channel) = daemon();
    block_on(daemon.add_repo_to_channel(&channel, "acme/newrepo")).expect("the first add");
    let before = repos_of(&daemon, &channel);
    block_on(daemon.add_repo_to_channel(&channel, "acme/newrepo")).expect("the second add");
    assert_eq!(repos_of(&daemon, &channel), before);
}

#[test]
fn what_is_not_owner_slash_name_is_refused_and_changes_nothing() {
    let (daemon, channel) = daemon();
    let before = repos_of(&daemon, &channel);
    for text in ["", "acme", "/etc/passwd", "~/code/api", "../api", "a/b/c", "https://github.com/acme/api"] {
        let answer = block_on(daemon.add_repo_to_channel(&channel, text));
        assert!(
            matches!(answer, Err(DaemonError::Refused { .. })),
            "{text:?} is refused: {answer:?}"
        );
    }
    assert_eq!(repos_of(&daemon, &channel), before);
}

#[test]
fn a_channel_that_is_gone_is_not_found() {
    let (daemon, _) = daemon();
    let answer = block_on(daemon.add_repo_to_channel(&ChannelId::new("nope"), "acme/api"));
    assert!(matches!(answer, Err(DaemonError::NotFound { .. })), "{answer:?}");
}

#[test]
fn a_failed_add_changes_nothing_and_the_next_one_works() {
    let (daemon, channel) = daemon();
    let before = repos_of(&daemon, &channel);
    daemon.fail_next(Operation::AddRepoToChannel, "the clone failed");
    assert!(block_on(daemon.add_repo_to_channel(&channel, "acme/newrepo")).is_err());
    assert_eq!(repos_of(&daemon, &channel), before);
    block_on(daemon.add_repo_to_channel(&channel, "acme/newrepo")).expect("the retry works");
    assert!(repos_of(&daemon, &channel).contains(&"acme/newrepo".to_owned()));
}

#[test]
fn a_repo_is_read_from_owner_name_or_a_github_address() {
    for text in [
        "acme/api",
        "  acme/api  ",
        "acme/api/",
        "acme/api.git",
        "https://github.com/acme/api",
        "https://github.com/acme/api/",
        "https://github.com/acme/api.git",
        "http://github.com/acme/api",
        "https://www.github.com/acme/api",
        "https://GitHub.com/acme/api",
        "github.com/acme/api",
        "git@github.com:acme/api.git",
        "ssh://git@github.com/acme/api.git",
        "https://github.com/acme/api/tree/main/src",
        "https://github.com/acme/api?tab=readme#top",
    ] {
        assert_eq!(parse_repo_text(text).as_deref(), Ok("acme/api"), "{text:?}");
    }
}

#[test]
fn a_name_with_dots_dashes_and_underscores_is_kept() {
    assert_eq!(parse_repo_text("a-b_c/d.e-f_g").as_deref(), Ok("a-b_c/d.e-f_g"));
}

#[test]
fn text_that_is_not_a_repo_is_refused() {
    assert_eq!(parse_repo_text("  "), Err(RepoTextError::Empty));
    for text in [
        "acme",
        "acme/",
        "/api",
        "/etc/passwd",
        "~/code/api",
        "~acme/api",
        "acme/api/extra",
        "../api",
        "acme/..",
        "acme/ api",
        "acme/ap!",
        "https://github.com/acme",
        "https://github.com/",
        "https://example.com/acme/api",
        "C:\\code\\api",
    ] {
        assert!(
            matches!(parse_repo_text(text), Err(RepoTextError::NotARepo { .. })),
            "{text:?}"
        );
    }
}
