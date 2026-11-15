//! FK-1/2: choose a kind, retain the draft on failure, and guard an in-flight fork.
use humata2_clients::{
    Clients, DaemonError, Operation, fixture,
    model::{ForkKind, SessionId},
};
use humata2_session::{ForkDraft, SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn a_failed_fork_keeps_its_title_and_kind_for_retry() {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    fakes.daemon.fail_next(Operation::Fork, "link unavailable");
    let state = SessionState {
        workspace: Arc::new(fakes.daemon.workspace()),
        open: Some(SessionId::new("ledger")),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients)).non_exhaustive();
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkKind(ForkKind::Conversation), |s| {
        s.fork.as_mut().unwrap().kind = ForkKind::Conversation
    });
    store.send(A::ForkTitle("Review safely".into()), |s| {
        s.fork.as_mut().unwrap().title = "Review safely".into()
    });
    store.send(A::ConfirmFork, |s| s.forking = true);
    store.receive(
        A::Forked(Err(DaemonError::Scripted {
            operation: Operation::Fork,
            message: "link unavailable".into(),
        })),
        |s| s.forking = false,
    );
    assert!(store.state().error.is_some());
    assert_eq!(
        store.state().fork.as_ref().unwrap(),
        &ForkDraft {
            kind: ForkKind::Conversation,
            title: "Review safely".into()
        }
    );
    store.send(A::ConfirmFork, |s| {
        s.forking = true;
        s.error = None;
    });
    store.receive_matching(
        "created fork",
        |a| matches!(a, A::Forked(Ok(_))),
        |s| {
            s.forking = false;
            s.fork = None;
        },
    );
    let workspace = fakes.daemon.workspace();
    let children: Vec<_> = workspace
        .sessions
        .iter()
        .filter(|s| {
            s.parent.as_ref() == Some(&SessionId::new("ledger")) && s.title == "Review safely"
        })
        .collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].fork, Some(ForkKind::Conversation));
    store.finish();
}
#[test]
fn pending_forks_ignore_repeated_create_and_edits() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let state = SessionState {
        open: Some(SessionId::new("ledger")),
        fork: Some(ForkDraft {
            kind: ForkKind::Shared,
            title: "Fresh attempt".into(),
        }),
        forking: true,
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients));
    for action in [
        A::ConfirmFork,
        A::ForkTitle("Changed".into()),
        A::ForkKind(ForkKind::Worktree),
        A::CancelFork,
        A::StartFork(ForkKind::Conversation),
    ] {
        store.send(action, |_| {});
    }
    store.finish();
}
