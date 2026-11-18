//! SE-23: waiting messages retain their session and wait for certain turn endings.
use humata2_clients::{
    Clients, Fakes, Operation, fixture,
    model::{Agent, SessionId, Status, Turn},
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
    for agent in [Agent::Codex, Agent::Shell] {
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
fn an_opencode_turn_end_from_its_hooks_delivers_waiting_messages() {
    let (mut store, fakes) = setup(Agent::OpenCode, Turn::Running);
    store.send(A::Draft("after this turn".into()), |_| {});
    store.send(A::Submit, |_| {});
    assert!(!store.state().sending);
    let mut workspace = fakes.daemon.workspace();
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
fn a_copilot_whose_turn_end_is_not_sure_keeps_its_messages_waiting() {
    let (mut store, _fakes) = setup(Agent::Copilot, Turn::Uncertain);
    store.send(A::Draft("only when sure".into()), |_| {});
    store.send(A::Submit, |_| {});
    assert!(!store.state().sending);
    assert_eq!(store.state().view().unwrap().queue.len(), 1);
    store.finish();
}

#[test]
fn messages_queued_at_the_same_instant_get_distinct_keys() {
    // The test clock never moves, so every message is made at the same instant.
    let (mut store, _fakes) = setup(Agent::Claude, Turn::Running);
    for _ in 0..3 {
        store.send(A::Draft("same words".into()), |_| {});
        store.send(A::Submit, |_| {});
    }
    let queue = &store.state().view().unwrap().queue;
    let keys: std::collections::BTreeSet<_> = queue.iter().map(|m| m.key().map(str::to_owned)).collect();
    assert_eq!(queue.len(), 3);
    assert_eq!(keys.len(), 3);
    assert!(!keys.contains(&None));
    store.finish();
}

#[test]
fn a_queued_message_sent_again_after_a_restart_is_typed_once() {
    let (mut store, fakes) = setup(Agent::Claude, Turn::Running);
    store.send(A::Draft("run the linter".into()), |_| {});
    store.send(A::Submit, |_| {});
    // What the app had saved when it quit while the message was on its way.
    let saved = serde_json::to_string(&store.state().view().unwrap().queue).unwrap();
    store.send(A::SendQueued(0), |s| s.sending = true);
    store.receive_matching(
        "first delivery",
        |a| matches!(a, A::Sent { result: Ok(()), .. }),
        |s| s.sending = false,
    );
    // The next run loads the saved queue, still holding the message, and sends it again.
    let queue: Vec<QueuedMessage> = serde_json::from_str(&saved).unwrap();
    let mut views = std::collections::BTreeMap::new();
    views.insert(
        SessionId::new("ledger"),
        SessionView {
            queue,
            ..Default::default()
        },
    );
    store.send(A::Restore(views), |_| {});
    store.send(A::SendQueued(0), |s| s.sending = true);
    store.receive_matching(
        "second delivery",
        |a| matches!(a, A::Sent { result: Ok(()), .. }),
        |s| s.sending = false,
    );
    let typed = fakes
        .daemon
        .workspace()
        .session(&SessionId::new("ledger"))
        .unwrap()
        .transcript
        .iter()
        .filter(|line| line.text == "run the linter")
        .count();
    assert_eq!(typed, 1);
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
