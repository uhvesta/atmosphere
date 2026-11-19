# Humata 2: crates and Bazel targets

Every package lives under `apps/humata2/` unless noted. Visibility is the narrowest that works: a feature is visible only to the app crates and their tests, and a daemon crate only to the daemon binaries and their tests.

The earlier H2-0/H2-1 code (`app` (Swift), `bindgen`, `core`, `rehearsal`) was deleted under D9; git history keeps it. The GUI binary is `desktop`. `prototype/` and `ui-prototype/` are retired references, not part of the build the app uses.

Packages built "once per engine" (D49, D57) have a plain target on ordinary GPUI and a `_fast` twin on GPUI Fast, for example `:diff_view` and `:diff_view_fast`.

## Shared
| Package | Crate | What it holds |
|---|---|---|
| `//libs/tca-rust` | `tca` | Store, reducers, effects, navigation, shared state, dependencies, persistence, observation, stream effects, TestStore. Framework-free. |
| `api/proto` | `humata2_proto_<domain>` (12 domain `*_rust` crates: `core`, `terminal`, `catalog`, `files`, `inbox`, `board`, `review`, `desk`, `machines`, `sessions`, `peer`, `events`), `humata2_proto` (`:humata2_rust` zero-ambiguity umbrella), `humata2_store_proto` | Prost message types from `api/proto/humata/v1`, split into 12 domain crates so consumers only rebuild on their own `.proto` changes, plus the messages the store keeps at rest. |
| `api/grpc` | `humata2_grpc_<domain>` (12 domain `api/grpc:<domain>` crates) + `api/grpc:all` | Tonic client/server codegen per proto domain, matching `api/proto` so services and live clients depend only on the domains they call. |
| `chunk` | `humata2_chunk` | 1 MiB chunks on a link, compressed with lz4 or zstd (or not), the codec policy, and the self-describing frame for data at rest (TR-11, TR-13, D60, D65). |
| `report` | `humata2_report` | Bug reports without a GUI (I9): recordings of interactions, actions, view trees and daemon calls, scrubbed, saved as text, and replayed as flow scripts against fakes. |

There is no separate model package: the domain types the app shows live in `clients/src/model.rs`, in the `clients:api` crate (`humata2_client_api::model`, re-exported as `humata2_clients::model`).

## Desktop app (on fake clients until the real daemon client exists)
| Package | Crate | What it holds |
|---|---|---|
| `features/app` | `humata2_app` | The root feature: routes and Back, overlays, the New session and New channel sheets, toasts, loading, the daemon's event stream, layout sizes, and saving where the person was across restarts. One file per area under `src/`. |
| `features/<name>` | `humata2_<name>` | One reducer per feature: `board`, `files`, `hosts`, `inbox`, `palette`, `review`, `session`, `settings`, `sidebar`, `views`. State, actions and effects only, no GUI. Tested with TestStore in `tests/*.rs`, one target per file. |
| `clients:api` | `humata2_client_api` | The client traits (daemon and terminals, clock, clipboard, file system, keychain, storage, file manager, machines, reviews), `Clients`, and the domain model in `src/model.rs`, with no fakes and no dummy data. Visible to `clients:__subpackages__`, `features:__subpackages__`, and `ui:__subpackages__`; all 14 `features/*` libraries and `ui/*` libraries depend on `:api` (aliased as `humata2_clients`), so fake/fixture edits never rebuild them. |
| `clients` | `humata2_clients` | Re-exports `:api` whole and adds the fakes (`src/fakes/`, `src/daemon/fake*`, `src/*/fake.rs`), the dummy data (`src/fixture/`), and `fake_clients`/`clients_with_fakes`/`demo_clients` for tests and `desktop`. |
| `clients/live` + `clients/live_*` | `humata2_live_clients` + `humata2_live_{core,daemon,desk,diff,files,machines,review}` + `live_test_support` | Live gRPC client implementations split by domain behind the thin `clients/live` re-export facade, each depending only on its narrow `api/grpc:<domain>` targets. |
| `ui` + `ui/*` | `humata2_ui` (thin 7-file shell) + `humata2_ui_{core,changes,desk,machines,overlays,report,settings,workspace}` | GPUI views split into 8 subcrates (`ui/core` shared bridge/theme/keys/widgets/chrome; leaf crates `ui/changes`, `ui/desk`, `ui/machines`, `ui/report`, `ui/settings`; `ui/overlays`; `ui/workspace`) behind the 7-file `ui` shell (`lib`, `shell`, `pages`, `panels`, `lists`, `starting`, `restart_report`). |
| `terminal/grid` | `terminal_grid` | The cell types and the `Grid` trait a renderer reads, with no dependencies, so the view needs no engine (D22). `:terminal_grid_fake` adds `FakeGrid` for tests. |
| `terminal/vt` | `ghostty_vt` | Safe Rust over libghostty-vt, built from pinned source with Zig inside Bazel (D10). |
| `terminal/view` | `terminal_view` | The GPUI grid view that draws any `Grid`, once per engine. |
| `terminal/pane` | `terminal_pane` | A terminal session's pane and tab strip over `ghostty_vt`, once per engine. |
| `diff` | `humata2_diff_view` | The diff view, once per engine; `:gate` measures frame times on the 20,000-line fixture (D67). Not yet used by `ui`. |
| `review` | `humata2_review_view` | The review pane: anchors, the comment book and its queue (D53). `review/harness` holds its GUI tests' fakes. Not yet used by `ui`. |
| `markdown` | `humata2_markdown` | The Markdown and Mermaid view, once per engine; its GUI tests are in `markdown/headless`. Not yet used by `ui`. |
| `engine` | `humata2_engine` | The one place the GUI engine is chosen (D57): `:engine` and `:engine_fast`, plus `:engine_test_support` for headless windows, natural frames and real input in tests, and the parity check of natural frames against fresh windows (D58). |
| `desktop` | `humata2` binary | The app: windows, menus, wiring. Its tests (`shell_test`, `panels_test`, `native_test`, `design_test`) draw the window headless; `design_test` holds the design goldens (D76). |
| `skills` | — | Agent skills Humata ships and its daemon installs for each agent (D790): one folder per skill with a thin `SKILL.md` and its reference files, `shared/` copied into each, and `README.md` with the list. `:bundle` packs them for `daemon/machines` to embed. |

## Terminal experiment (real local and SSH PTYs)
| Package | Crate | What it holds |
|---|---|---|
| `terminal/session` | `terminal_session` | UI-independent holder attachments, input acknowledgements/resize and saved descriptors/layout; legacy private journals or memory-only host mode, over local/SSH Unix sockets. |
| `terminal/wire` | `terminal_wire` | Platform-neutral ordered session messages, binary checkpoint identity, bounded history policy and explicit recovery metadata. |
| `terminal/codec` | `terminal_codec` | Bounded pluggable checkpoint envelopes, LZ4/Zlib codecs and explicit preference/fallback policy, shared by native and Wasm. |
| `terminal/provider` | `terminal_provider` | Pure versioned structured provider-event adapters with no transcript scanning or notification side effects. |
| `terminal/host` | `terminal_host` | Persistent authoritative terminal runtime and authenticated loopback WebSocket/static gateway; owns local/SSH sessions, memory state and bounded private disk checkpoints. |
| `terminal/client` | `terminal_replica`, `terminal_client` | Standalone single-terminal GPUI component with caller transport/checkpoint/hooks; optional shared workspace chrome and native WebSocket adapter. |
| `terminal/web` | Wasm application/bundle | Browser build of that same GPUI workspace; talks only to the local host server, which also owns SSH connections. Optimized Mac stress/refresh and Linux painting/clipboard/reload pass on the dedicated small VM. |
| `terminal/embed` | `terminal_embed` | Reusable GPUI terminal content over a real session; bounded background I/O, replay, focus notification and the existing TerminalPane. |
| `terminal/lab` | `terminal_lab`, `terminal_lab` binary | Standalone macOS experiment with tab movement, two groups, detach/resume and text paste. Headless tests use actual PTYs locally and in a dedicated small Lima VM over SSH; see its README. Main desktop transport remains unchanged. |

## Daemon and boundaries
| Package | Crate | What it holds |
|---|---|---|
| `daemon/ports` | `humata2_daemon_ports` | Narrow cross-service port traits (`InboxPort`, `AwayPort`, `PeerRelayPort`, `TransferPort`, `TerminalPort`, `MachinePort`, `LifecyclePort`, `ChannelPort`, `SessionPort`, `SessionTransferPort`, `SessionArchivePort`, `BoardPort`, `FilesPort`, `ReviewPort`, `TrashBinPort`, `CatalogPort`) and shared value types (`Caller`, `Gated`, `Release`, `PortError`). |
| `daemon/rpc_kit` | `humata2_rpc_kit` | Zero-service gRPC helpers (`blocking`, `failed`, `lock`, `now_ms`, `Answers`, `Compression`, `rfc3339`, `link_proof`, `sync`, runtime `admin` mode, `log`). |
| `daemon/runtime` | `humata2_daemon_runtime` (+ `:runtime_faults`) | Stateful daemon runtime helpers (`ready`, `terminal_uuid`, `screen_question`, `agent_switches`, `tool_path`, `paste`, `stop_gate`, runtime-gated `fault` points). |
| `daemon/sessions` + `daemon/sessions_*` | `humata2_sessions` facade + `humata2_sessions_{core,desk,carry}` (+ `*_faults`) | Session lifecycle, Desk folders, and cross-machine carry/roam split into 3 subcrates behind the `daemon/sessions` facade. |
| `daemon/svc_*` | `humata2_svc_{away,board,catalog,channels,desk,files,inbox,machines,mcp,peer,review,sessions_common,sessions_local,sessions_transfer,sessions_archive,stop_restart,terminal,transfers,upgrade}` (+ `*_faults` where needed) | Extracted domain service crates communicating through `daemon/ports`, `daemon/rpc_kit`, and `daemon/runtime` (with `SessionService` split 3-way into `svc_sessions_{local,transfer,archive}` over `svc_sessions_common`). |
| `daemon/humatad` | `humatad2` binary, `humata2_grpc` | Serves `api/proto` on a Unix socket or the peer link on stdio; `humatad2 bridge` carries the socket over SSH or a provider's connect command. Wiring binary; `:humatad2_admin` is the test daemon. Its tests are the gRPC boundary suites against the real binaries (see `workflow/milestones/SPLIT_CAPABILITY_MATRIX.md` for scoped Tier-B profiles). |
| `daemon/holder` | `humata2_holder` binary | The per-session process that holds a PTY and its scrollback, outliving the daemon. |
| `daemon/store` | `humata2_store` | Sessions, tabs, pointers and the journal in one SQLite database, written atomically. |
| `daemon/durable` | `humata2_durable` | File writes that say when they are on disk; `:durable_faults` simulates power cuts for tests. |
| `daemon/revive` | `humata2_revive` | Brings a machine's sessions back after their holders are gone and writes the restart report. |
| `daemon/review_diff` | `humata2_review_diff` | Review diffs from the git CLI, reviewed snapshots, per-file Viewed, review plans and the review chat's view state. |
| `daemon/access` | `humata2_access` | Capability tiers, grants and checks (`api/BOUNDARIES.md`). Every boundary calls it. |
| `daemon/machines` | `humata2_machines` | Providers (scripted `create`/`connect`/`identity`), installing humatad, Lima for tests; `:fake_machine` for tests. |
| `daemon/mcp` | `humata2_mcp` | The MCP server (`api/MCP.md`). |
| `cli` | `humata` binary | The CLI (`api/CLI.md`). |

## Testing
| Package | What it holds |
|---|---|
| `testing/snapshot` | State, view, layout and image snapshots: record and verify commands. `humata2_snapshot_test` in `testing/defs.bzl` makes a test's snapshots its data and adds a `bazel run` `_record` target. |
| `testing/flow` | The flow runner: scripts of user actions against real state and views with fake clients, or against real daemons. `:flow` and `:flow_fast`, one per engine. |
| `testing/linux` | `run.sh` runs any test target on Linux: it cross-compiles to static musl (`--config=linux_arm64`) on this Mac and runs the binary in the `humata2-linux` Lima VM. |
| `spike/gui` | The I2 spike, on each engine (`:spike`, `:spike_fast`); its on-screen benchmarks are in its README. |

## Running the suites
- macOS: `bazel test //apps/humata2/... //libs/tca-rust/...`
- Linux: `apps/humata2/testing/linux/run.sh <targets>`, which builds for `linux_arm64` and runs each test in the VM.
