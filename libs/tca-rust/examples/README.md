# Examples

Each file is one feature, with its reducers and the `TestStore` tests that
check it. Bazel builds and runs each one as `<name>_example_test`. They're
listed from simplest to most involved.

- [`reading_list.rs`](reading_list.rs): a child reducer on
  `&mut state.field`, its actions lifted with `Effect::map`.
- [`chat_composer.rs`](chat_composer.rs): a child given a projection of its
  parent's state and written back in part; a delegate action to the parent.
- [`search_box.rs`](search_box.rs): `debounce` on a `TestClock`; a newer query
  cancelling the search in flight.
- [`shared_settings.rs`](shared_settings.rs): `Shared<T>` read by several
  features; `changes` as an effect.
- [`library_client.rs`](library_client.rs): a client trait with a live and a
  fake implementation, chosen through `Dependencies`; an optimistic change
  rolled back.
- [`saved_preferences.rs`](saved_preferences.rs): `Persisting` with
  `MemoryStorage`, saving only part of the state; restoring it after a
  restart; a failed save as an action.
- [`request_inbox.rs`](request_inbox.rs): `IdentifiedVec` and `for_each`;
  each row's cancel ids kept apart; removing a row cancels its effect.
- [`file_preview.rs`](file_preview.rs): an optional child with `if_let`; a
  sheet with `PresentationState`, `if_presented` and `Effect::dismiss`, and
  the parent reacting to its save.
- [`attached_upload.rs`](attached_upload.rs): a child that isn't a view and
  lives only while attached, run as `Option` and as a presentation, side by
  side, to show what each does with its work.
- [`file_browser.rs`](file_browser.rs): `StackState` and `for_each_in_stack`;
  popping a screen cancels the screens above it.
- [`live_thread.rs`](live_thread.rs): `Effect::stream` resuming from a cursor
  kept in state; retries with backoff on a `TestClock`.
- [`app_root.rs`](app_root.rs): an optional app root; tabs; delegates passed
  up three levels; a sign-out `AlertState`; signing out stops every row's
  effect.
