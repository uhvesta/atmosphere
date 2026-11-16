# Remote GPUI terminal client

`RemoteWorkspace` is shared by the native executable and browser Wasm entry
point. It creates actual `TerminalPane` / `TerminalView` entities over the same
Ghostty VT core. The transport adapter only delivers wire messages; it never
draws terminal output. This package contains no PTY holder, SSH implementation,
process launcher or host-filesystem access in its shared library.

The host owns terminal state, process lifetime and byte/resize ordering. A
client restores the opaque versioned Ghostty checkpoint, applies output offsets
exactly once, and applies `Resized` in host order. First layout requests
dimensions without speculatively resizing the replica. Gaps disable input and
request a new snapshot. Automatic VT replies are suppressed in replicas because
the authoritative host already answers them. User keys, paste and mouse reports
still become raw PTY input. A written acknowledgement means bytes reached the
PTY, not that an agent executed a task.

The native adapter uses Tokio/tungstenite; browser transport uses the browser
WebSocket API. Closing either viewer does not stop the host process. The
in-memory input queue is bounded, and uncertain input is never replayed after
disconnect. Both variants bundle the same Inter and JetBrains Mono fonts.

```sh
bazel run //apps/humata2/terminal/client:native -- \
  --url ws://127.0.0.1:8741/ws \
  --token-file "$HOME/.humata-terminal-demo-c1/browser.token"
bazel run //apps/humata2/terminal/client:native_fast -- \
  --url ws://127.0.0.1:8741/ws \
  --token-file "$HOME/.humata-terminal-demo-c1/browser.token"
bazel test //apps/humata2/terminal/client:replica_test \
  //apps/humata2/terminal/client:replica_fast_test \
  //apps/humata2/terminal/client:outbox_test
```

The replica test uses protocol messages and a mock socket around the actual
headless GPUI widget. It checks authoritative resize ordering, duplicated
output, missing-output resync, disabled input while catching up, query-reply
suppression and frontend clipboard paste. Live host/browser acceptance is in
`../web`.

Desktop and browser can view the same server-owned local or SSH session
simultaneously. The first viewer controls it; subsequent viewers remain
read-only until they choose **Take control**. Control transfer requests a fresh
fit for the new viewport before the host orders the resize. The process and its
durable recovery remain on the server.

Read-only mirrors retain the controller's shared grid size. Local viewport
scrollbars expose columns and rows larger than the mirror window without
resizing the process. Taking control returns to the normal fitted viewport and
requests its new size through the host's ordered resize stream.
Terminal mouse reports, focus clicks and hyperlink hints use the visible
ancestor clip. Overflowing grid cells cannot receive input through another pane.

Use `TerminalPane::focus_input` / `TerminalView::focus_input` for programmatic
terminal focus, including returning from find and selecting tabs. This API
installs the platform text handler in the same application input effect. A raw
`FocusHandle` handoff may wait for GPUI's next paint; stale unfocused terminal
handlers always reject committed text and composition.

Protocol 3 bounds compressed checkpoint data to 4 MiB and its decoded binary
checkpoint to 16 MiB. The host and every viewer must upgrade together. Terminal
checkpoints remain opaque Ghostty binary data; the wire codec handles bounded
LZ4 transport encoding with bounded zlib fallback before the shared client
restores them.

Memory saver is opt-in. With it enabled, after 30 seconds in an unselected tab
an idle replica detaches and releases its pane, VT and row caches. Both visible
panes remain resident, including an unfocused read-only mirror. Selecting a
parked tab requests a fresh host checkpoint before accepting input; the host's
PTY, parser and recovery policy continue throughout. The header toggle disables
further parking, and `set_memory_saver` lets an embedder choose its idle timeout.

Find, composition, selected text, pending paste confirmation, scrolled history
and local mirror pan pin the existing view. Input must have a host-written
acknowledgement or an explicit uncertain/refused outcome before detaching.
Rapid refocus waits for the host's ordered detach acknowledgement before opening
again, so a delayed snapshot from the old subscription cannot enable input.
The ordered host barrier makes unrelated socket traffic safe; bounded
connection backpressure can still defer parking. Lightweight sequence/offset
state stays in memory. No frontend snapshot cache or draft file is written. Diagnostics
report resident and parked counts; releasing these allocations does not promise
that the browser or native allocator returns its peak memory to the OS.

## Embedding one terminal

`terminal_replica::TerminalReplica` is the reusable single-terminal GPUI entity
(Bazel `:terminal_replica` / `:terminal_replica_fast`). The demo client also
reexports it as `terminal_client::replica`. The standalone library has no demo
resource, font or application engine dependency. It has no socket, permanent
polling loop, application chrome, font registration, global
theme change, process factory or notification side effect. The caller supplies
`ReplicaConfig` (trusted `SessionInfo`, terminal theme, optional idle timeout and
opt-in shortcut policy) and an ordered `ReplicaSink`. The same entity runs with
native GPUI, GPUI Fast and the browser GPUI Fast backend.

Bind each authenticated connection using `bind_connection(epoch, session, cx)`.
The caller owns connection epochs: a replacement transport gets a new epoch;
recreating an entity on the same transport retains its epoch and continuation
counters. A reused numeric route cannot bind a different terminal UUID. Apply
single-terminal protocol frames with `apply`; authentication and workspace lists
belong to the application. Report accepted-queue flushes with `outbound_flushed`
so bounded backpressure can retry acknowledgements and resynchronization.

`checkpoint(max_bytes, cx)` exports an owned `ReplicaCheckpoint` containing
versioned metadata, raw opaque VT bytes, output offset, dimensions and sequence
continuation. Callers choose their own serialization and compression.
`restore_checkpoint` validates before replacing the existing grid. It retains
the existing pane entity and find draft; invalid state leaves it untouched.
Restore while an Open or Detach is in flight returns `LifecycleBusy`; wait for
that ordered lifecycle operation first. A restored cache is read-only until
trusted matching metadata, a fresh host snapshot and control confirm authority.
No input payload, lease, clipboard contents or draft is exported or replayed.
The VT snapshot excludes per-viewer selection, scroll bookmark and composition.

`inject_text(text, InjectionOptions, cx)` returns an `InputTicket` only after the
entire encoded input was accepted. Raw and bracketed-paste encoding are explicit;
`submit` defaults to false. The 256 KiB input limit includes wrappers and submit.
An oversized or rejected input consumes no sequence number and sends no prefix.
`ReplicaEvent::InputQueued`, `InputWritten` and `InputUncertain` distinguish local
queue acceptance, PTY written acknowledgement and unknown outcome. They never
claim an agent has executed a command. Injection never takes control implicitly.

`set_visible` describes actual embedding visibility, independently of focus.
All visible mirrors stay resident. Memory saver is opt-in (`None` by default).
It uses one idle timer, then orders acknowledgement and Detach, retaining the VT
until the host's `Detached` barrier. This also works for a continuously emitting
inactive TUI. Refocus opens a fresh host checkpoint before enabling input.
Complex local state pins the view instead of silently discarding it. Allocation
release is observable through `ReplicaState::resident`; it does not guarantee
lower OS RSS.

Subscribe to `ReplicaEvent` for lifecycle, focused terminal UUID, typed pane
errors, bell, hyperlink, clipboard policy requests, supplied provider events and
injection receipts. `provider_event` forwards normalized caller-supplied events;
no provider signal implies PTY exit. The embedder owns notification policy.

`ShortcutPolicy` defaults to no host shortcuts. `register_host_shortcuts` opts an
application into `NewTerminal`, `CommandPalette`, `NextTerminal` and
`PreviousTerminal` bindings. A focused component emits a UUID-scoped request;
actions outside it bubble to the app, which owns last-focused tracking, tab
ordering and UX. Browser-reserved Cmd+T/Ctrl+Tab may never reach the page; the
library does not request keyboard lock or fullscreen. Uninstalled actions leave
terminal/TUI bindings available.

The standalone `component_test` and `component_fast_test` embed this entity
without `RemoteWorkspace` and exercise cache restore, trusted identity, ordered
lifecycle, input receipts, atomic limits and backpressure.

Asynchronous clipboard reads retain their platform error source. Permission
denial, unavailable secure-context/API support and unsupported contents produce
distinct `PaneError` values through the normal event hook. The demo surfaces
these as actionable notices. A late completion after focus moves never pastes
or reports an error into another terminal. The component performs no automatic
permission changes or operating-system notification.

Browser paste preserves input order across asynchronous clipboard access.
Following key presses, releases and committed text wait behind the clipboard
read in a bounded per-view queue (1,024 events / 1 MiB including queued payload
and event overhead). Multiple reads may finish out of order; their inputs still
flush in gesture order. A read failure, focus loss, connection/authority change
or grid replacement cancels queued input rather than replaying it later.

`Pty::write_accepted` reports local enqueue acceptance, separately from written
receipts. A rejected, oversized or confirmation-gated paste cancels buffered
following keys; Enter cannot overtake the paste or approve its confirmation.
Adapters with rejecting queues override this method and keep their own typed
error reporting. Existing adapters retain the compatible `write` default.
`inject_text` returns typed `PendingPaste` while a clipboard read or paste
confirmation is pending, without consuming an input sequence number.
