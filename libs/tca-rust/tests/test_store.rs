use std::sync::Arc;
use std::time::Duration;

use tca::{
    Clock, Effect, Identified, IdentifiedVec, Reducer, TestClock, TestStore, for_each, if_let,
    reducer,
};

/// A file loader: loads take a while on the clock, a newer load replaces
/// one under way, and a load can be stopped.
#[derive(Clone, Debug, Default, PartialEq)]
struct Loader {
    loading: Option<String>,
    text: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Load {
    Open(String),
    Stop,
    Loaded(String),
}

struct Loading {
    clock: TestClock,
}

impl Reducer for Loading {
    type State = Loader;
    type Action = Load;

    fn reduce(&self, state: &mut Loader, action: Load) -> Effect<Load> {
        match action {
            Load::Open(name) => {
                state.loading = Some(name.clone());
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(1)).await;
                    Load::Loaded(format!("text of {name}"))
                })
                .cancellable("load", true)
            }
            Load::Stop => {
                state.loading = None;
                Effect::cancel("load")
            }
            Load::Loaded(text) => {
                state.loading = None;
                state.text = Some(text);
                Effect::none()
            }
        }
    }
}

fn loader() -> (TestStore<Loading>, TestClock) {
    let clock = TestClock::new();
    (
        TestStore::new(
            Loader::default(),
            Loading {
                clock: clock.clone(),
            },
        ),
        clock,
    )
}

#[test]
fn an_effects_action_is_received_and_reduced() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    clock.advance(Duration::from_secs(1));
    store.receive(Load::Loaded("text of a".into()), |s| {
        s.loading = None;
        s.text = Some("text of a".into());
    });
}

#[test]
fn a_newer_effect_with_the_same_id_replaces_the_one_running() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    store.send(Load::Open("b".into()), |s| s.loading = Some("b".into()));
    clock.advance(Duration::from_secs(1));
    store.receive(Load::Loaded("text of b".into()), |s| {
        s.loading = None;
        s.text = Some("text of b".into());
    });
    // Dropping checks nothing else came: "a" never loads.
}

#[test]
fn a_cancelled_effect_sends_nothing_and_stops_running() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    store.send(Load::Stop, |s| s.loading = None);
    clock.advance(Duration::from_secs(5));
    store.finish();
}

#[test]
#[should_panic]
fn a_change_the_test_didnt_expect_fails_it() {
    let (mut store, _clock) = loader();
    store.send(Load::Open("a".into()), |_| {});
}

#[test]
#[should_panic]
fn an_action_the_test_didnt_receive_fails_it() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    clock.advance(Duration::from_secs(1));
    store.finish();
}

#[test]
#[should_panic]
fn an_effect_still_running_at_the_end_fails_the_test() {
    let (store, _clock) = loader();
    let mut store = store.with_timeout(Duration::from_millis(50));
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    store.finish();
}

#[test]
#[should_panic]
fn receiving_the_wrong_action_fails_the_test() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    clock.advance(Duration::from_secs(1));
    store.receive(Load::Loaded("text of b".into()), |_| {});
}

#[test]
fn a_non_exhaustive_store_checks_only_what_the_test_asks() {
    let clock = TestClock::new();
    let mut store = TestStore::new(
        Loader::default(),
        Loading {
            clock: clock.clone(),
        },
    )
    .non_exhaustive();
    // Only says what loading became, not every field.
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    store.send(Load::Open("b".into()), |_| {});
    // Still running at the end, unreceived: let go.
}

#[test]
fn skipped_actions_are_still_reduced() {
    let (mut store, clock) = loader();
    store.send(Load::Open("a".into()), |s| s.loading = Some("a".into()));
    clock.advance(Duration::from_secs(1));
    store.skip_received();
    assert_eq!(store.state().text.as_deref(), Some("text of a"));
}

/// Effects in order and together.
#[test]
fn concatenated_effects_run_in_turn_and_merged_ones_all_run() {
    #[derive(Debug, PartialEq)]
    enum Step {
        Start,
        Did(u8),
    }
    let clock = TestClock::new();
    let waits = clock.clone();
    let steps = reducer(move |done: &mut Vec<u8>, step: Step| match step {
        Step::Start => {
            let (one, two) = (waits.clone(), waits.clone());
            Effect::concat([
                // The first takes longer, but the second waits for it.
                Effect::future(async move {
                    one.sleep(Duration::from_secs(2)).await;
                    Step::Did(1)
                }),
                Effect::future(async move {
                    two.sleep(Duration::from_secs(1)).await;
                    Step::Did(2)
                }),
            ])
            .and(Effect::send(Step::Did(0)))
        }
        Step::Did(n) => {
            done.push(n);
            Effect::none()
        }
    });
    let mut store = TestStore::new(Vec::new(), steps);
    store.send(Step::Start, |_| {});
    store.receive(Step::Did(0), |d| d.push(0));
    clock.advance(Duration::from_secs(2));
    store.receive(Step::Did(1), |d| d.push(1));
    clock.advance(Duration::from_secs(1));
    store.receive(Step::Did(2), |d| d.push(2));
}

#[test]
fn what_an_effect_sends_after_it_is_cancelled_is_dropped() {
    #[derive(Debug, PartialEq)]
    enum Tick {
        Start,
        Stop,
        Ticked,
    }
    let clock = TestClock::new();
    let ticks = clock.clone();
    let ticker = reducer(move |count: &mut u32, tick: Tick| match tick {
        Tick::Start => {
            let clock = ticks.clone();
            Effect::run(move |send| async move {
                loop {
                    clock.sleep(Duration::from_secs(1)).await;
                    send.send(Tick::Ticked);
                }
            })
            .cancellable("tick", false)
        }
        Tick::Stop => Effect::cancel("tick"),
        Tick::Ticked => {
            *count += 1;
            Effect::none()
        }
    });
    let mut store = TestStore::new(0, ticker);
    store.send(Tick::Start, |_| {});
    clock.advance(Duration::from_secs(1));
    store.receive(Tick::Ticked, |c| *c = 1);
    store.send(Tick::Stop, |_| {});
    clock.advance(Duration::from_secs(3));
    store.finish();
}

/// A list of rows, each with its own timer, and a sheet that may be open.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    id: u32,
    ticks: u32,
}

impl Identified for Row {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

#[derive(Debug, PartialEq)]
enum RowAction {
    Start,
    Stop,
    Ticked,
}

struct RowTimer {
    clock: Arc<dyn Clock>,
}

impl Reducer for RowTimer {
    type State = Row;
    type Action = RowAction;

    fn reduce(&self, row: &mut Row, action: RowAction) -> Effect<RowAction> {
        match action {
            RowAction::Start => {
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(1)).await;
                    RowAction::Ticked
                })
                .cancellable("timer", true)
            }
            RowAction::Stop => Effect::cancel("timer"),
            RowAction::Ticked => {
                row.ticks += 1;
                Effect::none()
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Board {
    rows: IdentifiedVec<Row>,
    sheet: Option<Row>,
}

#[derive(Debug, PartialEq)]
enum BoardAction {
    Row(u32, RowAction),
    Sheet(RowAction),
}

struct Boarding {
    row: RowTimer,
}

impl Reducer for Boarding {
    type State = Board;
    type Action = BoardAction;

    fn reduce(&self, board: &mut Board, action: BoardAction) -> Effect<BoardAction> {
        match action {
            BoardAction::Row(id, action) => for_each(&self.row, &mut board.rows, &id, action)
                .map(move |a| BoardAction::Row(id, a)),
            BoardAction::Sheet(action) => {
                if_let(&self.row, &mut board.sheet, action).map(BoardAction::Sheet)
            }
        }
    }
}

fn board() -> (TestStore<Boarding>, TestClock) {
    let clock = TestClock::new();
    let state = Board {
        rows: [Row { id: 1, ticks: 0 }, Row { id: 2, ticks: 0 }]
            .into_iter()
            .collect(),
        sheet: None,
    };
    let reducer = Boarding {
        row: RowTimer {
            clock: Arc::new(clock.clone()),
        },
    };
    (TestStore::new(state, reducer), clock)
}

#[test]
fn rows_effects_of_the_same_name_are_kept_apart() {
    let (mut store, clock) = board();
    store.send(BoardAction::Row(1, RowAction::Start), |_| {});
    store.send(BoardAction::Row(2, RowAction::Start), |_| {});
    // Stopping row 1's timer leaves row 2's.
    store.send(BoardAction::Row(1, RowAction::Stop), |_| {});
    clock.advance(Duration::from_secs(1));
    store.receive(BoardAction::Row(2, RowAction::Ticked), |b| {
        b.rows.update(&2, |row| row.ticks = 1);
    });
}

#[test]
fn an_action_for_a_closed_sheet_or_a_gone_row_does_nothing() {
    let (mut store, _clock) = board();
    store.send(BoardAction::Sheet(RowAction::Start), |_| {});
    store.send(BoardAction::Row(9, RowAction::Start), |_| {});
    store.finish();
}
