//! PR-1 to PR-8: the start dialog reads the pasted pull request at once, adds
//! more under "Also review", picks the machine, agent and first message, and
//! Start opens the new session. Each problem blocks Start where it shows.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, MachineId};
use humata2_clients::progress::{OpKind, OpProgress, OpUpdate};
use humata2_clients::pull_requests::{
    Call, FakePullRequests, PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState,
};
use humata2_pr_review::dialog::{
    Blocker, Delegate, Dialog, DialogAction as A, DialogState, Fetch, PasteProblem, Start,
};
use tca::TestStore;

const API_482: &str = "https://github.com/acme/api/pull/482";
const WEB_77: &str = "acme/web#77";

fn info(repo: &str, number: u32) -> PullRequestInfo {
    PullRequestInfo {
        pr: PullRequestRef::new(repo, number),
        title: format!("Change {number}"),
        author: "mara".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: format!("change-{number}"),
        head_sha: "abc123".to_owned(),
        files: 3,
        added: 40,
        removed: 8,
        state: PullRequestState::Open,
        checks: "3 passed".to_owned(),
    }
}

fn github() -> FakePullRequests {
    let fake = FakePullRequests::new();
    fake.add_pull_request(info("acme/api", 482));
    fake.add_pull_request(info("acme/web", 77));
    fake
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

/// The dialog open with This Mac and Claude as the usual picks.
fn opened(fake: &FakePullRequests) -> TestStore<Dialog> {
    let mut store = TestStore::new(DialogState::default(), Dialog::new(Arc::new(fake.clone()))).non_exhaustive();
    store.send(
        A::Open {
            machine: Some(mac()),
            agent: Agent::Claude,
        },
        |_| {},
    );
    store
}

fn read(store: &mut TestStore<Dialog>) {
    store.receive_matching(
        "GitHub answered",
        |action| matches!(action, A::Resolved { .. }),
        |_| {},
    );
}

fn main_fetch(store: &TestStore<Dialog>) -> Option<&Fetch> {
    store.state().main.as_ref().map(|main| &main.fetch)
}

fn starts_made(fake: &FakePullRequests) -> usize {
    fake.calls()
        .iter()
        .filter(|call| matches!(call, Call::StartReview(_)))
        .count()
}

#[test]
fn a_pasted_link_is_read_at_once_and_becomes_the_main_pull_request() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    assert_eq!(main_fetch(&store), Some(&Fetch::Fetching));
    assert_eq!(
        store.state().blocker(),
        Some(Blocker::Fetching(PullRequestRef::new("acme/api", 482)))
    );
    read(&mut store);
    assert_eq!(main_fetch(&store), Some(&Fetch::Found(info("acme/api", 482))));
    assert_eq!(store.state().blocker(), None);
}

#[test]
fn text_that_is_not_a_pull_request_is_refused_without_asking_github() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste("see the refund thing".to_owned()), |_| {});
    assert!(matches!(
        store.state().paste_problem,
        Some(PasteProblem::NotAPullRequest { .. })
    ));
    assert_eq!(store.state().main, None);
    assert_eq!(store.state().blocker(), Some(Blocker::NoPullRequest));
    assert!(fake.calls().is_empty());
}

#[test]
fn more_pull_requests_go_under_also_review_and_each_is_read() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::AddAlso(WEB_77.to_owned()), |_| {});
    read(&mut store);
    let state = store.state();
    assert_eq!(state.also.len(), 1);
    assert_eq!(state.also[0].fetch, Fetch::Found(info("acme/web", 77)));
    assert_eq!(
        state.pull_requests(),
        vec![&PullRequestRef::new("acme/api", 482), &PullRequestRef::new("acme/web", 77)]
    );
    store.send(A::RemoveAlso(PullRequestRef::new("acme/web", 77)), |_| {});
    assert!(store.state().also.is_empty());
}

#[test]
fn a_pull_request_already_in_the_dialog_is_not_added_twice() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::AddAlso(API_482.to_owned()), |_| {});
    assert_eq!(
        store.state().paste_problem,
        Some(PasteProblem::AlreadyAdded(PullRequestRef::new("acme/api", 482)))
    );
    assert!(store.state().also.is_empty());
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn start_does_nothing_while_a_pull_request_is_being_read() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    store.send(A::Start, |_| {});
    assert_eq!(store.state().start, Start::Idle);
    assert_eq!(starts_made(&fake), 0);
}

#[test]
fn start_needs_a_machine() {
    let fake = github();
    let mut store = TestStore::new(DialogState::default(), Dialog::new(Arc::new(fake.clone()))).non_exhaustive();
    store.send(
        A::Open {
            machine: None,
            agent: Agent::Claude,
        },
        |_| {},
    );
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    assert_eq!(store.state().blocker(), Some(Blocker::NoMachine));
    store.send(A::Start, |_| {});
    assert_eq!(starts_made(&fake), 0);
    store.send(A::PickMachine(mac()), |_| {});
    assert_eq!(store.state().blocker(), None);
}

#[test]
fn a_pull_request_that_cannot_be_read_blocks_start_on_its_card_until_it_reads() {
    let fake = github();
    fake.sign_out(true);
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    assert!(matches!(main_fetch(&store), Some(Fetch::Failed(PullRequestError::SignedOut { .. }))));
    assert_eq!(
        store.state().blocker(),
        Some(Blocker::CannotRead(PullRequestRef::new("acme/api", 482)))
    );
    store.send(A::Start, |_| {});
    assert_eq!(starts_made(&fake), 0);

    fake.sign_out(false);
    store.send(A::Retry(PullRequestRef::new("acme/api", 482)), |_| {});
    assert_eq!(main_fetch(&store), Some(&Fetch::Fetching));
    read(&mut store);
    assert_eq!(store.state().blocker(), None);
}

#[test]
fn a_pull_request_github_does_not_know_is_a_failure_on_its_card() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste("https://github.com/acme/api/pull/9999".to_owned()), |_| {});
    read(&mut store);
    assert!(matches!(main_fetch(&store), Some(Fetch::Failed(PullRequestError::NotFound { .. }))));
}

#[test]
fn a_rate_limit_says_to_wait_and_for_how_long() {
    let fake = github();
    fake.set_rate_limit(Some(Duration::from_secs(90)));
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    let Some(Fetch::Failed(error)) = main_fetch(&store) else {
        panic!("the card shows the failure, got {:?}", main_fetch(&store));
    };
    assert!(matches!(
        error,
        PullRequestError::RateLimited {
            retry_after: Some(wait),
            ..
        } if *wait == Duration::from_secs(90)
    ));
    assert!(error.to_string().contains("90"), "the message gives the time: {error}");
}

#[test]
fn an_also_review_pull_request_that_fails_blocks_start_until_it_is_taken_out() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::AddAlso("acme/web#9999".to_owned()), |_| {});
    read(&mut store);
    assert_eq!(
        store.state().blocker(),
        Some(Blocker::CannotRead(PullRequestRef::new("acme/web", 9999)))
    );
    store.send(A::RemoveAlso(PullRequestRef::new("acme/web", 9999)), |_| {});
    assert_eq!(store.state().blocker(), None);
}

#[test]
fn start_asks_for_every_pick_and_opens_the_new_session() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::PickMachine(MachineId::new("devbox-1")), |_| {});
    store.send(A::PickAgent(Agent::Codex), |_| {});
    store.send(A::SetModel("fast".to_owned()), |_| {});
    store.send(A::SetContext("Focus on the refund rounding".to_owned()), |_| {});
    store.send(A::AllowPush(true), |_| {});
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::AddAlso(WEB_77.to_owned()), |_| {});
    read(&mut store);

    store.send(A::Start, |_| {});
    assert_eq!(store.state().start, Start::Starting);
    assert_eq!(store.state().blocker(), Some(Blocker::Starting));
    store.receive_matching(
        "the daemon made the session",
        |action| matches!(action, A::Started(Ok(_))),
        |_| {},
    );
    store.receive_matching(
        "the parent is told to open it",
        |action| matches!(action, A::Delegate(Delegate::Opened(_))),
        |_| {},
    );

    let starts = fake.starts();
    assert_eq!(starts.len(), 1);
    let asked = &starts[0];
    assert_eq!(
        asked.prs,
        vec![PullRequestRef::new("acme/api", 482), PullRequestRef::new("acme/web", 77)]
    );
    assert_eq!(asked.machine, MachineId::new("devbox-1"));
    assert_eq!(asked.agent, Agent::Codex);
    assert_eq!(asked.model, "fast");
    assert_eq!(asked.first_message, "Focus on the refund rounding");
    assert!(asked.push_allowed);
    assert!(!store.state().open, "the dialog closes once the session exists");
}

#[test]
fn the_agent_may_not_push_unless_the_person_allows_it() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::Start, |_| {});
    store.receive_matching("started", |action| matches!(action, A::Started(Ok(_))), |_| {});
    assert!(!fake.starts()[0].push_allowed);
}

#[test]
fn a_failed_start_shows_why_and_can_be_tried_again() {
    let fake = github();
    let offline = PullRequestError::Daemon(DaemonError::Offline { machine: mac() });
    fake.fail("start_review", offline.clone());
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::Start, |_| {});
    store.receive_matching("start failed", |action| matches!(action, A::Started(Err(_))), |_| {});
    assert_eq!(store.state().start, Start::Failed(offline));
    assert_eq!(store.state().blocker(), None, "the person can press Start again");
    assert!(store.state().open);

    fake.clear_failure("start_review");
    store.send(A::Start, |_| {});
    store.receive_matching("started", |action| matches!(action, A::Started(Ok(_))), |_| {});
    assert_eq!(fake.starts().len(), 1);
}

#[test]
fn closing_forgets_what_was_pasted() {
    let fake = github();
    let mut store = opened(&fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store.send(A::Close, |_| {});
    assert_eq!(store.state(), &DialogState::default());
}

fn step(title: &str) -> OpProgress {
    OpProgress {
        title: title.to_owned(),
        fraction: None,
        label: String::new(),
        started_ms: 0,
    }
}

fn moved(id: &str, kind: OpKind, title: &str) -> A {
    A::OperationMoved(OpUpdate {
        id: id.to_owned(),
        kind,
        progress: Some(step(title)),
    })
}

/// The dialog open with a pull request read and This Mac picked, ready to start.
fn ready_to_start(fake: &FakePullRequests) -> TestStore<Dialog> {
    let mut store = opened(fake);
    store.send(A::Paste(API_482.to_owned()), |_| {});
    read(&mut store);
    store
}

fn receive_started_ok(store: &mut TestStore<Dialog>) {
    store.receive_matching("started", |action| matches!(action, A::Started(Ok(_))), |_| {});
    store.receive_matching(
        "the parent is told to open it",
        |action| matches!(action, A::Delegate(Delegate::Opened(_))),
        |_| {},
    );
}

#[test]
fn the_running_step_of_the_start_shows_and_follows_its_operation() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-7", OpKind::StartSession, "Cloning acme/api"), |_| {});
    assert_eq!(store.state().progress, Some(step("Cloning acme/api")));
    assert_eq!(store.state().op.as_deref(), Some("op-7"));

    store.send(moved("op-7", OpKind::StartSession, "Starting the agent"), |_| {});
    assert_eq!(store.state().progress, Some(step("Starting the agent")));
    assert_eq!(store.state().op.as_deref(), Some("op-7"));
    receive_started_ok(&mut store);
}

#[test]
fn a_review_start_operation_is_followed_too() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-9", OpKind::StartPrReview, "Reviewing acme/api"), |_| {});
    assert_eq!(store.state().progress, Some(step("Reviewing acme/api")));
    receive_started_ok(&mut store);
}

#[test]
fn a_later_operation_does_not_replace_the_one_being_followed() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-7", OpKind::StartSession, "Cloning acme/api"), |_| {});
    store.send(moved("op-8", OpKind::StartSession, "Some other step"), |_| {});
    assert_eq!(store.state().progress, Some(step("Cloning acme/api")));
    assert_eq!(store.state().op.as_deref(), Some("op-7"));
    receive_started_ok(&mut store);
}

#[test]
fn an_operation_of_another_kind_is_ignored() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-7", OpKind::Fork, "Forking"), |_| {});
    assert_eq!(store.state().progress, None);
    assert_eq!(store.state().op, None);
    receive_started_ok(&mut store);
}

#[test]
fn an_update_while_the_dialog_is_idle_changes_nothing() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(moved("op-7", OpKind::StartSession, "Cloning acme/api"), |_| {});
    assert_eq!(store.state().start, Start::Idle);
    assert_eq!(store.state().progress, None);
    assert_eq!(store.state().op, None);
}

#[test]
fn a_failed_start_keeps_its_error_and_drops_the_step() {
    let fake = github();
    let offline = PullRequestError::Daemon(DaemonError::Offline { machine: mac() });
    fake.fail("start_review", offline.clone());
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-7", OpKind::StartSession, "Cloning acme/api"), |_| {});
    store.receive_matching("start failed", |action| matches!(action, A::Started(Err(_))), |_| {});
    assert_eq!(store.state().start, Start::Failed(offline));
    assert_eq!(store.state().progress, None);
    assert_eq!(store.state().op, None);
}

#[test]
fn a_started_session_drops_the_step_as_the_dialog_closes() {
    let fake = github();
    let mut store = ready_to_start(&fake);
    store.send(A::Start, |_| {});
    store.send(moved("op-7", OpKind::StartSession, "Cloning acme/api"), |_| {});
    receive_started_ok(&mut store);
    assert_eq!(store.state().progress, None);
    assert_eq!(store.state().op, None);
    assert!(!store.state().open);
}
