use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tca::{Effect, LocalStore, Reducer, StoreError};
use tokio::task::LocalSet;

/// Not `Send`: a local store may hold such state.
#[derive(Debug, Default)]
struct Counter {
    count: i64,
    log: Rc<RefCell<Vec<i64>>>,
}

enum Action {
    Add(i64),
    AddTwiceNow(i64),
    AddLater(i64),
    Forever(Arc<AtomicBool>),
}

/// Set once dropped.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Counting;

impl Reducer for Counting {
    type State = Counter;
    type Action = Action;

    fn reduce(&self, state: &mut Counter, action: Action) -> Effect<Action> {
        match action {
            Action::Add(by) => {
                state.count += by;
                state.log.borrow_mut().push(state.count);
                Effect::none()
            }
            Action::AddTwiceNow(by) => {
                Effect::send(Action::Add(by)).and(Effect::send(Action::Add(by)))
            }
            Action::AddLater(by) => Effect::future(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                Action::Add(by)
            }),
            Action::Forever(dropped) => {
                // Made now, so it's dropped even if the effect stops
                // before it first runs.
                let dropped = Dropped(dropped);
                Effect::run(move |_| async move {
                    let _dropped = dropped;
                    std::future::pending::<()>().await;
                })
            }
        }
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

#[test]
fn a_send_is_reduced_and_observers_told_before_it_returns() {
    let runtime = runtime();
    let store = LocalStore::on(runtime.handle(), Counter::default(), Counting);
    let told = Rc::new(RefCell::new(Vec::new()));
    let telling = told.clone();
    let _observing = store.observe(|s| s.count, move |count| telling.borrow_mut().push(*count));
    store.send(Action::Add(2));
    assert_eq!(store.with_state(|s| s.count), 2);
    assert_eq!(*told.borrow(), [0, 2]);
}

#[test]
fn what_a_reducer_sends_at_once_is_reduced_in_the_same_send_and_told_once() {
    let runtime = runtime();
    let store = LocalStore::on(runtime.handle(), Counter::default(), Counting);
    let told = Rc::new(RefCell::new(Vec::new()));
    let telling = told.clone();
    let _observing = store.observe(|s| s.count, move |count| telling.borrow_mut().push(*count));
    store.send(Action::AddTwiceNow(3));
    assert_eq!(store.with_state(|s| s.count), 6);
    assert_eq!(*told.borrow(), [0, 6]);
}

#[test]
fn an_observer_may_send_and_its_action_is_reduced_next() {
    let runtime = runtime();
    let store = LocalStore::on(runtime.handle(), Counter::default(), Counting);
    let inner = store.clone();
    let _observing = store.observe(
        |s| s.count,
        move |count| {
            if *count == 1 {
                inner.send(Action::Add(10));
            }
        },
    );
    store.send(Action::Add(1));
    assert_eq!(store.with_state(|s| s.count), 11);
    assert_eq!(*store.with_state(|s| s.log.clone()).borrow(), [1, 11]);
}

#[test]
fn what_effects_send_later_is_reduced_by_run() {
    let runtime = runtime();
    let local = LocalSet::new();
    local.block_on(&runtime, async {
        let store = LocalStore::new(Counter::default(), Counting).unwrap();
        tokio::task::spawn_local(store.run());
        let (tell, mut told) = tokio::sync::mpsc::unbounded_channel();
        let _observing = store.observe(|s| s.count, move |count| tell.send(*count).unwrap());
        store.send(Action::AddLater(4));
        assert_eq!(store.with_state(|s| s.count), 0);
        let mut last = 0;
        while last != 4 {
            last = tokio::time::timeout(Duration::from_secs(2), told.recv())
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(store.with_state(|s| s.count), 4);
    });
}

#[test]
fn dropping_the_store_ends_run_and_stops_its_effects() {
    let runtime = runtime();
    let local = LocalSet::new();
    local.block_on(&runtime, async {
        let store = LocalStore::new(Counter::default(), Counting).unwrap();
        let running = tokio::task::spawn_local(store.run());
        let dropped = Arc::new(AtomicBool::new(false));
        store.send(Action::Forever(dropped.clone()));
        // Let the effect and run start.
        tokio::task::yield_now().await;
        drop(store);
        tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        let start = std::time::Instant::now();
        while !dropped.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(2) {
            tokio::task::yield_now().await;
        }
        assert!(dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn a_store_made_outside_a_runtime_says_so() {
    let made = LocalStore::new(Counter::default(), Counting);
    assert!(matches!(made, Err(StoreError::NoRuntime { .. })));
}
