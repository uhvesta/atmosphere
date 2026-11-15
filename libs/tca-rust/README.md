# tca-rust

A Rust take on [The Composable Architecture](https://github.com/pointfreeco/swift-composable-architecture).
A feature is its state, the actions that change it, and a reducer. For each
action the reducer changes the state and returns the work that follows, an
`Effect`. A `Store` runs the reducer on a thread of its own and tells
observers when the part they watch changes. A `TestStore` runs it in tests and
checks every change.

The library is called `tca`:

```toml
[dependencies]
tca-rust = "0.1"
```

```rust
use tca::{reducer, Effect, TestStore};

#[derive(Debug, PartialEq)]
enum Action { Add(i64), Added }

let counter = reducer(|count: &mut i64, action: Action| match action {
    Action::Add(by) => {
        *count += by;
        Effect::future(async { Action::Added })
    }
    Action::Added => Effect::none(),
});
let mut store = TestStore::new(0, counter);
store.send(Action::Add(2), |count| *count = 2);
store.receive(Action::Added, |_| {});
```

## What it has

- **Effects:** `none`, `send`, `run` and `future`; `merge` and `concat`;
  `cancellable` with an id, `cancel`, and `debounce`; `map` to lift a
  child's actions into its parent's.
- **Composition with plain code:** a parent hands a child its part of the
  state. `for_each` runs a child on one row of an `IdentifiedVec`, and keeps
  each row's cancel ids apart. `if_let` runs a child on optional state, such as
  a sheet.
- **Dependencies on the reducer:** a reducer is a struct that holds its clock,
  client or store, so a test builds it with fakes. `TestClock` moves time by
  hand.
- **`IdentifiedVec`:** rows found by id in constant time, and `changes` lists
  the removals, insertions and updates from one version to the next, moving
  as few rows as it can, for a list view to apply.
- **Two stores:** a `Store` takes actions from any thread and reduces them
  on its own task. A `LocalStore` stays on the thread that made it, such as a
  UI thread: `send` reduces the action and tells observers before it returns,
  with no channel or lock, and its state needn't be `Send`. What its effects
  send later is reduced by its `run` future, on that same thread.
- **Navigation:** `PresentationState` and `PresentationAction` hold a
  sheet or popover; `if_presented` runs the child on it, drops actions that
  come once it's gone and cancels its effects when it's dismissed, from the
  parent or by the child returning `Effect::dismiss()`. `StackState`,
  `StackAction` and `for_each_in_stack` do the same for a stack of screens:
  popping one pops those above it and stops their effects. `AlertState` and
  `alert` describe an alert and its buttons as data.
- **Shared state:** `Shared<T>` is one value many features read. Readers
  share it rather than copy it, a change made through any of them is seen by
  all, `subscribe` tells a callback of each change until its `Observation`
  is dropped, and `changes` is an effect that sends an action per change.
- **`Dependencies`:** clients found by type. The app builds the live set
  once; a test overrides any of them with `with` and builds the feature
  from that, so it never reaches the live client. A missing one is a
  `DependencyError` when the feature is made, not when it runs.
- **Persistence:** a `Storage` keeps bytes by key (`MemoryStorage` for
  tests, `FileStorage` writing each key to a file in a folder, replaced
  whole through a temporary file and a rename). A `Codec` turns a value into
  bytes. `Persistence` restores a value, falling back to the default with
  the reason when it's missing or can't be read, and saves it after changes
  pause. `Persisting` wraps a reducer to save the part of its state it's
  told to whenever that part changes.
- **Streams:** `Effect::stream` opens an `EventStream`, such as a `tokio`
  channel of results, sends an action per item and one when it ends
  (`StreamEnd`). Keep the last item's cursor in state and open the next
  stream from it to resume after a drop. Cancelling the effect drops the
  stream. A stream waits while `EFFECT_QUEUE` of what effects sent is still
  waiting for the reducer, so a fast server slows to the reducer's pace
  (over gRPC, through flow control) instead of filling memory. An effect of
  your own that sends as fast as something else produces awaits
  `Sender::ready` before taking more. What the UI sends never waits and is
  never dropped: a send can't block the thread it's on, and people act far
  slower than reducers reduce.
- **Fine-grained observation:** observers watch a part of the state chosen
  by a selector, and are told only when that part changes, so a row's view
  hears of its row, not of the other ten thousand.
- **An exhaustive `TestStore`:** each step must change the state as the test
  says, with a line diff when it doesn't. Every action effects send must be
  received, and no effect may still run at the end. `non_exhaustive` checks
  only what a test asks about: its expectations apply to the state after
  the action, and actions it doesn't ask for are reduced and let go.
  `try_send`, `try_receive` and `try_finish` return a `TestFailure` instead
  of panicking, and `settle` runs the effects that are ready, as after a
  `TestClock` is advanced.

## How it differs from the Swift library

It borrows the ideas, not the Swift. There are no macros, key paths or
case paths: scoping is a function call. Effects run on `tokio`. A `Store`
makes a small runtime of its own, or runs on one it's given with `Store::on`.
The store reduces actions in batches and tells observers once per batch, so a
burst of actions costs one update.

## License

MIT
