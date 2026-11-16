//! SE-23: waiting messages retain their session and wait for certain turn endings.
use humata2_clients::{
    Clients, Fakes, Operation, fixture,
    model::{Agent, Comment, CommentContext, SessionId, Status, Turn},
};
use humata2_session::{QueuedMessage, SessionAction as A, SessionState, SessionView, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup(agent: Agent, turn: Turn) -> (TestStore<Sessions>, Fakes) {
    let mut workspace = fixture::design();
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap();
    session.agent = agent;
    session.turn = turn;
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(SessionId::new("ledger")),
        ..Default::default()
    };
    state
        .views
        .insert(SessionId::new("ledger"), SessionView::default());
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes,
    )
}

#[test]
fn fallback_and_plain_shell_messages_wait_for_an_explicit_send() {
    for agent in [Agent::Codex, Agent::OpenCode, Agent::Shell] {
        let (mut store, fakes) = setup(agent, Turn::Ended);
        store.send(A::Draft("one whole message\nwith two lines".into()), |_| {});
        store.send(A::Submit, |_| {});
        assert_eq!(
            store
                .state()
                .view()
                .unwrap()
                .queue
                .iter()
                .map(|message| message.text())
                .collect::<Vec<_>>(),
            ["one whole message\nwith two lines"]
        );
        assert!(!store.state().sending);
        assert!(
            !fakes
                .daemon
                .workspace()
                .session(&SessionId::new("ledger"))
                .unwrap()
                .transcript
                .iter()
                .any(|l| l.text == "one whole message\nwith two lines")
        );
        store.send(A::SendQueued(0), |s| s.sending = true);
        store.receive_matching(
            "message delivered",
            |a| matches!(a, A::Sent { result: Ok(()), .. }),
            |s| s.sending = false,
        );
        assert!(store.state().view().unwrap().queue.is_empty());
        assert!(
            fakes
                .daemon
                .workspace()
                .session(&SessionId::new("ledger"))
                .unwrap()
                .transcript
                .iter()
                .any(|l| l.text == "one whole message\nwith two lines")
        );
        store.finish();
    }
}

#[test]
fn a_hook_turn_end_delivers_waiting_messages_but_running_and_uncertain_do_not() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    store.send(A::Draft("after this turn".into()), |_| {});
    store.send(A::Submit, |_| {});
    assert!(!store.state().sending);
    let mut workspace = fakes.daemon.workspace();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .turn = Turn::Uncertain;
    store.send(A::Show(Arc::new(workspace.clone())), |_| {});
    assert!(!store.state().sending);
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .turn = Turn::Ended;
    store.send(A::Show(Arc::new(workspace)), |s| s.sending = true);
    store.receive_matching(
        "hook delivery",
        |a| matches!(a, A::Sent { result: Ok(()), .. }),
        |s| s.sending = false,
    );
    assert!(store.state().view().unwrap().queue.is_empty());
    store.finish();
}

#[test]
fn a_failed_selected_message_returns_to_its_original_session_after_navigation() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    for text in ["first", "second"] {
        store.send(A::Draft(text.into()), |_| {});
        store.send(A::Submit, |_| {});
    }
    fakes.daemon.fail_next(Operation::Send, "link unavailable");
    store.send(A::SendQueued(1), |s| s.sending = true);
    assert_eq!(
        store
            .state()
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    store.send(A::RemoveQueued(0), |_| {});
    store.send(A::SendQueued(0), |_| {});
    store.send(A::Open(SessionId::new("brin")), |_| {});
    store.receive_matching(
        "failed send",
        |a| matches!(a, A::Sent { result: Err(_), .. }),
        |s| s.sending = false,
    );
    assert_eq!(
        store.state().views[&SessionId::new("ledger")]
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(store.state().view().unwrap().queue.is_empty());
    assert!(store.state().error.is_some());
    let mut workspace = fakes.daemon.workspace();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .turn = Turn::Ended;
    store.send(A::Show(Arc::new(workspace.clone())), |_| {});
    assert!(!store.state().sending);
    assert_eq!(
        store.state().views[&SessionId::new("ledger")]
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    // A new snapshot cannot silently retry a failed delivery. The person can.
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .turn = Turn::Running;
    store.send(A::Show(Arc::new(workspace)), |_| {});
    store.send(A::Open(SessionId::new("ledger")), |_| {});
    store.send(A::SendQueued(1), |s| s.sending = true);
    store.receive_matching(
        "explicit retry",
        |a| matches!(a, A::Sent { result: Ok(()), .. }),
        |s| s.sending = false,
    );
    assert_eq!(
        store
            .state()
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["first"]
    );
    assert!(store.state().failed_queues.is_empty());
    store.finish();
}

#[test]
fn offline_send_preserves_the_queue_and_remove_only_changes_the_open_session() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    store.send(A::Draft("keep this".into()), |_| {});
    store.send(A::Submit, |_| {});
    let mut workspace = fakes.daemon.workspace();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .status = Status::Offline;
    store.send(A::Show(Arc::new(workspace)), |_| {});
    store.send(A::SendQueued(0), |_| {});
    assert_eq!(
        store
            .state()
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["keep this"]
    );
    assert!(!store.state().sending);
    store.send(A::ShowQueue(true), |s| s.queue_expanded = true);
    store.send(A::Open(SessionId::new("brin")), |s| {
        s.queue_expanded = false
    });
    store.send(A::RemoveQueued(0), |_| {});
    assert_eq!(
        store.state().views[&SessionId::new("ledger")]
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        ["keep this"]
    );
    store.finish();
}

fn comment(body: &str) -> Comment {
    Comment {
        path: "src/client.rs".into(),
        line: 43,
        body: body.into(),
        sent: false,
        context: Some(CommentContext {
            repo: "api".into(),
            base: "main".into(),
            quoted: None,
        }),
    }
}

/// Bodies of comments delivered to ledger during the test: the fixture's
/// own already-sent comments are not deliveries.
fn delivered(fakes: &Fakes) -> Vec<String> {
    let ledger = |workspace: &humata2_clients::Workspace| -> Vec<String> {
        workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == SessionId::new("ledger"))
            .flat_map(|diff| &diff.comments)
            .map(|c| c.body.clone())
            .collect()
    };
    let before = ledger(&fixture::design());
    ledger(&fakes.daemon.workspace())
        .into_iter()
        .filter(|body| !before.contains(body))
        .collect()
}

fn queue_comments(store: &mut TestStore<Sessions>, body: &str) {
    store.send(
        A::QueueComments {
            session: SessionId::new("ledger"),
            comments: vec![comment(body)],
        },
        |_| {},
    );
}

fn receive_sent(store: &mut TestStore<Sessions>, ok: bool) {
    store.receive_matching(
        "delivery result",
        move |a| matches!(a, A::Sent { result, .. } if result.is_ok() == ok),
        |s| s.sending = false,
    );
}

#[test]
fn comments_queued_during_a_working_turn_are_delivered_right_away() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    queue_comments(&mut store, "split this function");
    assert!(store.state().sending);
    receive_sent(&mut store, true);
    assert!(store.state().view().unwrap().queue.is_empty());
    assert_eq!(delivered(&fakes), ["split this function"]);
    store.finish();
}

#[test]
fn comments_for_a_session_that_is_not_open_are_delivered_too() {
    let (mut store, fakes) = setup(Agent::Codex, Turn::Running);
    store.send(A::Open(SessionId::new("brin")), |_| {});
    queue_comments(&mut store, "elsewhere");
    receive_sent(&mut store, true);
    assert!(
        store.state().views[&SessionId::new("ledger")]
            .queue
            .is_empty()
    );
    assert_eq!(delivered(&fakes), ["elsewhere"]);
    store.finish();
}

#[test]
fn held_text_keeps_waiting_while_comments_queued_after_it_go_first() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    store.send(A::Draft("after this turn".into()), |_| {});
    store.send(A::Submit, |_| {});
    queue_comments(&mut store, "now please");
    receive_sent(&mut store, true);
    let queue = &store.state().view().unwrap().queue;
    assert_eq!(queue.len(), 1);
    assert!(matches!(queue[0], QueuedMessage::Text(_)));
    assert_eq!(delivered(&fakes), ["now please"]);
    assert!(!store.state().sending);
    store.finish();
}

#[test]
fn a_failed_comment_delivery_keeps_the_payload_and_retries_only_when_asked() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    fakes
        .daemon
        .fail_next(Operation::SendComments, "link unavailable");
    queue_comments(&mut store, "keep me");
    receive_sent(&mut store, false);
    assert_eq!(store.state().view().unwrap().queue.len(), 1);
    assert!(store.state().error.is_some());
    assert!(delivered(&fakes).is_empty());
    // A fresh snapshot must not retry on its own.
    let workspace = fakes.daemon.workspace();
    store.send(A::Show(Arc::new(workspace)), |_| {});
    assert!(!store.state().sending);
    assert_eq!(store.state().view().unwrap().queue.len(), 1);
    // Send now retries; then the queue is empty.
    store.send(A::SendNow, |s| s.sending = true);
    receive_sent(&mut store, true);
    assert!(store.state().view().unwrap().queue.is_empty());
    assert_eq!(delivered(&fakes), ["keep me"]);
    store.finish();
}

#[test]
fn send_queued_retries_a_failed_comment_delivery_and_the_next_message_follows() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    fakes
        .daemon
        .fail_next(Operation::SendComments, "link unavailable");
    queue_comments(&mut store, "first");
    receive_sent(&mut store, false);
    queue_comments(&mut store, "second");
    assert!(!store.state().sending);
    store.send(A::SendQueued(0), |s| s.sending = true);
    // Acknowledging the retried batch sends the next waiting comment
    // message at once, so delivery is still in progress.
    store.receive_matching(
        "retried delivery result",
        |a| matches!(a, A::Sent { result: Ok(()), .. }),
        |s| s.sending = true,
    );
    receive_sent(&mut store, true);
    assert!(store.state().view().unwrap().queue.is_empty());
    assert_eq!(delivered(&fakes), ["first", "second"]);
    store.finish();
}

#[test]
fn comments_for_a_session_that_cannot_take_input_stay_waiting_with_an_error() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    let mut workspace = fakes.daemon.workspace();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap()
        .status = Status::Offline;
    store.send(A::Show(Arc::new(workspace)), |_| {});
    queue_comments(&mut store, "later");
    assert!(!store.state().sending);
    assert!(store.state().error.is_some());
    assert_eq!(store.state().view().unwrap().queue.len(), 1);
    assert!(delivered(&fakes).is_empty());
    store.finish();
}
