//! The fake daemon behaves like the daemon the app will talk to: replies
//! wait on its clock, failures can be scripted, reconnecting a host brings
//! each session back one of three ways, and event queues stay bounded.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{ItemId, MachineId, Recovery, SessionId, Status};
use humata2_clients::{
    Daemon, DaemonError, EVENT_QUEUE, Event, FakeDaemon, ForkOptions, Operation, fixture,
};
use tca::TestClock;
use tokio::runtime::Builder;

fn options(kind: humata2_clients::model::ForkKind, title: &str) -> ForkOptions {
    ForkOptions {
        kind,
        title: title.to_owned(),
        before_turn: None,
        agent: None,
        machine: None,
        branch: String::new(),
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

#[test]
fn reconnecting_a_host_reattaches_resumes_or_explains_each_session() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let outcomes =
        block_on(daemon.reconnect(&MachineId::new("ci-runner"))).expect("the host reconnects");
    assert!(outcomes.contains(&(SessionId::new("s-tf"), Recovery::Reattached)));
    assert!(outcomes.contains(&(SessionId::new("s-runner"), Recovery::Resumed)));
    assert!(
        outcomes
            .iter()
            .any(|(id, recovery)| id.as_str() == "s-backup"
                && matches!(recovery, Recovery::Recoverable { .. }))
    );
    let workspace = daemon.workspace();
    assert!(
        workspace
            .machine(&MachineId::new("ci-runner"))
            .is_some_and(|m| m.online)
    );
    let tf = workspace
        .session(&SessionId::new("s-tf"))
        .expect("s-tf exists");
    assert_ne!(tf.status, Status::Offline);
}

#[test]
fn typing_into_an_offline_session_says_to_reconnect() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let error = block_on(daemon.send(&SessionId::new("s-tf"), "plan"));
    assert_eq!(
        error,
        Err(DaemonError::Offline {
            machine: MachineId::new("ci-runner")
        })
    );
}

#[test]
fn a_dependency_that_would_make_a_cycle_is_refused() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    // W-3 already waits for W-2.
    let refused = block_on(daemon.add_dependency(&ItemId::new("W-2"), &ItemId::new("W-3")));
    assert!(matches!(refused, Err(DaemonError::Refused { .. })));
    let allowed = block_on(daemon.add_dependency(&ItemId::new("W-5"), &ItemId::new("W-4")));
    assert_eq!(allowed, Ok(()));
}

#[test]
fn a_scripted_failure_fails_only_the_next_call() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    daemon.fail_next(Operation::Load, "the link dropped");
    assert!(matches!(
        block_on(daemon.load()),
        Err(DaemonError::Scripted { .. })
    ));
    assert!(block_on(daemon.load()).is_ok());
}

#[test]
fn replies_wait_for_the_latency_on_the_clock() {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(fixture::standard(), clock.clone())
        .with_latency(Duration::from_millis(200));
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime starts");
    let reply = runtime.spawn(daemon.load());
    runtime.block_on(async {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    });
    assert!(!reply.is_finished());
    clock.advance(Duration::from_millis(200));
    let loaded = runtime.block_on(reply).expect("the task ran");
    assert!(loaded.is_ok());
}

#[test]
fn a_subscriber_that_falls_behind_is_told_to_reload() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let mut events = block_on(daemon.events()).expect("events open");
    let machine = daemon.workspace().machines[0].clone();
    for _ in 0..EVENT_QUEUE + 10 {
        daemon.emit(Event::Machine(machine.clone()));
    }
    let mut received = 0;
    while let Ok(event) = events.try_recv() {
        received += 1;
        assert!(event.is_ok());
    }
    assert_eq!(received, EVENT_QUEUE);
    daemon.emit(Event::Machine(machine));
    assert_eq!(events.try_recv().ok(), Some(Ok(Event::Resync)));
}

#[test]
fn a_fork_starts_with_its_parents_clone_listing() {
    use humata2_clients::model::ForkKind;
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let parent = SessionId::new("s-auth");
    let before = daemon.workspace().session_files[&parent].clone();
    assert!(!before.is_empty());
    for kind in [ForkKind::Worktree, ForkKind::Shared, ForkKind::Conversation] {
        let fork = block_on(daemon.fork(&parent, options(kind, "Try another fix"))).expect("fork");
        let workspace = daemon.workspace();
        assert_eq!(workspace.session_files.get(&fork), Some(&before));
        assert_eq!(workspace.session_files.get(&parent), Some(&before));
    }
}

#[test]
fn a_second_agent_attaches_once_while_shells_are_independent() {
    use humata2_clients::{Terminals, model::Agent};
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let (first, _) = block_on(daemon.open(&id, Agent::Codex)).expect("codex starts");
    let (again, _) = block_on(daemon.open(&id, Agent::Codex)).expect("codex attaches");
    assert_eq!(first, again);
    let (main, _) = block_on(daemon.open(&id, Agent::Claude)).expect("main attaches");
    assert_eq!(main, 0);
    let (shell, _) = block_on(daemon.open(&id, Agent::Shell)).expect("shell starts");
    let (other, _) = block_on(daemon.open(&id, Agent::Shell)).expect("another shell starts");
    assert_ne!(shell, other);
    assert_eq!(
        daemon
            .workspace()
            .session(&id)
            .unwrap()
            .terminal_agents
            .len(),
        1
    );
}

#[test]
fn reading_a_file_uses_its_sessions_clone_and_reports_offline() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let text = block_on(daemon.read_file(&id, "api", "src/client.rs")).expect("file exists");
    assert!(text.contains("pub fn insert_entry"));
    assert!(matches!(
        block_on(daemon.read_file(&SessionId::new("brin"), "api", "src/client.rs")),
        Err(DaemonError::NotFound { .. })
    ));
    daemon.take_offline(&MachineId::new("devbox-1"));
    assert!(matches!(
        block_on(daemon.read_file(&id, "api", "src/client.rs")),
        Err(DaemonError::Offline { .. })
    ));
}

#[test]
fn approving_an_item_transition_moves_only_its_item_and_denying_keeps_it() {
    use humata2_clients::model::{Answer, ItemStatus};
    for (answer, expected) in [
        (Answer::Approved, ItemStatus::Done),
        (Answer::Denied { reply: None }, ItemStatus::InProgress),
    ] {
        let workspace = fixture::design();
        let request = workspace.item_details[&ItemId::new("PAY-3")]
            .request
            .clone()
            .expect("board approval");
        let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
        block_on(daemon.answer(&request, answer)).expect("answer accepted");
        let workspace = daemon.workspace();
        assert_eq!(
            workspace
                .items
                .iter()
                .find(|i| i.id.as_str() == "PAY-3")
                .unwrap()
                .status,
            expected
        );
        assert_eq!(
            workspace
                .items
                .iter()
                .find(|i| i.id.as_str() == "INF-2")
                .unwrap()
                .status,
            ItemStatus::InProgress
        );
        assert!(workspace.waiting().all(|r| r.id != request));
    }
}

#[test]
fn archive_undo_restores_tree_and_pin_without_rewinding_terminal_output() {
    let mut workspace = fixture::design();
    let id = SessionId::new("ledger");
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == id)
        .unwrap()
        .pinned = true;
    let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
    block_on(daemon.archive(&id, false)).expect("archive");
    assert!(daemon.workspace().session(&id).unwrap().archived);
    assert_ne!(
        daemon
            .workspace()
            .session(&SessionId::new("brin"))
            .unwrap()
            .parent,
        Some(id.clone())
    );
    block_on(daemon.send(&id, "kept output")).expect("output continues");
    let lines = daemon.workspace().session(&id).unwrap().transcript.clone();
    block_on(daemon.restore_session(&id)).expect("undo");
    let workspace = daemon.workspace();
    let restored = workspace.session(&id).unwrap();
    assert!(!restored.archived && restored.pinned);
    assert_eq!(restored.transcript, lines);
    assert_eq!(
        workspace.session(&SessionId::new("brin")).unwrap().parent,
        Some(id)
    );
}

#[test]
fn session_launch_choices_assign_the_item_and_keep_only_included_repositories() {
    use humata2_clients::model::{Agent, ChannelId, ItemStatus, SessionOptions, Speaker};
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let options = SessionOptions {
        model: Some("Opus".into()),
        repos: vec!["acme/ledger".into()],
        first_message: "Work on PAY-9\nKeep tests passing".into(),
        item: Some(ItemId::new("PAY-9")),
        branch: "backfill-progress-bar".into(),
    };
    let id = block_on(daemon.create_session(
        &ChannelId::new("payments-rewrite"),
        Agent::Claude,
        &MachineId::new("devbox-1"),
        "Backfill progress bar",
        options.clone(),
    ))
    .expect("start session");
    let workspace = daemon.workspace();
    let session = workspace.session(&id).unwrap();
    assert_eq!(workspace.session_options[&id], options);
    assert_eq!(session.repo, "acme/ledger");
    assert_eq!(session.branch, "backfill-progress-bar");
    assert!(
        session
            .transcript
            .iter()
            .any(|line| line.from == Speaker::You && line.text == options.first_message)
    );
    let item = workspace
        .items
        .iter()
        .find(|i| i.id.as_str() == "PAY-9")
        .unwrap();
    assert_eq!(item.session, Some(id));
    assert_eq!(item.status, ItemStatus::InProgress);
}

#[test]
fn adding_an_ssh_destination_reuses_it_and_rejects_a_list() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    assert!(matches!(
        block_on(daemon.add_machine("one two")),
        Err(DaemonError::Refused { .. })
    ));
    let id = block_on(daemon.add_machine("dev@newbox")).expect("added");
    assert_eq!(
        block_on(daemon.add_machine("dev@newbox")).expect("reused"),
        id
    );
    assert!(daemon.workspace().machine(&id).unwrap().online);
    assert_eq!(
        daemon
            .workspace()
            .machines
            .iter()
            .filter(|m| m.id == id)
            .count(),
        1
    );
}

/// SE-10/16: bytes go to the selected fake PTY; size and offline checks stay scoped.
#[test]
fn raw_terminal_input_waits_for_enter_and_cannot_type_while_offline() {
    use humata2_clients::Terminals;
    // One session sits on gpu-box, which the design fixture keeps offline.
    let mut workspace = fixture::design();
    let offline = SessionId::new("partial-brin");
    workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == offline)
        .unwrap()
        .machine = MachineId::new("gpu-box");
    let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let initial = daemon.workspace();
    let agent = initial.terminal_bytes[&(id.clone(), 0)].clone();
    block_on(daemon.input(&id, 1, b"pwd".to_vec())).unwrap();
    let before_enter = daemon.workspace().terminal_bytes[&(id.clone(), 1)].clone();
    assert!(before_enter.ends_with(b"pwd"));
    block_on(daemon.input(&id, 1, b"\r".to_vec())).unwrap();
    assert!(daemon.workspace().terminal_bytes[&(id.clone(), 1)].ends_with(b"~/humata\r\n$ "));
    assert_eq!(daemon.workspace().terminal_bytes[&(id.clone(), 0)], agent);
    block_on(daemon.resize(&id, 1, 73, 29)).unwrap();
    assert_eq!(daemon.workspace().terminal_sizes[&(id, 1)], (73, 29));
    let snapshot = daemon.workspace().terminal_bytes.clone();
    assert!(matches!(
        block_on(daemon.input(&offline, 0, b"x".to_vec())),
        Err(DaemonError::Offline { .. })
    ));
    assert_eq!(daemon.workspace().terminal_bytes, snapshot);
}

/// FK-2/10/11: code-only starts fresh; context forks retain the conversation.
#[test]
fn fork_kinds_keep_the_right_conversation_files_and_branch() {
    use humata2_clients::model::ForkKind;
    let mut workspace = fixture::design();
    let source = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "ledger")
        .unwrap();
    source.archived = true;
    source
        .terminal_agents
        .insert(1, humata2_clients::model::Agent::Codex);
    let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
    let parent = SessionId::new("ledger");
    let before = daemon.workspace();
    let source = before.session(&parent).unwrap();
    let original = &source.transcript[1];
    for kind in [ForkKind::Worktree, ForkKind::Conversation, ForkKind::Shared] {
        let id = block_on(daemon.fork(&parent, options(kind, "Another approach"))).unwrap();
        let workspace = daemon.workspace();
        let fork = workspace.session(&id).unwrap();
        assert_eq!(fork.parent.as_ref(), Some(&parent));
        assert_eq!(fork.transcript.contains(original), kind != ForkKind::Shared);
        assert_eq!(fork.branch == source.branch, kind == ForkKind::Conversation);
        assert!(fork.shells.is_empty() && fork.terminal_agents.is_empty());
        assert!(!fork.archived);
        assert_ne!(fork.started, source.started);
        assert_eq!(
            block_on(daemon.read_file(&id, "api", "src/client.rs")).unwrap(),
            block_on(daemon.read_file(&parent, "api", "src/client.rs")).unwrap()
        );
        assert!(
            workspace
                .activity
                .iter()
                .any(|a| a.kind == humata2_clients::model::ActivityKind::Fork
                    && a.session.as_ref() == Some(&id))
        );
        assert!(!workspace.terminal_bytes[&(id, 0)].is_empty());
    }
}
