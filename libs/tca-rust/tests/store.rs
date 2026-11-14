use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use tca::{Effect, Reducer, Store, reducer};

#[derive(Debug, Default)]
struct Counter {
    count: i64,
    /// Changes with every action, unlike the count.
    actions: u64,
}

enum Action {
    Add(i64),
    Touch,
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
        state.actions += 1;
        match action {
            Action::Add(by) => {
                state.count += by;
                Effect::none()
            }
            Action::Touch => Effect::none(),
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

fn counter() -> Store<Counting> {
    Store::new(Counter::default(), Counting).unwrap()
}

/// Waits for `values` to tell `until`, returning all it told.
fn wait_for(values: &mpsc::Receiver<i64>, until: i64) -> Vec<i64> {
    let mut told = Vec::new();
    while let Ok(value) = values.recv_timeout(Duration::from_secs(2)) {
        told.push(value);
        if value == until {
            break;
        }
    }
    told
}

#[test]
fn an_observer_is_told_now_and_as_its_part_changes() {
    let store = counter();
    let (tell, told) = mpsc::channel();
    let _observing = store.observe(|s| s.count, move |count| tell.send(*count).unwrap());
    store.send(Action::Add(2));
    store.send(Action::AddLater(3));
    let values = wait_for(&told, 5);
    assert_eq!(values.first(), Some(&0));
    assert_eq!(values.last(), Some(&5));
    assert_eq!(store.with_state(|s| s.count), 5);
}

#[test]
fn an_observer_isnt_told_of_changes_to_other_parts() {
    let store = counter();
    let (tell, told) = mpsc::channel();
    let _observing = store.observe(|s| s.count, move |count| tell.send(*count).unwrap());
    for _ in 0..50 {
        store.send(Action::Touch);
    }
    store.send(Action::Add(1));
    assert_eq!(wait_for(&told, 1), [0, 1]);
}

#[test]
fn a_dropped_observation_is_told_nothing_more() {
    let store = counter();
    let (tell, told) = mpsc::channel();
    let observing = store.observe(|s| s.count, move |count| tell.send(*count).unwrap());
    store.send(Action::Add(1));
    assert_eq!(wait_for(&told, 1), [0, 1]);
    drop(observing);
    store.send(Action::Add(1));
    // Something after, to know the add was reduced.
    let (tell_after, told_after) = mpsc::channel();
    let _after = store.observe(|s| s.count, move |count| tell_after.send(*count).unwrap());
    let _ = wait_for(&told_after, 2);
    assert!(told.try_recv().is_err());
}

#[test]
fn dropping_the_store_stops_its_effects() {
    let store = counter();
    let dropped = Arc::new(AtomicBool::new(false));
    store.send(Action::Forever(dropped.clone()));
    // Let it start.
    let (tell, told) = mpsc::channel();
    let _observing = store.observe(|s| s.actions, move |n| tell.send(*n as i64).unwrap());
    let _ = wait_for(&told, 1);
    drop(_observing);
    drop(store);
    let start = std::time::Instant::now();
    while !dropped.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn a_store_runs_on_a_runtime_it_is_given() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let reduce = reducer(|count: &mut i64, by: i64| {
        *count += by;
        Effect::none()
    });
    let store = Store::on(runtime.handle(), 0, reduce);
    let (tell, told) = mpsc::channel();
    let _observing = store.observe(|c| *c, move |count| tell.send(*count).unwrap());
    store.send(4);
    assert_eq!(wait_for(&told, 4).last(), Some(&4));
}

#[test]
fn dropping_a_store_on_a_runtime_it_was_given_stops_its_effects() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let store = Store::on(runtime.handle(), Counter::default(), Counting);
    let dropped = Arc::new(AtomicBool::new(false));
    store.send(Action::Forever(dropped.clone()));
    let (tell, told) = mpsc::channel();
    let observing = store.observe(|s| s.actions, move |n| tell.send(*n as i64).unwrap());
    let _ = wait_for(&told, 1);
    drop(observing);
    drop(store);
    let start = std::time::Instant::now();
    while !dropped.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(dropped.load(Ordering::SeqCst));
}
