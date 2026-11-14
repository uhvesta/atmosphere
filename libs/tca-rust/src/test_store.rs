//! A store for tests that checks every change: each action sent or
//! received must change the state as the test says, every action effects
//! send must be received, and no effect may still be running at the end.

use std::fmt;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::diff::diff;
use crate::effect::Sender;
use crate::reducer::Reducer;
use crate::runner::Runner;

/// Runs a reducer as a [`Store`](crate::Store) would, checking each step.
/// It runs effects on a runtime of its own, so use it from plain
/// `#[test]`s, not `#[tokio::test]`s. Move time with a
/// [`TestClock`](crate::TestClock) given to the reducer.
///
/// ```
/// use tca::{reducer, Effect, TestStore};
///
/// #[derive(Debug, PartialEq)]
/// enum Action { Add(i64), Added }
///
/// let counter = reducer(|count: &mut i64, action: Action| match action {
///     Action::Add(by) => {
///         *count += by;
///         Effect::future(async { Action::Added })
///     }
///     Action::Added => Effect::none(),
/// });
/// let mut store = TestStore::new(0, counter);
/// store.send(Action::Add(2), |count| *count = 2);
/// store.receive(Action::Added, |_| {});
/// ```
pub struct TestStore<R: Reducer>
where
    R::State: Clone + PartialEq + fmt::Debug,
    R::Action: fmt::Debug + Send + 'static,
{
    reducer: R,
    state: R::State,
    runner: Runner,
    send: Sender<R::Action>,
    received: mpsc::UnboundedReceiver<R::Action>,
    timeout: Duration,
    exhaustive: bool,
    finished: bool,
    // Last, so effects are dropped before what they might use.
    runtime: Runtime,
}

impl<R: Reducer> TestStore<R>
where
    R::State: Clone + PartialEq + fmt::Debug,
    R::Action: fmt::Debug + Send + 'static,
{
    /// A test store starting at `state`.
    ///
    /// # Panics
    ///
    /// If its runtime can't be made.
    pub fn new(state: R::State, reducer: R) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("couldn't make the test store's runtime: {error}"));
        let (actions, received) = mpsc::unbounded_channel();
        let send = Sender::new(move |action| {
            let _ = actions.send(action);
        });
        Self {
            reducer,
            state,
            runner: Runner::new(runtime.handle().clone()),
            send,
            received,
            timeout: Duration::from_secs(1),
            exhaustive: true,
            finished: false,
            runtime,
        }
    }

    /// How long [`receive`](Self::receive) and [`finish`](Self::finish)
    /// wait for effects, a second unless set.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Checks only what the test asks about: a step's change holds if what
    /// it sets is so, actions the test doesn't receive are reduced and let
    /// go, and effects may still run at the end. For tests of one part of
    /// a large feature.
    pub fn non_exhaustive(mut self) -> Self {
        self.exhaustive = false;
        self
    }

    /// The state as it is now.
    pub fn state(&self) -> &R::State {
        &self.state
    }

    /// The reducer, for its dependencies.
    pub fn reducer(&self) -> &R {
        &self.reducer
    }

    /// Sends `action`, checking it changes the state as `expect` does to
    /// the state before it.
    ///
    /// # Panics
    ///
    /// If the state isn't as expected, or effects had sent actions the
    /// test hasn't received.
    #[track_caller]
    pub fn send(&mut self, action: R::Action, expect: impl FnOnce(&mut R::State)) {
        self.settle();
        if self.exhaustive {
            let waiting = self.waiting();
            if !waiting.is_empty() {
                panic!(
                    "sent {action:?} before receiving what effects sent: {waiting:?}. Receive those first, \
                     or make the store non-exhaustive."
                );
            }
        }
        let what = format!("sending {action:?}");
        self.reduce(action, expect, &what);
    }

    /// Waits for the next action effects send, checking it's `expected`
    /// and changes the state as `expect` does. A non-exhaustive store lets
    /// other actions go first.
    ///
    /// # Panics
    ///
    /// If none comes in time, another comes, or the state isn't as
    /// expected.
    #[track_caller]
    pub fn receive(&mut self, expected: R::Action, expect: impl FnOnce(&mut R::State))
    where
        R::Action: PartialEq,
    {
        let shown = format!("{expected:?}");
        self.receive_matching(&shown, |action| *action == expected, expect);
    }

    /// Like [`receive`](Self::receive), for actions that can't be compared:
    /// waits for the next action, checking `matches` says it's the one
    /// called `name`.
    #[track_caller]
    pub fn receive_matching(
        &mut self,
        name: &str,
        matches: impl Fn(&R::Action) -> bool,
        expect: impl FnOnce(&mut R::State),
    ) {
        loop {
            let Some(action) = self.next() else {
                panic!(
                    "expected to receive {name}, but no action came in {:?}. If it comes later, \
                     give the store a longer timeout; if it waits on a TestClock, advance the clock first.",
                    self.timeout
                );
            };
            if matches(&action) {
                let what = format!("receiving {action:?}");
                self.reduce(action, expect, &what);
                return;
            }
            if self.exhaustive {
                panic!("expected to receive {name}, but received {action:?}.");
            }
            let effect = self.reducer.reduce(&mut self.state, action);
            self.start(effect);
        }
    }

    /// Reduces actions effects sent without checking them, until none
    /// comes for a moment.
    pub fn skip_received(&mut self) {
        self.settle();
        while let Ok(action) = self.received.try_recv() {
            let effect = self.reducer.reduce(&mut self.state, action);
            self.start(effect);
        }
    }

    /// Stops the effects still running, for a test that's done with them.
    pub fn skip_in_flight(&mut self) {
        self.runner.cancel_all();
        self.settle();
    }

    /// Checks nothing is left: no action unreceived and, if exhaustive, no
    /// effect running once the timeout has passed. Dropping the store
    /// checks the same.
    ///
    /// # Panics
    ///
    /// If something is left.
    #[track_caller]
    pub fn finish(mut self) {
        self.check_finished();
    }

    #[track_caller]
    fn check_finished(&mut self) {
        self.finished = true;
        if self.exhaustive {
            let runner = self.runner.clone();
            let timeout = self.timeout;
            self.runtime.block_on(async {
                let _ = tokio::time::timeout(timeout, async {
                    while runner.running() > 0 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await;
            });
        } else {
            self.settle();
        }
        if !self.exhaustive {
            return;
        }
        let waiting = self.waiting();
        if !waiting.is_empty() {
            panic!(
                "effects sent actions the test didn't receive: {waiting:?}. Receive them, or skip_received()."
            );
        }
        let running = self.runner.running();
        if running > 0 {
            panic!(
                "{running} effects still ran after {:?}. Cancel them in the reducer, advance the test clock they \
                 wait on, or skip_in_flight() if the test is done with them.",
                self.timeout
            );
        }
    }

    #[track_caller]
    fn reduce(&mut self, action: R::Action, expect: impl FnOnce(&mut R::State), what: &str) {
        let mut expected = self.state.clone();
        let effect = self.reducer.reduce(&mut self.state, action);
        if !self.exhaustive {
            expected = self.state.clone();
        }
        expect(&mut expected);
        if expected != self.state {
            panic!(
                "{what} left the state other than expected (- expected, + actual):\n{}",
                diff(&format!("{expected:#?}"), &format!("{:#?}", self.state))
            );
        }
        self.start(effect);
    }

    /// Starts `effect` and lets it run until it waits, so what it waits on,
    /// such as a test clock, knows of it before the test goes on.
    fn start(&mut self, effect: crate::Effect<R::Action>) {
        {
            let _runtime = self.runtime.enter();
            self.runner.run(effect, &self.send);
        }
        self.settle();
    }

    /// The next action effects send, waiting up to the timeout.
    fn next(&mut self) -> Option<R::Action> {
        let timeout = self.timeout;
        let received = &mut self.received;
        self.runtime.block_on(async {
            tokio::time::timeout(timeout, received.recv())
                .await
                .ok()
                .flatten()
        })
    }

    /// Lets effects that are ready run until they wait.
    fn settle(&mut self) {
        self.runtime.block_on(async {
            for _ in 0..64 {
                tokio::task::yield_now().await;
            }
        });
    }

    /// Takes the actions effects have sent so far.
    fn waiting(&mut self) -> Vec<R::Action> {
        let mut waiting = Vec::new();
        while let Ok(action) = self.received.try_recv() {
            waiting.push(action);
        }
        waiting
    }
}

impl<R: Reducer> Drop for TestStore<R>
where
    R::State: Clone + PartialEq + fmt::Debug,
    R::Action: fmt::Debug + Send + 'static,
{
    fn drop(&mut self) {
        if !self.finished && !std::thread::panicking() {
            self.check_finished();
        }
        self.runner.cancel_all();
    }
}
