//! PR-15 to PR-17, D286, D298, D300: new commits arrive on the daemon's feed
//! (the app runs no timer), show as a notice with Update the clone, and a
//! merged pull request offers Archive.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::fake::Call;
use humata2_clients::pull_requests::{
    FakePullRequests, HeadChange, PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState,
    PullRequestsClient, ReviewStart, StackPosition,
};
use humata2_pr_review::heads::{Delegate, Heads, HeadsAction as A, HeadsState};
use tca::TestStore;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn api() -> PullRequestRef {
    PullRequestRef::new("acme/api", 482)
}

fn change(new_commits: u32) -> HeadChange {
    HeadChange {
        pr: api(),
        new_commits,
        force_pushed: false,
        state: PullRequestState::Open,
        base_branch: None,
        stack: None,
    }
}

/// A fake GitHub with one review of `acme/api#482`, and notices following it.
fn rig() -> (FakePullRequests, SessionId, TestStore<Heads>) {
    let fake = FakePullRequests::new();
    fake.add_pull_request(PullRequestInfo {
        pr: api(),
        title: "Round refunds".to_owned(),
        author: "mara".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: "round-refunds".to_owned(),
        head_sha: "abc123".to_owned(),
        files: 3,
        added: 40,
        removed: 8,
        state: PullRequestState::Open,
        checks: String::new(),
    });
    let session = block_on(fake.start_review(ReviewStart {
        prs: vec![api()],
        machine: MachineId::new("this-mac"),
        agent: Agent::Claude,
        model: String::new(),
        first_message: String::new(),
        push_allowed: false,
    }))
    .expect("the review starts");
    let store = TestStore::new(HeadsState::default(), Heads::new(Arc::new(fake.clone()))).non_exhaustive();
    (fake, session, store)
}

/// Following the feed, which is open before the test changes anything.
fn following() -> (FakePullRequests, SessionId, TestStore<Heads>) {
    let (fake, session, mut store) = rig();
    store.send(A::Start, |_| {});
    store.settle();
    (fake, session, store)
}

fn pushed(store: &mut TestStore<Heads>) {
    store.receive_matching(
        "the daemon pushed a head change",
        |action| matches!(action, A::Pushed(_)),
        |_| {},
    );
}

fn commits_of(store: &TestStore<Heads>, session: &SessionId) -> Option<u32> {
    store
        .state()
        .notice(session)
        .into_iter()
        .find(|(pr, _)| **pr == api())
        .map(|(_, moved)| moved.new_commits)
}

#[test]
fn new_commits_arrive_on_the_feed_and_show_as_a_notice_without_any_check() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(2));
    pushed(&mut store);
    assert_eq!(commits_of(&store, &session), Some(2));
    assert!(!fake.calls().iter().any(|call| matches!(call, Call::CheckHeads(_))));
}

#[test]
fn pushes_add_up_until_the_clone_is_updated() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(2));
    pushed(&mut store);
    fake.emit_head_change(
        &session,
        HeadChange {
            force_pushed: true,
            ..change(3)
        },
    );
    pushed(&mut store);
    assert_eq!(commits_of(&store, &session), Some(5));
    let notice = store.state().notice(&session);
    assert!(notice[0].1.force_pushed);
}

#[test]
fn a_head_that_did_not_move_is_no_news() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(0));
    pushed(&mut store);
    assert!(store.state().notice(&session).is_empty());
}

#[test]
fn a_retargeted_base_and_its_stack_show_even_with_no_new_commits() {
    let (fake, session, mut store) = following();
    let stack = StackPosition {
        parent: Some(PullRequestRef::new("acme/api", 481)),
        children: vec![PullRequestRef::new("acme/api", 483)],
    };
    fake.emit_head_change(
        &session,
        HeadChange {
            base_branch: Some("release".to_owned()),
            stack: Some(stack.clone()),
            ..change(0)
        },
    );
    pushed(&mut store);
    let notice = store.state().notice(&session);
    assert_eq!(notice.len(), 1);
    assert_eq!(notice[0].1.base_branch.as_deref(), Some("release"));
    assert_eq!(notice[0].1.stack.as_ref(), Some(&stack));
}

#[test]
fn update_the_clone_asks_once_and_clears_the_notice() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(2));
    pushed(&mut store);

    store.send(A::UpdateClone(session.clone()), |_| {});
    store.send(A::UpdateClone(session.clone()), |_| {});
    assert_eq!(store.state().updating, vec![session.clone()]);
    store.receive_matching("updated", |action| matches!(action, A::CloneUpdated { result: Ok(()), .. }), |_| {});
    assert!(store.state().notice(&session).is_empty());
    assert!(store.state().updating.is_empty());
    let updates = fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::UpdateClone(_)))
        .count();
    assert_eq!(updates, 1);
}

#[test]
fn a_clone_that_could_not_be_updated_keeps_the_notice_and_says_why() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(2));
    pushed(&mut store);
    fake.fail(
        "update_clone",
        PullRequestError::Daemon(DaemonError::Refused {
            what: "The update".to_owned(),
            reason: "the clone has uncommitted changes; commit or discard them first".to_owned(),
        }),
    );
    store.send(A::UpdateClone(session.clone()), |_| {});
    store.receive_matching("refused", |action| matches!(action, A::CloneUpdated { result: Err(_), .. }), |_| {});
    assert!(store.state().error.is_some());
    assert_eq!(commits_of(&store, &session), Some(2));
    assert!(store.state().updating.is_empty(), "it can be tried again");

    fake.clear_failure("update_clone");
    store.send(A::UpdateClone(session.clone()), |_| {});
    store.receive_matching("updated", |action| matches!(action, A::CloneUpdated { result: Ok(()), .. }), |_| {});
    assert!(store.state().error.is_none());
    assert_eq!(commits_of(&store, &session), None);
}

#[test]
fn a_merged_pull_request_offers_archive_and_keeps_it_after_the_clone_updates() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(
        &session,
        HeadChange {
            state: PullRequestState::Merged,
            ..change(0)
        },
    );
    pushed(&mut store);
    assert_eq!(store.state().archivable(&session), vec![&api()]);

    store.send(A::UpdateClone(session.clone()), |_| {});
    store.receive_matching("updated", |action| matches!(action, A::CloneUpdated { .. }), |_| {});
    assert_eq!(store.state().archivable(&session), vec![&api()]);

    store.send(A::Archive(session.clone()), |_| {});
    store.receive(A::Delegate(Delegate::Archiving(session)), |_| {});
}

#[test]
fn an_open_pull_request_offers_no_archive() {
    let (fake, session, mut store) = following();
    fake.emit_head_change(&session, change(1));
    pushed(&mut store);
    assert!(store.state().archivable(&session).is_empty());
}

#[test]
fn a_feed_that_ends_is_followed_again() {
    let (fake, session, mut store) = following();
    store.send(A::WatchEnded, |_| {});
    store.settle();
    fake.emit_head_change(&session, change(1));
    pushed(&mut store);
    assert_eq!(commits_of(&store, &session), Some(1));
}

#[test]
fn a_feed_that_cannot_be_opened_says_so() {
    let (fake, _session, mut store) = rig();
    fake.fail(
        "watch_heads",
        PullRequestError::Daemon(DaemonError::Offline {
            machine: MachineId::new("this-mac"),
        }),
    );
    store.send(A::Start, |_| {});
    store.receive_matching("failed", |action| matches!(action, A::WatchFailed(_)), |_| {});
    assert!(!store.state().watching);
    assert!(store.state().error.is_some());
}
