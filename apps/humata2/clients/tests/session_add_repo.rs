//! CH-21: adding a repo to a running session on the fake daemon.

use std::sync::Arc;

use humata2_clients::model::{ChannelId, Session, SessionId};
use humata2_clients::repos::RepoPlace;
use humata2_clients::{Daemon, DaemonError, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> (FakeDaemon, SessionId) {
    let workspace = fixture::design();
    let Some(session) = workspace.sessions.first().map(|session| session.id.clone()) else {
        panic!("the design workspace has a session");
    };
    (FakeDaemon::new(workspace, Arc::new(TestClock::new())), session)
}

fn session_of(daemon: &FakeDaemon, id: &SessionId) -> Session {
    let loaded = block_on(daemon.load()).expect("the workspace loads");
    match loaded.sessions.iter().find(|session| &session.id == id) {
        Some(session) => session.clone(),
        None => panic!("the session {id} is there"),
    }
}

fn channel_repos(daemon: &FakeDaemon, channel: &ChannelId) -> Vec<String> {
    let loaded = block_on(daemon.load()).expect("the workspace loads");
    loaded
        .channel(channel)
        .map(|found| found.repos.clone())
        .unwrap_or_default()
}

#[test]
fn an_added_repo_is_cloned_in_the_session_on_its_branch_and_joins_the_channel_and_library() {
    let (daemon, id) = daemon();
    let before = session_of(&daemon, &id);

    block_on(daemon.add_repo_to_session(&id, "acme/newrepo")).expect("the repo is added");

    let after = session_of(&daemon, &id);
    let clone = after
        .clones
        .iter()
        .find(|clone| clone.repo == "acme/newrepo")
        .expect("the session has a clone of it");
    assert_eq!(clone.branch, before.branch);
    assert!(channel_repos(&daemon, &after.channel).contains(&"acme/newrepo".to_owned()));
    let library = block_on(daemon.repo_library()).expect("the library lists");
    let held = library
        .iter()
        .find(|repo| repo.repo == "acme/newrepo")
        .expect("the add is recorded");
    assert_eq!(held.last_used_in, RepoPlace::Channel(after.channel.to_string()));
}

#[test]
fn adding_a_repo_the_session_has_changes_nothing() {
    let (daemon, id) = daemon();
    block_on(daemon.add_repo_to_session(&id, "acme/newrepo")).expect("the first add");
    let before = session_of(&daemon, &id).clones;

    block_on(daemon.add_repo_to_session(&id, "acme/newrepo")).expect("the second add");

    assert_eq!(session_of(&daemon, &id).clones, before);
}

#[test]
fn what_is_not_owner_slash_name_is_refused_and_changes_nothing() {
    let (daemon, id) = daemon();
    let before = session_of(&daemon, &id).clones;
    for text in ["", "acme", "/etc/passwd", "~/code/api", "../api", "a/b/c", "https://github.com/acme/api"] {
        let answer = block_on(daemon.add_repo_to_session(&id, text));
        assert!(matches!(answer, Err(DaemonError::Refused { .. })), "{text:?} is refused: {answer:?}");
    }
    assert_eq!(session_of(&daemon, &id).clones, before);
}

#[test]
fn a_session_that_is_not_there_is_not_found() {
    let (daemon, _) = daemon();
    let answer = block_on(daemon.add_repo_to_session(&SessionId::new("nowhere"), "acme/api"));
    assert!(matches!(answer, Err(DaemonError::NotFound { .. })), "{answer:?}");
}
