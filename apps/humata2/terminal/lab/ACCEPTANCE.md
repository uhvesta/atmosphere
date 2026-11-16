# Generic gpui-kit terminal acceptance

The owner’s target is a reusable terminal component, stress tested through native and browser hosts. `terminal/pane::TerminalPane` consumes any `Pty` implementation and emits host events. `terminal/embed::EmbeddedTerminal` is the original local/SSH durable-session adapter; `terminal_replica::Replica` provides the transport-neutral single-terminal component; `terminal/client::RemoteWorkspace` is optional demo chrome that shares those same entities between desktop and browser through `terminal/wire`. The persistent `terminal/host` owns real local PTYs and SSH connections, authoritative VT state, and bounded private checkpoints. Both viewers must mirror **that same server's sessions**, including SSH-backed terminals. The lab's sidebar/tab layout is host policy, not a required component layout. Native direct-session acceptance and browser/gateway acceptance are separate gates.

## Repeatable matrix

All build, test, format and benchmark commands go through Bazel. The normal and `_fast` targets exercise the respective GPUI backends. The system-app and SSH targets are manual: they deliberately need real installed applications or the dedicated VM. Keep GUI compilation/linking sandboxed and select `--strategy=TestRunner=local` only for acceptance execution.

| Condition | Evidence / gate |
|---|---|
| Real independent local processes, PID/environment identity, raw input and resize | `session:session_test`; `lab:app_test`, `lab:app_fast_test` |
| Fixed-width vertical shrink, grow and restore reaches the actual PTY and keeps its bottom row/cursor | Local `lab:app_test` and `lab:app_fast_test` passed; focused `lab:vertical_resize_ssh_test` and `lab:vertical_resize_ssh_fast_test` passed on real SSH, with unchanged shell PID/environment and active alternate screen |
| Actual browser vertical resize and subsequent reload preserve local/SSH session identity | `demo:fixture_runtime_test` with `TERMINAL_BROWSER_RESIZE=1`; optimized13 passed both Mac and Linux browsers, with real `stty` rows 34→18→45→34 at 77 columns and last-row RGB paint checks |
| Twelve simultaneous PTYs, isolated state and persisted tab layout | `session:session_test` (twelve-independent-terminals case) |
| Focus only sends raw keyboard input to its terminal; text field does not leak | `lab:app_test`, `lab:app_fast_test`; pane input/focus tests |
| Cmd+V to zsh, selected output via Cmd+C, Up/Down history | `lab:interactive_test`, `lab:interactive_fast_test` with actual owner zsh |
| Actual owner theme and plugins, colored `ls`, `cd`, Git status | Same manual interactive gates; captures under `tests/__snapshots__/interactive/` |
| Neovim RGB syntax, status line, mouse position and Up/Down keys | `lab:neovim_test`, `lab:neovim_fast_test`; interactive gates |
| Editing, font zoom, half/full window resize, unsaved-buffer resume, file write | Neovim gates; captures under `tests/__snapshots__/neovim/` |
| In-band resize negotiation (DEC 2048), with no further output to wake replies | `pane:inband_test`, `pane:inband_fast_test`; real Neovim gates |
| ANSI/256/RGB colors, Unicode, wide/combining characters and styled cells | `vt/...`, `view/...`, `pane/...` tests; actual shell/Neovim captures |
| OSC 8 hidden destinations; URLs through soft wraps; removal and scrolling | `vt:links_test`; `pane:links_test`, `pane:links_fast_test` |
| Modifier-click consumes the click rather than leaking it into a TUI | Pane link tests; host applies URL scheme policy |
| Text selection, Shift mouse override, scroll wheel, find, multiline/bracketed paste | `pane:pane_test`, `pane:pane_fast_test` |
| Drag across groups/reorder preserves the entity and actual process | App and interactive gates on each backend |
| 12,000 real output lines, Page Up, Top/Bottom, cached idle frames | Interactive gates; print observed output delivery and natural scroll-frame timing |
| Restore that large history from disk and reconnect its existing shell | Interactive gates (large-history replay case) |
| Frontend SIGKILL, reopened workspace and same shell PID | `session:session_test` (crash-client fixture) |
| Real Host or SSH tunnel SIGKILL reconnects the existing process; holder/child death does not silently replace it | `host:hard_kill_test` passed three local cases; `host:hard_kill_ssh_test` passed four SSH cases. Host/tunnel loss retains PID and UUID; holder loss preserves a read-only checkpoint; child loss reports exit without an unsolicited respawn |
| Partial final journal write vs complete corruption | `session:session_test`; partial tail repairs, complete corruption is preserved/reported |
| Competing workspace and frontend attachment owners | `session:session_test`; locks refuse a second owner |
| Metadata write failure rolls back memory and preserves saved layout | `session:session_test` (metadata-write-failure case) |
| Dead holder does not silently become a different shell | `session:session_test`; saved output remains inspectable |
| Real remote PTYs, remote resize, same PID after tunnel/frontend recreation | `session:ssh_test` in dedicated Lima VM |
| Remote output while disconnected catches up on reattach | SSH session gate |
| SSH Neovim + zsh mouse/keys, Cmd+C/V, movement, resume and 12k history | Interactive gates with `TERMINAL_LAB_SSH_CONFIG` |
| Ordinary SSH output copies exact Unicode/hard newlines without ANSI styling, then pastes back exact remote bytes | `lab:clipboard_ssh_test` and `lab:clipboard_ssh_fast_test` both passed, including the post-FIFO regression run (2.8/3.2 seconds). An owned raw remote PTY validates the actual copied value with bracketed-paste framing; an application-input paste stays outside the terminal. Uses isolated headless frontend clipboards, never the macOS clipboard |
| Every expected row keeps painting over idle GPU captures | Local/SSH app gates on both backends, eight successive captures |
| Binary checkpoint alternate screen/history, unfinished sequences, incompatible data | `vt:checkpoint_test` (8 tests passed after hardening) |
| Native64 → Wasm32 → native64, continued UTF-8 and 1,000 restore/free cycles | `vt:wasm_checkpoint_test` passed; standalone Wasm runtime, not yet GPUI browser certification |
| Host queries with zero viewers, auth/Origin, replay replies, bounded queues and recovery | An earlier `host:host_test` gate passed 60 active cases; the expanded Host regressions are included in the latest 52-target gate below. SSH cases remain separate manual gates. Includes historical-reply suppression, unanswered downtime queries, uncertain-reply guards, detach barriers and malformed/private-file recovery cases |
| Same local server owns local and SSH sessions; simultaneous viewers/control transfer | `host:host_ssh_test` passed one real SSH case; local/remote shell PIDs and exported state survive backend restart |
| Fresh SSH sibling has its own process, remote OS/cwd and native injection receipts | `host:spawn_ssh_test` passed one real SSH case against the dedicated Lima VM |
| Public native API works outside the Host package | `demo:native_api_test` passed one case using public handles, metadata/name/attribute APIs and injection receipts; its owned real PTYs exit before cleanup |
| Memory-only metadata excludes argv/input/output; legacy argument migration | `session:session_test` passed, including private-memory and migration cases |
| Durable UUID, automatic/manual/programmatic names, bounded attributes and atomic migration | `session:metadata_test` passed 21 cases; schema 1/2→3 migration preserves routes/layout, explicit overrides survive automatic updates, reset restores the automatic name, and failed writes roll back |
| Existing durable sessions and attachment ownership remain intact | `session:session_test` passed 14 cases, including an owned reconnect capability that outlives the store, resumes from its output offset and never replays input or starts a replacement; `session:session_unit_test` passed the inherited-descriptor ownership regression for memory-only and legacy journals |
| Opt-in agent recovery settles launch/shutdown races and respects explicit Stop across restart | Expanded `host:host_test` passes the paused-launch shutdown barrier, durable reservation/settings and native/viewer Stop restart cases; `host:agent_resume_codex_test` also passes with the actual installed Codex CLI |
| Real installed CLIs resume the exact saved conversation after SIGKILL and refuse missing UUIDs | `provider:codex_resume_test`, `provider:claude_resume_test`, `provider:copilot_resume_test`; isolated state and deterministic loopback inference, with committed context and structured session identity checks |
| Actual provider forks retain a distinct child conversation across SIGKILL without changing parent conversation history | `provider:codex_fork_test`, `provider:claude_fork_test`, `provider:copilot_fork_test`; native fork operations, exact child-ID resume and parent/child isolation checks |
| Explicit provider events normalize without transcript or terminal scraping | `provider:provider_test` passed 13 cases covering Claude/Codex hooks, both Copilot formats, OpenCode v2, trusted identity/path boundaries, unknown events and extension limits |
| Protocol 3 compact binary carriers and legacy compatibility | `wire:wire_test` passed 6 cases; bounded base64 input/output/checkpoint carriers, recorded codec/integrity fields, legacy byte arrays and historical Zlib checkpoint reads |
| Extensible bounded codecs, corruption checks and explicit preference fallback | `codec:codec_test` passed 10 cases, including binary carriers, exact consumed/written lengths, CRCs, malformed/truncated/trailing data, custom codecs and fallback only for actual encoded-budget overflow |
| Dense 10k history survives the new Host checkpoint/restart path | Host integration passed with 9,598 retained rows and an 8,716,971-byte raw checkpoint. Current budgets separate 32 MiB live history, 16 MiB raw VT state and a 4 MiB encoded checkpoint/file limit; optimized codec measurements are below |
| Shared GPUI browser component renders actual terminal and accepts real browser input | Actual Mac fixture passes local/SSH Tab completion, RGB, OSC8, find/Escape, reload, simultaneous mirrors/control transfer and read-only panning. Actual Linux Chromium paints the same GPUI component through WebGL and passes local/SSH input/style/clipboard and reload with the optimized bundle on the unchanged small VM |
| Stable versioned demo refresh preserves the running host and sessions | Protocol3 host upgrade preserves the original local holder/Zsh and SSH holder/Bash process identities. Versioned optimized assets passed live-page paint/discovery and are promoted; private `viewer.url` remains the local entry point |
| Inactive frontend memory saver leaves the process running | Actual Mac browser fixture receives 12k lines, retains approximately 9.6k under its page budget, releases an idle replica after 30s, requests fresh state on wake and accepts new input. Backend zero-viewer queries, detach barriers and checkpoint recovery pass. Wasm allocator pages may remain reserved after release |
| Remote SSH loss automatically reattaches the same process | `host:ssh_reconnect_test` passed with at least five failed dials before route restoration, same UUID/PID/cwd/environment, offline output/query recovery and responsive metadata. Lost receipts are uncertain and input is never automatically replayed |
| Generic component has no application chrome or socket requirement | `client:component_test` and `client:component_fast_test` pass15 cases each; supplied configuration/sink, identity/epoch binding, offline checkpoints, explicit visibility, opt-in memory policy and hooks |
| Physical browser clipboard, including exact remote paste-back | Isolated Linux Chromium fixtures pass both local and SSH cases: real Ctrl+Shift+V immediately followed by Enter, actual drag/Ctrl+Shift+C, Unicode across hard lines, ANSI stripping and exact remote bracketed UTF-8 bytes. The tested fixture never accesses the personal macOS clipboard |

Run commands, VM provisioning and system acceptance environment variables are in [README.md](README.md). The VM’s prompt fixture contains only public Oh My Zsh files. The local acceptance intentionally loads the owner’s config, redirects its cache/compdump/history and disables framework auto-updates. It does not grant direnv permissions or edit personal startup files. Current local startup messages about `_mullvad` and a blocked `.envrc` are faithfully rendered shell output.

Current priority is ordinary terminal and SSH clipboard behavior. Further
Neovim-specific clipboard work is explicitly deferred; historical editor
evidence above remains recorded. Physical browser Unicode/multiline clipboard assertions now pass for both
local and SSH sessions. The complete optimized fixture now passes, including Linux reload. Earlier
debug-build reload failures and their OOM evidence remain recorded below.

## Reading performance results

The interactive case generates 12,000 lines through a real PTY and a disk-synced output journal. It measures end-to-end output arrival separately from scrolling already-retained output. Each `scroll_frame_*` sample dispatches a Page Up keystroke and draws one natural headless frame. It checks paint failures and that an idle frame rebuilds no terminal rows. The earlier `scroll_p*` metric used the test helper plus extra draws and measured four frames per step; do not compare it to the single-frame metric.

Observed before the final gates, in a shared, unoptimized developer checkout:

| Backend / transport | Scroll median | Scroll p95 | Retained rows |
|---|---:|---:|---:|
| Ordinary GPUI / SSH | 5.387 ms | 7.420 ms | 12,028 |
| GPUI Fast / SSH | 3.259 ms | 4.464 ms | 12,027 |

These are headless observations, not presented-frame latency, a release benchmark, or a cross-machine guarantee. Cold startup, disk replay, sustained output and UI drawing have different costs. Preserve exact build mode and workload when comparing changes; use the final test logs for current values. Existing `pane:stream_bench` and `pane:stream_fast_bench` provide narrower renderer costs through `bazel run -c opt`.

The new codec comparison is a separate native `-c opt` workload, through `codec:dense_test`. It compresses an actual Ghostty checkpoint with 9,596 retained colored Unicode rows and 8,716,946 raw bytes. Three samples produced these averages:

| Codec | Encoded bytes | Encode | Decode |
|---|---:|---:|---:|
| LZ4 block | 4,154,587 | 29.047 ms | 7.851 ms |
| Zlib | 2,202,398 | 49.663 ms | 23.925 ms |

LZ4 fits this fixture's 4 MiB encoded budget with 39,717 bytes remaining. Protocol 3 and the default Host explicitly prefer `[LZ4, Zlib]`, falling back only when actual LZ4 output exceeds the encoded budget. Unknown codecs, plugin failures and malformed codec progress return errors. A caller-selected strict codec remains strict. These native compression timings do not measure browser parsing, drawing or end-to-end latency.

## Current API and persistence boundary

Protocol 3 records codec ID, decoded/encoded lengths and checksums with compact base64 checkpoint data. The codec registry supports caller-provided implementations; decoding uses bounded output and exact consumption checks. Binary envelopes keep disk carriers compact. Historical Zlib/byte-array readers remain explicit compatibility paths. The Host reserves its archive header/metadata allowance within the encoded file budget; the core VT snapshot retains its separate 16 MiB raw limit.

Workspace schema 3 adds a durable canonical UUID without changing the existing numeric route or holder location. Session metadata exposes the effective title, latest automatic title, optional manual/programmatic override and explicit string attributes. Explicit overrides use last-writer-wins; automatic runtime titles preserve them. Names contain 1–512 UTF-8 bytes; attributes allow 32 entries, 64 bytes per key, 4096 per value and 32 KiB total. Workspace files remain capped at 16 MiB and 256 terminals. Native metadata and injection APIs are covered by the public SDK proof. Raw/paste helpers do not append submit by default; raw bytes retain their normal terminal meaning. Epoch-scoped written receipts confirm PTY delivery, not agent execution. Uncertain input is not automatically persisted or resent.

The incoming-queue4009 regression is fixed with byte accounting and bounded frame draining; actual Mac12k/park/wake acceptance passes. Async browser clipboard reads have bounded FIFO barriers so Enter cannot overtake a paste; blur, disconnect, refusal and failures cancel safely. Actual Linux WebGL painting and both physical clipboard cases pass.

Debug browser builds repeatedly OOM-killed Chromium during reload in the 721 MiB/no-swap VM. Explicit viewer teardown was added, but the debug build still failed. Changing only the build mode to `-c opt` reduced Wasm from 46,596,431 to 24,283,027 bytes; the same strict complete fixture passed in 102.5 seconds with unchanged VM resources and WebGL-only SwiftShader flags. Optimized12 is frozen with SHA256 `3d5ace05871ce3a2df825d4b9f5ae789a56f5ed96c0175d7291258ab78048a43`. This establishes the tested workload, not a universal memory ceiling or a leak-free guarantee. Use optimized assets for release/milestone browser checks.

The latest broad nonmanual suite, gate96422, passed all 52 terminal test targets in 325.2 seconds. Real SSH restart, sibling, prolonged outage/reroute, hard-kill, vertical resize and native clipboard remain separate manual gates; these passed in this session. The focused SSH vertical tests do not depend on historical app screenshot goldens. An older `lab:app_fast_ssh_test` image comparison failed before its resize phase and was not re-recorded or treated as a passing current screenshot gate.

## Vertical resize and provider conversation recovery

The optimized13 browser fixture, gate53988, passed the complete Mac and Linux workload in 142 seconds on the unchanged small VM. At a fixed 1280-pixel browser width, heights 800→500→1000→800 produced actual PTY rows 34→18→45→34 while columns stayed at 77. Both the local and SSH panes retained their UUID, shell PID and exported environment. A sentinel addressed using the PTY's own row count remained on the last row, with the cursor on that row and at least 117 pixels of its explicit RGB color in the corresponding screenshot region. A fresh browser document then restored the same sessions and executed commands confirming the original shell PIDs and environment. This checks actual painting and PTY dimensions, beyond model-state assertions.

The provider proofs use the real installed CLIs with private configuration directories and deterministic local model endpoints. They make no upstream model calls and use no personal conversation or authentication files. They certify CLI persistence and context transport, not model reasoning, account access or upstream service availability.

| Installed CLI | Exact saved-conversation resume | Actual conversation fork |
|---|---|---|
| Codex 0.169.0 | `codex exec resume` with the full structured thread UUID | `codex exec fork` creates a distinct child UUID |
| Claude Code 2.1.325 | `claude --resume` with the full structured session UUID | `claude --resume --fork-session` creates a distinct child UUID |
| Copilot 1.0.112-0 | `copilot --resume=UUID` with the full structured session UUID | Native interactive `/fork` creates a distinct child UUID |

All three resume gates reject an exact nonexistent UUID without starting a new conversation or issuing inference. Copilot's `--session-id` is deliberately excluded from recovery because it can create a new session when the UUID does not exist. The fork gates resume the exact child ID after SIGKILL and verify inherited parent turns plus committed child-only turns. Forking and child-only work do not change the parent's existing conversation messages; native fork/resume metadata may legitimately be appended to its files. Unfinished input is provider-dependent and is reported separately from committed history. The Host never automatically resends uncertain input to compensate for a provider's rollback.

Transport reconnection keeps the existing process and terminal identity. After confirmed process loss, explicitly configured agent recovery instead creates a new PTY/process and terminal record linked to the predecessor, while retaining the exact provider conversation ID. A fork changes that provider ID: the integration must supply the current child identity through structured provider events rather than infer it from terminal text or keep using its parent ID. Shutdown waits for an in-flight recovery launch to settle; explicit native or viewer Stop remains an automatic-recovery opt-out after Host restart.

## Remaining stress work and adoption boundaries

- Rapid alternating resize while output is streaming, followed by crash/replay: the legacy direct-session lab journal has a presentation-order race. The new host owns ordered output/resize and true binary checkpoints; exercise that race through the real browser/native replicas before claiming visual equivalence.
- Sustained multi-megabyte output while several panes are visible, input under backpressure, very large paste, and event-to-screen latency under CPU/disk contention. Backend burst/acknowledgement and actual Mac browser12k/park/wake cases pass; broader throughput and latency under contention remain unmeasured.
- Mid-journal ENOSPC/EIO and interrupted metadata rename/fsync, beyond the covered partial-tail/corruption/write-permission cases. Preserve already-synced bytes and report actionable errors.
- Long-duration retention/compaction in legacy direct-session mode: its journal grows and reads complete history on attachment. The new memory-only Host has passed dense checkpoint/restart, checkpoint-failure and recovery-gap backend cases, using bounded compressed checkpoints and volatile holder replay. Long-duration and actual browser sustained-output acceptance remain separate work.
- Whole-VM gate28349 passes `host:agent_resume_vm_test` (37.5s): force-stop a fresh isolated Lima/VZ guest, confirm a new kernel boot ID, synced state/history and installed executable hashes, remain Unavailable with no duplicate, then explicitly recover the exact conversation after trusted new-boot evidence. Fresh UUID/process and same cwd are verified; the successful VM is deleted. The remote agent is a strict disk-backed fixture, while real Codex/Claude/Copilot tests run on macOS. The first probe exposed zero-byte unsynced copied executables; setup now fsyncs files/directories. No storage-destruction or arbitrary process-memory recovery claim.
- Forced network blackhole and keepalive expiry, SSH server restart, remote disk/permission failures and missing/incompatible remote holder. The current gates also cut and reroute an owned real SSH connection, proving automatic recovery after repeated failed dials. They do not certify every network/server failure mode or predictive echo.
- IME/dead-key/emoji/font fallback across actual platform backends and displays, mixed DPI, font installation changes, accessibility and terminal context menus. Core composition/Unicode tests exist; full platform certification does not.
- File/image transfer and explicit consent for programmatic clipboard reads remain host integrations. Inline terminal image protocols are not implemented by this text-grid renderer.
- The standalone host has two groups; a consuming app can choose a different layout. Main-app/prototype fake-transport replacement, native daemon integration and window/scroll/find/draft persistence remain consumer work. Opt-in provider conversation recovery is now implemented and tested at the native Host boundary; wiring every installed provider's structured events and trusted recovery configuration into a consuming application remains integration work. A dead machine cannot preserve arbitrary running-process memory.

Update this matrix with the exact failed case and regression target whenever iteration finds another bug. Do not infer universal compatibility from a successful shell banner or a single screenshot.

Browser architecture and alternatives are documented separately in [BROWSER.md](BROWSER.md). The optimized full fixture passes on the small VM; broader presented-frame performance remains unmeasured; this matrix does not claim universal terminal compatibility. The remote transport decision and exact recovery limits are in [TRANSPORT.md](../host/TRANSPORT.md).
