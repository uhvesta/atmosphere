# Polling and blocking audit, 2026-11-14

Owner direction: daemons talking must never be a lag source; links push both
ways and nothing wastes CPU polling (SPEC TR-7–TR-10, D52). This audits the
production code (not tests) of `daemon/**`, `cli/**` and `libs/tca-rust` for
sleep and interval loops, repeated calls that wait for a change, unbounded
queues, blocking work on async threads, reconnects per call, and places that
should watch files. Line numbers are as of this audit. D54 logs the picks;
GRPC.md §2.1 lists the polls that remain.

Legend: **Fixed** here; **Open** with the fix, for the crate's owner.

## Links and keepalive

| Where | What | Cost | Status / fix |
| --- | --- | --- | --- |
| humatad `peer_link.rs` (Server::builder) | No HTTP/2 keepalive: a dead SSH link was noticed only by ssh's own ServerAlive or a failed write | a dead link lingers; the host can't tell | **Fixed**: `http2_keepalive_interval(30 s)`, `http2_keepalive_timeout(20 s)`; tests on the paused clock show an idle hour sends only PINGs (≤ 120) and an unanswered PING ends the link |
| humatad `serve.rs` (local socket server) | No keepalive | none: a closed Unix socket ends the connection at once | fine as is |
| machines `link.rs:272` | ssh `ServerAliveInterval=15`, `ServerAliveCountMax=4` | a wake every 15 s on every link, twice the 30 s TR-9 allows | **Open**: set 30 (and CountMax 2), or drop it and rely on the HTTP/2 PING |
| (no code yet) host-side peer client | No `Endpoint` dials the peer link yet | n/a | **Open**: when built, `http2_keep_alive_interval(30 s)`, `keep_alive_timeout(20 s)`, `keep_alive_while_idle(true)` |
| humatad `peer_link.rs` stdio | `tokio::io::stdin()`/`stdout()` hand each read and write to the blocking pool | a thread hop per frame; adds to every event's latency | **Open** (measured by `peer_link_latency_bench`): if the bench shows it, read stdin with `AsyncFd` on a non-blocking dup, keeping a fallback for takeover terminals that can't be made non-blocking |

## Waiting for an answer

| Where | What | Cost | Status / fix |
| --- | --- | --- | --- |
| cli `call.rs:235` `Ctx::wait` | Called `WaitForAnswer` (a server stream) as a unary call with `wait_seconds: 60` in a loop: each call returned only after the stream ended, so a terminal re-asked every 60 s and made one extra call after the answer | an RPC a minute while waiting; the answer came up to a stream late | **Fixed**: one stream held with `wait_seconds: u32::MAX`, keepalives skipped, returns on the first settled message; a stream that ends unanswered is an error |
| humatad `inbox.rs:517` `wait_for_answer` | Watch channel on the request (good); a 15 s keepalive interval while a wait is open | one wake per 15 s per open wait | kept: TP-12 promises the keepalive and MCP turns it into progress |
| mcp `server.rs:74` writer queue | `unbounded_channel` for every reply and notification | progress for a stalled agent piled up without end | **Fixed**: progress is skipped while 64 lines wait to be written; replies (one per agent request) are never dropped |
| humatad `cli_login.rs:313` | one `timeout(CONFIRM_WAIT, answered)` on a oneshot | none | fine |
| humatad `mcp.rs` `GrpcDaemon` | lazy channel, connected once | none | fine |
| cli `live.rs` | one channel for the process | none | fine |

## Blocking work on async threads

| Where | What | Cost | Status / fix |
| --- | --- | --- | --- |
| humatad `inbox.rs:447` `request_access` | `grant_target::ask_ahead_call` reads the store (SQLite) on the runtime thread | a stalled worker while SQLite works | **Fixed**: `spawn_blocking` |
| cli `commands/streams.rs:239` `files put` | `std::fs::read` of the whole file on the runtime, then every chunk queued at once | the whole file in memory, a stalled runtime on a big file | **Fixed**: hashed then streamed from `spawn_blocking` through a 2-deep queue |
| humatad `peer.rs:214` Hello | `EnrollmentKey::read` reads a 65-byte file synchronously, once per link | negligible | leave |
| humatad `identity.rs:196`, `serve.rs` startup | sync fs and store reads before serving | startup only | leave |
| humatad `terminal.rs`, `cli_login.rs` | store and `/proc` reads already in `spawn_blocking` | none | fine |
| store (all) | rusqlite is synchronous | callers must `spawn_blocking` | rule for every caller; terminal.rs and cli_login.rs do |

## Per-call reconnects

| Where | What | Cost | Status / fix |
| --- | --- | --- | --- |
| humatad `terminal.rs:160` `describe` | `ListTabs` opens a holder connection per tab, one after another, 2 s probe timeout each | O(tabs) connects and handshakes per list; a hung holder stalls the list 2 s | **Open** (holder + terminal wiring): keep one control connection per holder, with the tab's state pushed as a watch; meanwhile probe tabs concurrently (`join_all`) |
| humatad `terminal.rs:~640` `type_into` | attaches to the holder per call | a connect per call | **Open**: reuse that per-holder connection |
| humatad `bridge.rs:86–130` `connect_or_start` | 25 ms connect loop for 0.5 s, and up to 10 s after starting the daemon | startup only | **Open**: have the started daemon write a byte to a pipe once it listens, and wait on that |

## Streams: one message per event

| Where | What | Cost | Status / fix |
| --- | --- | --- | --- |
| cli `commands/streams.rs:73` attach | sent an `OutputAck` for every output message | a client message per output chunk, over the link | **Fixed**: one ack per quarter window (256 KiB by default); the window never fills |
| humatad `attach.rs` | one gRPC message per holder output event; bounded (64 out, 16 in) | many small frames on fast output | **Open**: coalesce queued output events up to ~64 KiB before sending |

## Polls in other crates (not edited here)

| Where | What it waits for | Cost | Fix |
| --- | --- | --- | --- |
| holder `server.rs:202`, `saved.rs:33` | `interval(SYNC_EVERY)` 500 ms forever per tab, a `spawn_blocking` each tick even with nothing new | 2 wakes and 2 thread hops a second per tab, idle | a `Notify` set where output is appended; the sync task waits on it, then debounces 500 ms, so an idle tab never wakes |
| holder `server.rs:159–161` `end_program` | 20 ms sleep loop until `exited` is set, up to 2 s | 100 wakes while a program hangs up | a `watch` on exit: `timeout(HANG_UP_GRACE, exited.wait_for(Option::is_some))` |
| holder `server.rs:221` | 50 ms sleep after a failed accept | only on errors | fine (error backoff) |
| holder `server.rs:436` | `recv_timeout(1 s)` on a std thread | one wake a second per tab on a blocking thread | wait without a timeout and end the thread by closing the sender |
| revive `agent.rs:283`, `306` | `interval(POLL)` 100 ms while verifying a resume: checks the conversation files' size and mtime (`read_dir` at `:250`, sync on the runtime) | 10 wakes a second and a directory scan each, per resuming agent | watch the conversation folder (the file-watch seam below) and wake on change; do the scan in `spawn_blocking` |
| revive `agent.rs:395–401` | `sleep(POLL)` loop probing the holder after SIGTERM, up to 5 s | a connect every 100 ms | the holder's event stream ending (or its pid's exit via kqueue/pidfd) |
| machines `system.rs:117–157` | `try_wait` plus `thread::sleep(POLL)` / `recv_timeout(POLL)`, 20 ms, per command | 50 wakes a second on a blocking thread per running command | a thread that calls `child.wait()` and sends on the same channel; then `recv_timeout(deadline)` only |
| machines `takeover.rs` | deadlines for a takeover | per takeover | fine where it's a single timeout |
| tca-rust `store.rs:105`, `local_store.rs:93` | `unbounded_channel` for actions | unbounded memory if a producer floods | bounded, or coalesce latest-value actions at the source |
| tca-rust `test_store.rs:301` | 1 ms sleep loop | test helper only | fine |

## Files to watch

No code watches agent conversation files, git state or `shared/` yet. TR-7
asks for notifications: a small seam (a `Watcher` trait with an
FSEvents/inotify implementation via the `notify` crate, and a fake for
tests) in its own crate, used by revive (conversation files), the git
status reader and `shared/` sync when they're built. Not added here: no
code in this audit's scope reads those files yet.

## Measured

- `humatad2_unit_test`: an idle link for an hour on the paused clock sends
  only PING frames, at most one per 30 s; an unanswered PING ends it.
- `peer_link_latency_bench` (manual, `-c opt`): WhoAmI round trip over a
  real `serve --stdio` child, 5000 calls on the M-series dev Mac
  (2026-11-14, `-c opt`): p50 158 µs, p90 370 µs, p99 1.8 ms, max 20.5 ms.
  The p99/max tail is the one to watch; the blocking-pool stdio hop is a
  suspect. Uplink, the event
  stream, isn't built, so event latency itself isn't measurable yet.
