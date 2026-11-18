//! PR-19, D200: Ask types one question, with its file and lines, into the
//! review chat and nowhere else.
use std::sync::Arc;

use humata2_clients::comments::{Anchor, Side};
use humata2_clients::fixture;
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{DaemonError, FakeDaemon, Operation};
use humata2_pr_review::ask::{Ask, AskAction as A, AskState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn anchor(first: u32, last: u32) -> Anchor {
    Anchor {
        repo: "acme/api".to_owned(),
        path: "src/refund.rs".to_owned(),
        side: Side::New,
        first,
        last,
        quoted: vec!["let total = round(amount);".to_owned()],
        base: None,
        detached: false,
        outdated: false,
    }
}

fn rig() -> (FakeDaemon, TestStore<Ask>) {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
            each.pull_request_review = true;
        }
    }
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace, clock.clone());
    let store = TestStore::new(AskState::default(), Ask::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    (daemon, store)
}

fn sends(daemon: &FakeDaemon) -> Vec<String> {
    daemon.calls().into_iter().filter(|call| call.starts_with("send ")).collect()
}

fn ask(store: &mut TestStore<Ask>, anchor: Anchor, question: &str) {
    store.send(
        A::Ask { session: session(), anchor, question: question.to_owned() },
        |_| {},
    );
    store.receive_matching("the terminal answered", |action| matches!(action, A::Typed(_)), |_| {});
}

#[test]
fn ask_types_exactly_one_message_naming_the_file_and_lines() {
    let (daemon, mut store) = rig();
    ask(&mut store, anchor(12, 14), "Why round here?");
    let sent = sends(&daemon);
    assert_eq!(sent.len(), 1, "one message: {sent:?}");
    assert!(sent[0].contains("acme/api"));
    assert!(sent[0].contains("src/refund.rs"));
    assert!(sent[0].contains("12-14"));
    assert!(sent[0].contains("let total = round(amount);"));
    assert!(sent[0].contains("Why round here?"));
    assert!(store.state().error.is_none());
}

#[test]
fn a_blank_question_types_nothing() {
    let (daemon, mut store) = rig();
    store.send(
        A::Ask { session: session(), anchor: anchor(3, 3), question: "   ".to_owned() },
        |_| {},
    );
    assert!(sends(&daemon).is_empty());
}

#[test]
fn two_questions_are_two_messages() {
    let (daemon, mut store) = rig();
    ask(&mut store, anchor(3, 3), "First?");
    ask(&mut store, anchor(3, 3), "First?");
    assert_eq!(sends(&daemon).len(), 2);
}

#[test]
fn a_refused_question_shows_what_went_wrong_and_can_be_dismissed() {
    let (daemon, mut store) = rig();
    daemon.fail_next_with(
        Operation::Send,
        DaemonError::Refused {
            what: "Typing into ledger".to_owned(),
            reason: "the session has exited".to_owned(),
        },
    );
    ask(&mut store, anchor(3, 3), "Anyone there?");
    assert!(store.state().error.is_some());
    store.send(A::DismissError, |_| {});
    assert!(store.state().error.is_none());
}

#[test]
fn a_failure_belongs_to_the_review_it_was_asked_in() {
    let (daemon, mut store) = rig();
    daemon.fail_next_with(
        Operation::Send,
        DaemonError::Refused {
            what: "Typing into ledger".to_owned(),
            reason: "the session has exited".to_owned(),
        },
    );
    ask(&mut store, anchor(3, 3), "Anyone there?");
    assert!(store.state().error_for(&session()).is_some());
    assert!(store.state().error_for(&SessionId::new("brin")).is_none());
    store.send(A::DismissError, |_| {});
    assert!(store.state().error_for(&session()).is_none());
}
