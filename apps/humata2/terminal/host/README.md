# Persistent native terminal host

This is the process and VT-state owner behind the generic gpui-kit viewers. It runs on macOS or Linux without a graphical session. Browser/native viewers attach to it; closing a view leaves the PTY, parser, alternate screen and history running on the host.

The host serves the built shared GPUI browser bundle over loopback HTTP and `/ws`. Browser and native viewers use the shared GPUI terminal component. Every viewer gets a versioned native Ghostty checkpoint and the subsequent raw output stream. The host alone answers terminal queries and applies resize/output in order; viewers suppress automatic VT replies.

## Build and start

```sh
export PATH="$PWD/.tools/bin:$PATH"
bazel build //apps/humata2/terminal/host:terminal_host
bazel run //apps/humata2/terminal/host:terminal_host -- \
  --state /tmp/my-terminal-host \
  --holder "$PWD/bazel-bin/apps/humata2/daemon/holder/humata2_holder" \
  --static-dir /absolute/path/to/built/browser/assets \
  --listen 127.0.0.1:8741 \
  --start shell
```

The `--state` directory must be new or private (0700), and separate from legacy raw-journal lab data. The default trusted `shell` profile runs the host user's `$SHELL -l` in their home directory. With a custom profile file, only its configured names can be launched; browser messages never contain arbitrary executable arguments.

`http://127.0.0.1:8741/` serves the bundle. `/health` returns `ok`. The private authentication token is at `<state>/browser.token` (0600); the server never prints it. The viewer sends `Authenticate {protocol: 3, token}` as its first WebSocket message. Up to 16 simultaneous WebSocket connections, including pending authentication, can mirror the same sessions. The first opener controls a session; other viewers are read-only. `TakeControl` explicitly transfers its input/resize lease and notifies both viewers. Disconnecting releases the lease without stopping the process. Native clients may omit Origin, but still require authentication. Browser Origin must match a loopback HTTP Host; untrusted origins are rejected.

## Trusted profiles

Pass `--profiles /private/path/profiles.json` and repeat `--start NAME` to ensure configured startup profiles exist. Profile files are trusted launcher configuration, not host-generated recovery records. Keep them private if their launch arguments are sensitive.

```json
[
  {
    "name": "shell",
    "title": "Zsh",
    "program": "/bin/zsh",
    "args": ["-i"],
    "cwd": "/path/to/workspace",
    "history": {"mode": "recent", "max_lines": 10000, "max_bytes": 4194304, "interval_seconds": 30}
  },
  {
    "name": "editor",
    "title": "Neovim",
    "program": "/usr/bin/nvim",
    "args": ["--clean", "-n", "-i", "NONE", "/path/to/file.rs"],
    "cwd": "/path/to/workspace"
  }
]
```

Optional `recovery` is an explicit object with `provider`, `session_id`, `working_directory`, and `state_paths`. These hints are supplied by the launcher/provider integration or authenticated settings, never inferred by scraping prompts, environment variables or terminal text. Live session titles retain the trusted profile/machine label as a prefix to any sanitized OSC2 runtime title. Valid OSC7 absolute paths and decoded file URIs update session cwd metadata without probing the browser filesystem or changing provider recovery hints. The host persists the trusted profile's name, explicit hints and sanitized session metadata; it does not serialize launch argv, environment or input events.

## Recovery and limits

Live state remains in the authoritative host even with zero viewers. Each local/remote holder retains a bounded in-memory output tail so the host can reconnect after a restart. Memory-only holders fail closed if the installed binary does not support that mode; raw PTY output is not duplicated into holder/frontend disk journals.

By default, recent native checkpoints are saved every 30 seconds with approximately 10,000 history lines and a strict 4 MiB encoded/compressed budget. Native history eviction is page-granular. Checkpoints are opaque Ghostty binary state, including screen/history, modes, parser continuation and links; they are compressed, private and atomically replaced. Protocol v3 carries a versioned LZ4/Zlib checkpoint envelope with base64 compressed bytes: at most 16 MiB decoded state and 4 MiB compressed data. Input/output bytes also use bounded base64; legacy integer arrays remain readable. Live native history has its own 32 MiB byte limit, independently of the compressed disk budget. A size or disk-write failure preserves the previous checkpoint. Recovery metadata reads are bounded to 16 MiB and 256 records. Decoding is bounded and rejects malformed/incompatible data instead of truncating it.

`--no-history` is a sticky host-wide opt-out: it overrides saved policies and removes existing screen checkpoints and interrupted checkpoint temporary files. Explicit recovery identifiers and paths remain available as private metadata. `--history-lines`, `--checkpoint-bytes`, and `--checkpoint-seconds` configure the default policy. Authenticated per-session `SetHistory` may change it, except a viewer cannot override a host started with `--no-history`.

SSH transport loss reattaches automatically to the same holder with persistent 1–30 second capped backoff and at most four concurrent attempts; four is not an attempt-count limit. Old connection acknowledgements retire before new transport tasks begin, and missing viewer acknowledgements become explicit input uncertainty. No shell is spawned and no input is resent.

A clean shutdown (SIGINT or SIGTERM) drains mechanical reply/resize acknowledgements for up to two seconds, then flushes bounded checkpoints and detaches transports while keeping holders alive. Before answering a terminal query, the host writes a private pending-reply marker; an actual holder acknowledgement advances its checksummed offset watermark. Replay suppresses replies already known to have reached the process and answers new queries emitted while the host was down. If a crash leaves a reply delivery ambiguous, the original holder remains alive and recovery preserves its last display with input disabled. These 21-byte records contain offsets/status only. Initial viewer snapshots wait until retained output has been parsed, so historical clipboard/bell events are restored as state rather than replayed into a viewer. Before applying a resize, the host writes a small private epoch guard; a durable checkpoint clears it only after the native resize completes. If the host crashes with an unresolved guard, recovery preserves the last verified display and disables live input rather than replaying bytes against an uncertain grid. The original holder remains alive; event-ordered resize replay is a future improvement. Each applied resize currently pays for a durable guard write; viewers coalesce unsent same-session resize requests around input/control barriers to avoid a window-drag backlog. A process cannot recover arbitrary memory after its machine reboots; provider/session hints support an explicit provider recovery action. If the holder's memory replay has a gap after the saved checkpoint, the host preserves the last valid screen and disables live input rather than pretending it reconstructed a complete TUI.

Output uses a 1 MiB unacknowledged per-viewer window, a 64-frame queue and a 36 MiB queued-payload budget held until WebSocket delivery. Slow clients receive `ResyncRequired` and request a fresh atomic checkpoint. Output stops before the frame queue fills, reserving eight slots for ordered resync/control/receipt traffic. A burst reaches `ResyncRequired` without disconnecting an otherwise healthy viewer or releasing its controller lease; the canonical terminal keeps parsing. A hard queue/budget exhaustion from other requests still evicts that viewer; socket writes time out after 15 seconds so stalled sockets release retained frames and permits. Payload-free host diagnostics distinguish queue, byte-budget and socket-write failures. Input acknowledgements mean bytes reached the PTY, not that an agent executed a request. Uncertain input is never automatically resent. Paste frames are limited to 256 KiB; oversize/backpressure is reported. Raw output is not decoded as HTML.

## Remote process host

Build and install the host and matching holder on the process machine:

```sh
bazel build --config=linux_arm64 \
  --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
  //apps/humata2/terminal/host:terminal_host \
  //apps/humata2/daemon/holder:humata2_holder
```

Run the gateway there on `127.0.0.1`, then use an authenticated SSH forward outside the viewer component, for example `ssh -N -L 8741:127.0.0.1:8741 HOST`. Serve the same browser bundle from that host. The browser machine opens its own forwarded localhost URL, preserving the browser's secure-context clipboard behavior. The browser renderer receives only protocol messages; it has no native PTY, process API or SSH keys.

## Verification

```sh
bazel test //apps/humata2/terminal/host:host_test --test_output=errors
```

Verified by Bazel: `host_test` passed 60 cases (three manual SSH cases are ignored in that target), `host_ssh_test` passed against the dedicated Lima fixture, and `terminal_host` built in the same invocation. `spawn_ssh_test` separately passed with distinct real shell PIDs, the same observed remote cwd and Linux OS, and the original process still alive. The restart SSH gate retained local and remote shell PIDs and exported state across backend restart. A dense colored Unicode fixture retained 9,598 history rows across restart: 8,716,963 raw bytes, 4,154,606 LZ4 wire bytes, and 4,154,789 LZ4 disk bytes; feed plus snapshot took 424 ms in that full test run. Opt codec-only measurements are documented in the codec package; full host timing includes PTY/process/test scheduling. The SSH transport now also reattaches automatically after outages with persistent capped backoff; see [transport decision and fault proof](TRANSPORT.md). Browser rendering/clipboard parity remains a separate shared-client acceptance gate. The host suite also covers a real gateway receiving and ACKing a 12,000-line plain-output burst, plus deterministic small-frame high-water resync without losing the controller lease.

Tests cover private/reused authentication, Origin validation, compressed bounded atomic recovery and opt-out removal, real WebSocket authentication, and a real PTY that needs a terminal query answered **before any viewer connects**. Simultaneous viewers receive identical process output; read-only input/resize are refused, explicit takeover preserves shell state, and controller disconnect releases the lease. Viewer replacement keeps the same live terminal state. Backend restart/checkpoint recovery is tested separately from browser reload; visual browser/native parity and remote rendering use the shared client's acceptance harness.

The SSH profile endpoint uses the existing session backend; both clients can mirror local and SSH sessions in the same host:

```json
"endpoint": {"Ssh": {"config": "/private/path/ssh.config", "destination": "trusted-host", "holder": "/tmp/h2-terminal-holder", "root": "/var/tmp/private-terminal-host"}}
```

The SSH configuration and key paths remain on the server. Reusing the same `--state` and `--start shell --start ssh` reattaches recorded sessions rather than duplicating startup profiles. Startup profile identity is reserved atomically with session metadata before launch; retrying after a separate settings-write failure reuses the same holder. Legacy known profile/ID mappings are bound to that identity without respawning. A stopped/dead profile is not automatically restarted; an explicit `Create` starts another instance.

After installing the matching memory-only holder in a dedicated trusted VM, run the manual same-server local+SSH check:

```sh
bazel test //apps/humata2/terminal/host:host_ssh_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_env=TERMINAL_LAB_SSH_HOST=lima-h2-terminal-c1 \
  --test_env=TERMINAL_LAB_SSH_HOLDER=/tmp/h2-terminal-holder \
  --test_output=errors
```

This verifies two viewers see both terminals, then restarts the native host and checks the same shell PIDs and exported state survived on both endpoints. It does not create a VM or modify SSH trust automatically.

## Native embedding APIs

`Host::start` prefers LZ4 screen archives and falls back to Zlib only when LZ4's actual encoded output exceeds the configured disk budget. Every archive records its codec, decoded/encoded lengths and checksums; the total artifact remains within the session budget. Legacy version-one Zlib archives remain readable, including strict truncation/trailing-data checks. `Host::start_with_compression(config, store, profiles, Arc<compression::Registry>, codec_id)` selects a strict single codec, including trusted custom implementations registered through the re-exported `compression` module. Reopening a custom archive requires its codec again; an unavailable or failing codec preserves the existing artifact and returns an explicit error. Wire compression is configured independently.

`Host::bind_provider(terminal_id, provider::TrustedContext)` is a native launcher capability. Its returned `ProviderBinding::ingest(ProviderEvent)` accepts supplied normalized hook events; no browser wire command can submit one. Pure adapters normalize documented structured payloads outside the host. Rebinding invalidates old handles. Only `SessionStarted` may change the explicit recovery identity; other events must match the current provider/session. Trusted paths come from the binding, and notification bodies are ephemeral. `Host::observe_provider(capacity)` is opt-in, bounded to sixteen observers and 1–1024 events per observer, and yields explicit `ProviderObservation::Lagged` counts rather than silently pretending complete delivery. There are no default OS notifications or provider configuration edits.

`Host::spawn_from(source_id, SpawnFromOptions)` opens a fresh sibling PTY without a UI dependency. Choose the same configured profile or a named trusted profile; the source's exact local/SSH endpoint always wins. The default `SpawnCwd::RequireObserved` requires canonical OSC7 directory metadata. `AllowLastKnown` explicitly accepts launch/saved metadata when no valid report exists; `Explicit(path)` supplies a known directory on that machine. `Spawned` returns the directory and `CwdSource` provenance. Shells without integration do not reveal every `cd`, so last-known metadata is never advertised as verified current cwd. The host does not inject `pwd` into a running TUI or probe a remote directory before launch; holder chdir failures identify the reserved new ID.

Siblings inherit the source history policy, including Off, unless explicitly overridden; host-wide Off remains sticky. Provider conversation IDs, pending input, history contents and process memory start empty. Profile arguments remain trusted configuration as supplied, including any explicit resume argument the launcher chose. A source must be live and hydrated, its profile must still be configured, and errors after reservation identify the new ID/launch stage so callers can inspect it before retrying. The original process is left intact.

Fresh SSH sibling verification uses the same trusted fixture environment as the restart test:

```sh
bazel test //apps/humata2/terminal/host:spawn_ssh_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=errors
```

## Durable native terminal handles

`Host::terminals()` returns sanitized records, including durable UUIDs. Reopen one through `Host::terminal(uuid)` to get a `TerminalHandle`; numeric `route()` values remain host-scoped. Workspace schema-three migration assigns legacy records UUIDs atomically without replacing their PTYs. `info()` exposes effective/automatic names and explicit caller attributes. `set_name` records a programmatic override; `set_name_with_source` also supports Manual, and `clear_name` restores automatic naming. Explicit names use last-writer-wins semantics; application authorization lives outside this core. Runtime OSC titles update the automatic name while preserving an override. Automatic title composition preserves the trusted prefix and respects the 512-byte UTF-8 limit, including blank legacy fallback labels.

Names are at most 512 bytes. Caller attributes are explicit non-secret string metadata: at most 32 keys, 64 bytes per key, 4,096 bytes per value and 32 KiB total. Bounds are checked before enqueueing and again before durable replacement. Input, process argv and environment are not inferred into attributes.

```rust,ignore
let record = host.terminals().await?.remove(0);
let terminal = host.terminal(&record.uuid).await?;
terminal.set_name("Backfill shell".into()).await?;
let queued = terminal.inject_text("ls".into(), InjectionOptions::raw()).await?;
let ticket = queued.written().await?; // actual holder ACK, not command/agent completion
```

`InjectionOptions::raw()` and `paste()` default to `submit: false`. Raw preserves supplied UTF-8 bytes; Paste uses the authoritative terminal's paste encoder and current bracketed-paste mode. Newlines already present remain subject to terminal interpretation. Set `submit: true` to append an explicit Enter after the encoded text. The default `inject_text` refuses another controller. `inject_text_with_control(..., ControlPolicy::TakeControl)` explicitly transfers control while that handle's writes are outstanding; the native lease releases after the last acknowledged write. Viewer reclaim is explicit. At most 128 native writes await acknowledgement globally, and encoded input retains the 256 KiB frame limit.

Queued acceptance and written acknowledgement are separate. Tickets contain the durable terminal UUID, a fresh per-Host epoch and a monotonically increasing sequence. They are never persisted or resent. Disconnect, process exit or host closure makes unacknowledged delivery uncertain; errors retain the original ticket and any available transport source. Reconnect by UUID and inspect state before deciding what to send next. A clean shutdown also drains native input acknowledgements within its bounded deadline.

`stop()` refuses another controller by default; `stop_with_control` makes takeover explicit. Its `QueuedStop` confirms ordered queue acceptance, and `info().state` confirms actual process exit. `TerminalHandle::spawn_from` is the handle form of fresh-sibling creation; `Spawned` includes both its numeric route and durable UUID. Dropping a handle or calling host shutdown preserves live holders; lifecycle stop is explicit. `QueuedStop` confirms acceptance, and callers observe `SessionState::Exited` separately.

The external-package consumer at `//apps/humata2/terminal/demo:native_api_test` passed its real-PTY behavior check and exercises these public APIs without private host commands or reading private workspace metadata. It owns its test shells and confirms both exit before cleanup.

Run the public SDK proof with:

```sh
bazel test //apps/humata2/terminal/demo:native_api_test --test_output=errors
```

The combined native verification gate built `terminal_host` and passed 58 Host cases, both real SSH targets and that external SDK consumer; a subsequent scoped authentication-diagnostics gate passed all 59 Host cases and rebuilt the binary. Read-side diagnostics record only whitelisted frame kinds, JSON error categories, typed WebSocket failure classes and numeric close codes. No input, tokens, private close reason or raw protocol-error description is logged.

Authentication rejection diagnostics distinguish deadline, EOF, WebSocket error, non-text, oversize, invalid JSON, wrong frame type, protocol mismatch and token mismatch using static categories only. The five-second initial-frame deadline and generic client error are unchanged. The classification test exercises all nine failure paths without exposing credential or frame contents.

See [TRANSPORT.md](TRANSPORT.md) for the evidence-based Mosh decision, automatic SSH reattachment policy, prolonged-outage verification and boundaries. Mosh is not installed or used as a binary host transport.
