//! One repository failing must not hide the others' changes (DS-35): the
//! fake reads a repository on its own, fails one repository on request, and
//! can hold a read in flight.

use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use humata2_clients::model::{DiffBase, FileDiff, RepoDiff, SessionId};
use humata2_clients::{Daemon, DaemonError, FakeDaemon, Operation, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

const SESSION: &str = "s-auth";

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session() -> SessionId {
    SessionId::new(SESSION)
}

fn file(path: &str) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added: 1,
        removed: 0,
        lines: Vec::new(),
        ..FileDiff::default()
    }
}

fn changes(repo: &str, files: Vec<FileDiff>) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "main".to_owned(),
        files,
        comments: Vec::new(),
    }
}

/// A session with changes in acme/api and acme/web.
fn daemon() -> FakeDaemon {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    daemon.set_diffs(
        &session(),
        vec![
            changes("acme/api", vec![file("src/client.rs")]),
            changes("acme/web", vec![file("src/auth/SignIn.tsx")]),
        ],
    );
    daemon
}

fn paths(diffs: &[RepoDiff]) -> Vec<String> {
    diffs
        .iter()
        .flat_map(|diff| diff.files.iter().map(|file| file.path.clone()))
        .collect()
}

#[test]
fn a_repository_read_alone_gives_only_its_changes() {
    let daemon = daemon();
    let api = block_on(daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/api"));
    assert_eq!(paths(&api.unwrap_or_default()), ["src/client.rs"]);
}

#[test]
fn a_repository_without_changes_reads_empty() {
    let daemon = daemon();
    let none = block_on(daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/infra"));
    assert!(matches!(none, Ok(diffs) if diffs.is_empty()));
}

#[test]
fn a_failing_repository_fails_the_whole_read_but_not_the_others_alone() {
    let daemon = daemon();
    daemon.fail_repo(&session(), "acme/web", "the clone is locked");

    let all = block_on(daemon.diffs(&session(), &DiffBase::OriginMain));
    assert!(matches!(all, Err(DaemonError::Scripted { .. })));

    let api = block_on(daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/api"));
    assert_eq!(paths(&api.unwrap_or_default()), ["src/client.rs"]);

    let web = block_on(daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/web"));
    assert!(matches!(web, Err(DaemonError::Scripted { .. })));
}

#[test]
fn a_healed_repository_reads_again() {
    let daemon = daemon();
    daemon.fail_repo(&session(), "acme/web", "the clone is locked");
    daemon.heal_repo(&session(), "acme/web");

    let all = block_on(daemon.diffs(&session(), &DiffBase::OriginMain));
    assert_eq!(paths(&all.unwrap_or_default()), ["src/client.rs", "src/auth/SignIn.tsx"]);
    let web = block_on(daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/web"));
    assert_eq!(paths(&web.unwrap_or_default()), ["src/auth/SignIn.tsx"]);
}

#[test]
fn a_failing_repository_of_another_session_changes_nothing() {
    let daemon = daemon();
    daemon.fail_repo(&SessionId::new("s-other"), "acme/web", "the clone is locked");
    assert!(block_on(daemon.diffs(&session(), &DiffBase::OriginMain)).is_ok());
}

#[test]
fn a_held_read_waits_until_released() {
    let daemon = daemon();
    daemon.hold(Operation::Diffs);
    let mut read = daemon.diffs(&session(), &DiffBase::OriginMain);
    let mut context = Context::from_waker(Waker::noop());

    for _ in 0..5 {
        assert!(read.as_mut().poll(&mut context).is_pending());
    }
    daemon.release(Operation::Diffs);
    match read.as_mut().poll(&mut context) {
        Poll::Ready(Ok(diffs)) => assert_eq!(paths(&diffs), ["src/client.rs", "src/auth/SignIn.tsx"]),
        Poll::Ready(Err(error)) => panic!("the read finishes: {error}"),
        Poll::Pending => panic!("the released read finishes"),
    }
}

#[test]
fn a_held_repository_read_waits_too_and_later_reads_go_after_release() {
    let daemon = daemon();
    daemon.hold(Operation::Diffs);
    daemon.hold(Operation::Diffs);
    let mut read = daemon.repo_diff(&session(), &DiffBase::OriginMain, "acme/api");
    let mut context = Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut context).is_pending());

    daemon.release(Operation::Diffs);
    assert!(read.as_mut().poll(&mut context).is_ready());
    assert!(block_on(daemon.diffs(&session(), &DiffBase::OriginMain)).is_ok());
}

#[test]
fn releasing_what_is_not_held_does_nothing() {
    let daemon = daemon();
    daemon.release(Operation::Diffs);
    assert!(block_on(daemon.diffs(&session(), &DiffBase::OriginMain)).is_ok());
}
