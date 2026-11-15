//! SE-23: waiting messages retain their session and wait for certain turn endings.
use humata2_clients::{
    Clients, Fakes, Operation, fixture,
    model::{Agent, SessionId, Status, Turn},
};
use humata2_session::{SessionAction as A, SessionState, SessionView, Sessions};
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
            store.state().view().unwrap().queue,
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
    assert_eq!(store.state().view().unwrap().queue, ["first", "second"]);
    store.send(A::RemoveQueued(0), |_| {});
    store.send(A::SendQueued(0), |_| {});
    store.send(A::Open(SessionId::new("brin")), |_| {});
    store.receive_matching(
        "failed send",
        |a| matches!(a, A::Sent { result: Err(_), .. }),
        |s| s.sending = false,
    );
    assert_eq!(
        store.state().views[&SessionId::new("ledger")].queue,
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
        store.state().views[&SessionId::new("ledger")].queue,
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
    assert_eq!(store.state().view().unwrap().queue, ["first"]);
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
    assert_eq!(store.state().view().unwrap().queue, ["keep this"]);
    assert!(!store.state().sending);
    store.send(A::ShowQueue(true), |s| s.queue_expanded = true);
    store.send(A::Open(SessionId::new("brin")), |s| {
        s.queue_expanded = false
    });
    store.send(A::RemoveQueued(0), |_| {});
    assert_eq!(
        store.state().views[&SessionId::new("ledger")].queue,
        ["keep this"]
    );
    store.finish();
}
