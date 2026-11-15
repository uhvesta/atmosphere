//! A view observes only the part of the state it shows, and is told only
//! when that part changes: measured on a 10k-row list.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use tca::{Effect, Identified, IdentifiedVec, LocalStore, Observation, Reducer, Store};

const ROWS: u32 = 10_000;

#[derive(Clone, Debug, PartialEq)]
struct Row {
    id: u32,
    value: u64,
}

impl Identified for Row {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

#[derive(Debug)]
struct List {
    rows: IdentifiedVec<Row>,
    title: String,
}

fn list() -> List {
    List {
        rows: (0..ROWS).map(|id| Row { id, value: 0 }).collect(),
        title: String::new(),
    }
}

enum Action {
    Set(u32, u64),
    Rename(String),
}

struct Listing;

impl Reducer for Listing {
    type State = List;
    type Action = Action;

    fn reduce(&self, list: &mut List, action: Action) -> Effect<Action> {
        match action {
            Action::Set(id, value) => {
                list.rows.update(&id, |row| row.value = value);
            }
            Action::Rename(title) => list.title = title,
        }
        Effect::none()
    }
}

/// What the observers were told after they first were.
#[derive(Default)]
struct Told {
    rows: Vec<u32>,
    counts: Vec<usize>,
}

#[test]
fn changing_one_row_of_ten_thousand_tells_only_that_rows_observer() {
    let store = Store::new(list(), Listing).unwrap();
    let told = Arc::new(Mutex::new(Told::default()));
    let mut observations: Vec<Observation> = Vec::new();
    for id in 0..ROWS {
        let told = told.clone();
        observations.push(store.observe(
            move |list: &List| list.rows.get(&id).map(|row| row.value),
            move |_| told.lock().unwrap().rows.push(id),
        ));
    }
    let counts = told.clone();
    observations.push(store.observe(
        |list: &List| list.rows.len(),
        move |count| counts.lock().unwrap().counts.push(*count),
    ));
    // Told the title last, so once it's told every observer has looked.
    let (tell, titles) = mpsc::channel();
    observations.push(store.observe(
        |list: &List| list.title.clone(),
        move |title| tell.send(title.clone()).unwrap(),
    ));
    *told.lock().unwrap() = Told::default();
    assert_eq!(titles.recv_timeout(Duration::from_secs(2)).unwrap(), "");

    store.send(Action::Set(4_321, 7));
    // Setting a row to what it is changes nothing.
    store.send(Action::Set(10, 0));
    store.send(Action::Rename("done".into()));
    assert_eq!(
        titles.recv_timeout(Duration::from_secs(5)).unwrap(),
        "done"
    );

    let told = told.lock().unwrap();
    assert_eq!(told.rows, [4_321]);
    assert!(told.counts.is_empty());
}

#[test]
fn on_a_local_store_changing_one_row_tells_only_that_rows_observer() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = LocalStore::on(runtime.handle(), list(), Listing);
    let told = Rc::new(RefCell::new(Told::default()));
    let mut observations: Vec<Observation> = Vec::new();
    for id in 0..ROWS {
        let told = told.clone();
        observations.push(store.observe(
            move |list: &List| list.rows.get(&id).map(|row| row.value),
            move |_| told.borrow_mut().rows.push(id),
        ));
    }
    let counts = told.clone();
    observations.push(store.observe(
        |list: &List| list.rows.len(),
        move |count| counts.borrow_mut().counts.push(*count),
    ));
    *told.borrow_mut() = Told::default();

    store.send(Action::Set(9_999, 3));
    store.send(Action::Set(9_999, 3));
    store.send(Action::Set(0, 1));

    let told = told.borrow();
    assert_eq!(told.rows, [9_999, 0]);
    assert!(told.counts.is_empty());
}

#[test]
fn a_dropped_rows_observer_is_told_nothing() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = LocalStore::on(runtime.handle(), list(), Listing);
    let told = Rc::new(RefCell::new(Vec::new()));
    let seen = told.clone();
    let observing = store.observe(
        |list: &List| list.rows.get(&1).map(|row| row.value),
        move |value| seen.borrow_mut().push(*value),
    );
    drop(observing);
    store.send(Action::Set(1, 5));
    assert_eq!(*told.borrow(), [Some(0)]);
}
