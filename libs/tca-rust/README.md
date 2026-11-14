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
- **An exhaustive `TestStore`:** each step must change the state as the test
  says, with a line diff when it doesn't. Every action effects send must be
  received, and no effect may still run at the end. `non_exhaustive` checks
  only what a test asks about.

## How it differs from the Swift library

It borrows the ideas, not the Swift. There are no macros, key paths or
case paths: scoping is a function call. Effects run on `tokio`. A `Store`
makes a small runtime of its own, or runs on one it's given with `Store::on`.
The store reduces actions in batches and tells observers once per batch, so a
burst of actions costs one update.

## License

MIT
