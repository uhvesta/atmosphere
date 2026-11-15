//! The test store's two modes, and what it says when a test fails.

use std::time::Duration;

use tca::{Effect, Reducer, TestFailure, TestStore};

#[derive(Clone, Debug, Default, PartialEq)]
struct Counter {
    count: i64,
    label: String,
    log: Vec<String>,
}

#[derive(Debug, PartialEq)]
enum Action {
    /// Adds, then logs that it did, then logs again.
    Add(i64),
    Logged(String),
    Rename(String),
    /// Runs until cancelled.
    Wait,
}

struct Counting;

impl Reducer for Counting {
    type State = Counter;
    type Action = Action;

    fn reduce(&self, counter: &mut Counter, action: Action) -> Effect<Action> {
        match action {
            Action::Add(by) => {
                counter.count += by;
                Effect::concat([
                    Effect::future(async move { Action::Logged(format!("added {by}")) }),
                    Effect::future(async { Action::Logged("again".into()) }),
                ])
            }
            Action::Logged(line) => {
                counter.log.push(line);
                Effect::none()
            }
            Action::Rename(label) => {
                counter.label = label;
                Effect::none()
            }
            Action::Wait => Effect::run(|_| std::future::pending()),
        }
    }
}

fn store() -> TestStore<Counting> {
    TestStore::new(Counter::default(), Counting).with_timeout(Duration::from_millis(50))
}

fn marked(diff: &str, mark: char) -> Vec<String> {
    diff.lines()
        .filter(|line| line.starts_with(mark))
        .map(|line| line[1..].trim().to_owned())
        .collect()
}

#[test]
fn an_exhaustive_store_passes_when_every_change_and_action_is_asserted() {
    let mut store = store();
    store.send(Action::Add(2), |c| c.count = 2);
    store.receive(Action::Logged("added 2".into()), |c| {
        c.log = vec!["added 2".into()]
    });
    store.receive(Action::Logged("again".into()), |c| {
        c.log.push("again".into())
    });
    store.send(Action::Rename("n".into()), |c| c.label = "n".into());
    store.finish();
}

#[test]
fn a_wrong_expectation_fails_with_a_diff_of_only_what_differs() {
    let mut store = store();
    let failure = store
        .try_send(Action::Rename("total".into()), |c| c.label = "count".into())
        .unwrap_err();
    let TestFailure::StateMismatch { diff, .. } = failure else {
        panic!("expected a state mismatch, got {failure:?}");
    };
    assert_eq!(marked(&diff, '-'), [r#"label: "count","#]);
    assert_eq!(marked(&diff, '+'), [r#"label: "total","#]);
}

#[test]
fn a_change_left_out_of_the_expectation_fails_an_exhaustive_store() {
    let mut store = store();
    let failure = store.try_send(Action::Add(1), |_| {}).unwrap_err();
    assert!(matches!(failure, TestFailure::StateMismatch { .. }));
}

#[test]
#[should_panic]
fn a_failing_step_panics_when_not_tried() {
    let mut store = store();
    store.send(Action::Rename("total".into()), |c| c.label = "count".into());
}

#[test]
fn sending_before_receiving_what_effects_sent_fails_an_exhaustive_store() {
    let mut store = store();
    store.send(Action::Add(1), |c| c.count = 1);
    let failure = store
        .try_send(Action::Rename("n".into()), |c| c.label = "n".into())
        .unwrap_err();
    let TestFailure::Unreceived { actions, .. } = failure else {
        panic!("expected unreceived actions, got {failure:?}");
    };
    assert!(!actions.is_empty());
}

#[test]
fn receiving_out_of_order_fails_an_exhaustive_store() {
    let mut store = store();
    store.send(Action::Add(1), |c| c.count = 1);
    let failure = store
        .try_receive(Action::Logged("again".into()), |_| {})
        .unwrap_err();
    assert!(matches!(failure, TestFailure::WrongAction { .. }));
}

#[test]
fn expecting_an_action_that_never_comes_fails() {
    let mut store = store();
    let failure = store
        .try_receive(Action::Logged("never".into()), |_| {})
        .unwrap_err();
    assert!(matches!(failure, TestFailure::NothingReceived { .. }));
}

#[test]
fn an_effect_still_running_at_the_end_fails_an_exhaustive_store() {
    let mut store = store();
    store.send(Action::Wait, |_| {});
    let failure = store.try_finish().unwrap_err();
    assert!(matches!(failure, TestFailure::StillRunning { count: 1, .. }));
}

#[test]
fn actions_left_unreceived_at_the_end_fail_an_exhaustive_store() {
    let mut store = store();
    store.send(Action::Add(1), |c| c.count = 1);
    let failure = store.try_finish().unwrap_err();
    assert!(matches!(failure, TestFailure::Unreceived { .. }));
}

#[test]
fn a_non_exhaustive_store_skips_actions_and_state_the_test_doesnt_assert() {
    let mut store = store().non_exhaustive();
    // The log changes too, unasserted.
    store.send(Action::Add(3), |c| c.count = 3);
    // The first log line is skipped, though still reduced. A
    // non-exhaustive step's expectation applies to the state as it is
    // after the action, so it sets what it checks.
    store.receive(Action::Logged("again".into()), |c| c.count = 3);
    assert_eq!(store.state().log, ["added 3", "again"]);
    // Sends with what effects sent still unreceived are fine.
    store.send(Action::Add(1), |c| c.count = 4);
    store.send(Action::Wait, |_| {});
    // Effects still run and actions are unreceived: let go.
    store.finish();
}

#[test]
fn a_non_exhaustive_store_still_fails_on_what_the_test_does_assert() {
    let mut store = store().non_exhaustive();
    let failure = store.try_send(Action::Add(3), |c| c.count = 4).unwrap_err();
    let TestFailure::StateMismatch { diff, .. } = failure else {
        panic!("expected a state mismatch, got {failure:?}");
    };
    assert_eq!(marked(&diff, '-'), ["count: 4,"]);
    assert_eq!(marked(&diff, '+'), ["count: 3,"]);
}
