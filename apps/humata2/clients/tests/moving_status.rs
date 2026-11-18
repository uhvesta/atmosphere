//! MV-4, MV-17 on the fakes: while a session moves it reads Moving, and its
//! `moving` names the target machine, the move's operation, the step in
//! progress and that step's progress, which only rises; its terminal takes no
//! input; once the agent resumes it reads Working or Idle on the new machine
//! with no `moving` left.

use std::sync::Arc;

use humata2_clients::model::{MachineId, MoveProgress, MoveStepState, Session, SessionId, Status};
use humata2_clients::{Daemon, Event, Events, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

/// A running session on This Mac in the standard workspace.
fn session_on_this_mac(daemon: &FakeDaemon) -> Session {
    daemon
        .workspace()
        .sessions
        .into_iter()
        .find(|session| {
            session.machine == MachineId::new("this-mac") && !session.archived && session.status.accepts_input()
        })
        .expect("the fixture has a running session on This Mac")
}

/// What the feed told of `id`, in order.
fn told(events: &mut Events, id: &SessionId) -> Vec<Session> {
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Ok(Event::Session(session)) = event
            && &session.id == id
        {
            seen.push(session);
        }
    }
    seen
}

#[test]
fn a_session_mid_move_reads_moving_to_its_target_with_rising_progress_then_resumes() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let session = session_on_this_mac(&daemon);
    assert_eq!(session.moving, None, "a session that isn't moving has no move progress");
    let to = MachineId::new("gpu-box");
    let mut events = block_on(daemon.events()).expect("events open");

    block_on(daemon.move_session(&session.id, &to)).expect("the move succeeds");

    let seen = told(&mut events, &session.id);
    let during: Vec<&Session> = seen.iter().filter(|told| told.status == Status::Moving).collect();
    assert!(!during.is_empty(), "the session read Moving during the move: {seen:?}");
    let moving: Vec<&MoveProgress> = during
        .iter()
        .map(|told| told.moving.as_ref().expect("a moving session carries its move's progress"))
        .collect();
    for progress in &moving {
        assert_eq!(progress.to, to, "it moves to gpu-box");
        assert!(!progress.step.is_empty(), "it names the step in progress");
        assert!(
            progress.total_units == 0 || progress.done_units <= progress.total_units,
            "progress never passes its total: {progress:?}"
        );
    }
    for pair in moving.windows(2) {
        assert_eq!(pair[0].operation, pair[1].operation, "one move, one operation");
        if pair[0].step == pair[1].step {
            assert!(pair[1].done_units >= pair[0].done_units, "a step's progress only rises: {pair:?}");
        }
    }
    assert!(
        moving.windows(2).any(|pair| pair[0].step == pair[1].step && pair[1].done_units > pair[0].done_units),
        "some step's progress rises while the move runs: {moving:?}"
    );
    assert!(
        during.iter().all(|told| !told.status.accepts_input()),
        "a moving session's terminal takes no input (MV-4)"
    );

    let last = seen.last().expect("the feed told of the session");
    assert!(
        matches!(last.status, Status::Working | Status::Idle),
        "the feed's last word is the resumed session: {:?}",
        last.status
    );
    assert_eq!(last.machine, to);
    assert_eq!(last.moving, None, "a resumed session has no move progress");
    let now = daemon
        .workspace()
        .sessions
        .into_iter()
        .find(|found| found.id == session.id)
        .expect("the session is still there");
    assert_eq!(now.machine, to, "it runs on gpu-box");
    assert!(matches!(now.status, Status::Working | Status::Idle), "it resumed: {:?}", now.status);
    assert_eq!(now.moving, None);
}

#[test]
fn moving_has_its_own_label_and_takes_no_input() {
    let label = Status::Moving.label();
    assert!(!label.is_empty());
    for other in [Status::Idle, Status::Working, Status::Offline, Status::Stopped] {
        assert_ne!(label, other.label(), "Moving reads unlike {other:?}");
    }
    assert!(!Status::Moving.accepts_input());
}

#[test]
fn every_moving_event_lists_every_step_in_order_with_one_running_and_the_done_ones_before_it() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let session = session_on_this_mac(&daemon);
    let mut events = block_on(daemon.events()).expect("events open");
    block_on(daemon.move_session(&session.id, &MachineId::new("gpu-box"))).expect("the move succeeds");

    let seen = told(&mut events, &session.id);
    let moving: Vec<&MoveProgress> = seen.iter().filter_map(|told| told.moving.as_ref()).collect();
    assert!(!moving.is_empty(), "the session read Moving during the move: {seen:?}");
    let titles: Vec<&str> = moving[0].steps.iter().map(|step| step.title.as_str()).collect();
    assert!(titles.len() > 1, "a move has several steps: {titles:?}");
    let mut running_at = Vec::new();
    for progress in &moving {
        let now: Vec<&str> = progress.steps.iter().map(|step| step.title.as_str()).collect();
        assert_eq!(now, titles, "every event lists the same steps in the same order");
        let states: Vec<MoveStepState> = progress.steps.iter().map(|step| step.state).collect();
        let running: Vec<usize> = (0..states.len()).filter(|at| states[*at] == MoveStepState::Running).collect();
        assert_eq!(running.len(), 1, "exactly one step runs at a time: {states:?}");
        let at = running[0];
        assert!(states[..at].iter().all(|state| *state == MoveStepState::Done), "done steps come first: {states:?}");
        assert!(states[at + 1..].iter().all(|state| *state == MoveStepState::Pending), "later steps wait: {states:?}");
        assert_eq!(progress.steps[at].title, progress.step, "the summary names the running step");
        running_at.push(at);
    }
    assert!(running_at.windows(2).all(|pair| pair[0] <= pair[1]), "the running step only moves forward: {running_at:?}");
    assert_eq!(running_at.last().copied(), Some(titles.len() - 1), "the move ends on its last step");
}
