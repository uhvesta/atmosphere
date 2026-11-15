//! State snapshots: a [`TestStore`] run written down as text.

use std::fmt::{Debug, Write as _};

use tca::{Reducer, TestStore};

use crate::diff;

/// Unchanged lines of the state shown around each change.
const CONTEXT: usize = 2;

/// Runs a [`TestStore`] and writes down each step: the action sent or
/// received, and the lines of the state (as `{:#?}` shows it) that the step
/// changed. Every step is still checked as the store checks it, so a
/// recorder fails where the store would.
///
/// ```ignore
/// let mut recorder = StateRecorder::new(TestStore::new(State::default(), Feature));
/// recorder.send(Action::Increment, |state| state.count = 1);
/// snapshots!()?.assert_text("increment", &recorder.finish());
/// ```
pub struct StateRecorder<R: Reducer>
where
    R::State: Clone + PartialEq + Debug,
    R::Action: Debug + Send + 'static,
{
    store: TestStore<R>,
    transcript: String,
}

impl<R: Reducer> StateRecorder<R>
where
    R::State: Clone + PartialEq + Debug,
    R::Action: Debug + Send + 'static,
{
    /// Starts the transcript with `store`'s state.
    pub fn new(store: TestStore<R>) -> Self {
        let mut transcript = String::from("initial state\n");
        for line in format!("{:#?}", store.state()).lines() {
            let _ = writeln!(transcript, "    {line}");
        }
        Self { store, transcript }
    }

    /// Sends `action` as [`TestStore::send`] does, and writes the step down.
    ///
    /// # Panics
    ///
    /// Where [`TestStore::send`] does.
    #[track_caller]
    pub fn send(&mut self, action: R::Action, expect: impl FnOnce(&mut R::State)) {
        let step = format!("send {action:?}");
        let before = self.state();
        self.store.send(action, expect);
        self.write_step(&step, &before);
    }

    /// Receives `expected` as [`TestStore::receive`] does, and writes the
    /// step down.
    ///
    /// # Panics
    ///
    /// Where [`TestStore::receive`] does.
    #[track_caller]
    pub fn receive(&mut self, expected: R::Action, expect: impl FnOnce(&mut R::State))
    where
        R::Action: PartialEq,
    {
        let step = format!("receive {expected:?}");
        let before = self.state();
        self.store.receive(expected, expect);
        self.write_step(&step, &before);
    }

    /// Receives the action `matches` picks, as
    /// [`TestStore::receive_matching`] does, and writes the step down as
    /// `name`.
    ///
    /// # Panics
    ///
    /// Where [`TestStore::receive_matching`] does.
    #[track_caller]
    pub fn receive_matching(
        &mut self,
        name: &str,
        matches: impl Fn(&R::Action) -> bool,
        expect: impl FnOnce(&mut R::State),
    ) {
        let step = format!("receive {name}");
        let before = self.state();
        self.store.receive_matching(name, matches, expect);
        self.write_step(&step, &before);
    }

    /// Finishes the store as [`TestStore::finish`] does, and returns the
    /// transcript, to check with
    /// [`Snapshots::assert_text`](crate::Snapshots::assert_text).
    ///
    /// # Panics
    ///
    /// Where [`TestStore::finish`] does.
    #[track_caller]
    pub fn finish(self) -> String {
        let Self { store, transcript } = self;
        store.finish();
        transcript
    }

    fn state(&self) -> String {
        format!("{:#?}", self.store.state())
    }

    fn write_step(&mut self, step: &str, before: &str) {
        let after = self.state();
        let changes = diff::render(&diff::lines(before, &after), CONTEXT, "    ");
        let _ = writeln!(self.transcript, "\n{step}");
        if changes.is_empty() {
            self.transcript.push_str("    (state unchanged)\n");
        } else {
            self.transcript.push_str(&changes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tca::Effect;

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Counter {
        count: i64,
        label: String,
    }

    #[derive(Debug)]
    enum Action {
        Add(i64),
        Nothing,
    }

    struct Feature;
    impl Reducer for Feature {
        type State = Counter;
        type Action = Action;
        fn reduce(&self, state: &mut Counter, action: Action) -> Effect<Action> {
            if let Action::Add(amount) = action {
                state.count += amount;
            }
            Effect::none()
        }
    }

    #[test]
    fn each_step_shows_its_action_and_only_what_it_changed() {
        let mut recorder = StateRecorder::new(TestStore::new(Counter::default(), Feature));
        recorder.send(Action::Add(2), |state| state.count = 2);
        recorder.send(Action::Nothing, |_| {});
        let transcript = recorder.finish();

        let add = transcript.find("Add(2)").unwrap();
        let nothing = transcript.find("Nothing").unwrap();
        assert!(add < nothing, "steps are in the order they were taken");

        let add_step = &transcript[add..nothing];
        assert!(add_step.contains("- ") && add_step.contains("count: 0"));
        assert!(add_step.contains("+ ") && add_step.contains("count: 2"));
        assert!(
            !add_step.lines().any(|line| line.starts_with("    -") && line.contains("label")),
            "a field the step left alone is not shown as changed"
        );
    }

    #[test]
    #[should_panic]
    fn a_step_the_store_rejects_still_fails() {
        let mut recorder = StateRecorder::new(TestStore::new(Counter::default(), Feature));
        recorder.send(Action::Add(1), |state| state.count = 5);
    }
}
