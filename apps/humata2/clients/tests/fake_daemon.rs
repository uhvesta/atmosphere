//! The fake daemon behaves like the daemon the app will talk to: replies
//! wait on its clock, failures can be scripted, reconnecting a host brings
//! each session back one of three ways, and event queues stay bounded.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{ItemId, MachineId, Recovery, SessionId, Status};
use humata2_clients::{Daemon, DaemonError, EVENT_QUEUE, Event, FakeDaemon, Operation, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

#[test]
fn reconnecting_a_host_reattaches_resumes_or_explains_each_session() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let outcomes = block_on(daemon.reconnect(&MachineId::new("ci-runner")))
        .expect("the host reconnects");
    assert!(outcomes.contains(&(SessionId::new("s-tf"), Recovery::Reattached)));
    assert!(outcomes.contains(&(SessionId::new("s-runner"), Recovery::Resumed)));
    assert!(outcomes.iter().any(|(id, recovery)| id.as_str() == "s-backup"
        && matches!(recovery, Recovery::Recoverable { .. })));
    let workspace = daemon.workspace();
    assert!(workspace.machine(&MachineId::new("ci-runner")).is_some_and(|m| m.online));
    let tf = workspace.session(&SessionId::new("s-tf")).expect("s-tf exists");
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
    assert!(matches!(block_on(daemon.load()), Err(DaemonError::Scripted { .. })));
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
        let fork = block_on(daemon.fork(&parent, kind, "Try another fix")).expect("fork");
        let workspace = daemon.workspace();
        assert_eq!(workspace.session_files.get(&fork), Some(&before));
        assert_eq!(workspace.session_files.get(&parent), Some(&before));
    }
}
