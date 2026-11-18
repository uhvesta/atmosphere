//! PR-8 and PR-25: the Info panel's switches for a review: whether the agent
//! may push to the pull request's branch, and which of Focus, Notes and Order
//! it may do. A failed change puts the switch back and says why.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::fake::Call as PullCall;
use humata2_clients::pull_requests::{
    FakePullRequests, PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState,
    PullRequestsClient, ReviewStart,
};
use humata2_clients::review_chat::fake::Call as ChatCall;
use humata2_clients::review_chat::{AgentSwitches, FakeReviewChat, ReviewChatClient};
use humata2_pr_review::info::{Info, InfoAction as A, InfoState, Switch};
use tca::TestStore;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn refused() -> DaemonError {
    DaemonError::Refused {
        what: "The change".to_owned(),
        reason: "the daemon is not taking it right now".to_owned(),
    }
}

fn rig() -> (FakePullRequests, FakeReviewChat, SessionId, TestStore<Info>) {
    let pr = PullRequestRef::new("acme/api", 482);
    let pulls = FakePullRequests::new();
    pulls.add_pull_request(PullRequestInfo {
        pr: pr.clone(),
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
    let session = block_on(pulls.start_review(ReviewStart {
        prs: vec![pr],
        machine: MachineId::new("this-mac"),
        agent: Agent::Claude,
        model: String::new(),
        first_message: String::new(),
        push_allowed: false,
    }))
    .expect("the review starts");
    let chat = FakeReviewChat::new();
    let store = TestStore::new(
        InfoState::default(),
        Info::new(Arc::new(pulls.clone()), Arc::new(chat.clone())),
    )
    .non_exhaustive();
    (pulls, chat, session, store)
}

fn opened() -> (FakePullRequests, FakeReviewChat, SessionId, TestStore<Info>) {
    let (pulls, chat, session, mut store) = rig();
    store.send(A::Open { session: session.clone(), push_allowed: false }, |_| {});
    store.receive_matching("the switches load", |a| matches!(a, A::Loaded { .. }), |_| {});
    (pulls, chat, session, store)
}

#[test]
fn the_defaults_are_every_agent_switch_on_and_pushing_off() {
    let state = InfoState::default();
    assert_eq!(state.switches, AgentSwitches::default());
    assert!(state.switches.focus && state.switches.notes && state.switches.order);
    assert!(!state.push_allowed);
    let (_, _, _, store) = opened();
    assert_eq!(store.state().switches, AgentSwitches::default());
}

#[test]
fn switches_the_daemon_already_holds_are_shown_as_they_are() {
    let (_, chat, session, mut store) = rig();
    let held = AgentSwitches { focus: false, notes: true, order: false };
    block_on(chat.set_switches(&session, held)).expect("set");
    store.send(A::Open { session, push_allowed: false }, |_| {});
    store.receive_matching("the switches load", |a| matches!(a, A::Loaded { .. }), |_| {});
    assert_eq!(store.state().switches, held);
}

#[test]
fn turning_notes_off_reaches_the_clients_switches() {
    let (_, chat, session, mut store) = opened();
    store.send(A::SetSwitch(Switch::Notes, false), |_| {});
    assert!(!store.state().switches.notes, "the switch moves at once");
    store.receive_matching("answered", |a| matches!(a, A::SwitchesSet { result: Ok(_), .. }), |_| {});
    let want = AgentSwitches { focus: true, notes: false, order: true };
    assert!(chat.calls().contains(&ChatCall::SetSwitches(session, want)));
    assert!(store.state().error.is_none());
}

#[test]
fn turning_push_on_calls_set_push_allowed() {
    let (pulls, _, session, mut store) = opened();
    store.send(A::SetPush(true), |_| {});
    store.receive_matching("answered", |a| matches!(a, A::PushSet { result: Ok(()), .. }), |_| {});
    assert!(store.state().push_allowed);
    assert!(pulls.calls().contains(&PullCall::SetPushAllowed(session, true)));
}

#[test]
fn a_switch_the_daemon_refused_goes_back_and_says_why() {
    let (_, chat, _, mut store) = opened();
    chat.fail("set_switches", refused());
    store.send(A::SetSwitch(Switch::Order, false), |_| {});
    store.receive_matching("refused", |a| matches!(a, A::SwitchesSet { result: Err(_), .. }), |_| {});
    assert!(store.state().switches.order);
    assert!(store.state().error.is_some());

    store.send(A::DismissError, |_| {});
    assert!(store.state().error.is_none());
}

#[test]
fn a_push_change_the_daemon_refused_goes_back_and_says_why() {
    let (pulls, _, _, mut store) = opened();
    pulls.fail("set_push_allowed", PullRequestError::Daemon(refused()));
    store.send(A::SetPush(true), |_| {});
    assert!(store.state().push_allowed);
    store.receive_matching("refused", |a| matches!(a, A::PushSet { result: Err(_), .. }), |_| {});
    assert!(!store.state().push_allowed);
    assert!(store.state().error.is_some());
}

#[test]
fn a_review_that_may_push_shows_its_push_switch_on() {
    let (_, _, session, mut store) = rig();
    store.send(A::Open { session, push_allowed: true }, |_| {});
    assert!(store.state().push_allowed);
}
