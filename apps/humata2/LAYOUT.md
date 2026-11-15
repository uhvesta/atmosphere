# Humata 2: crates and Bazel targets

Every package lives under `apps/humata2/` unless noted. Visibility is the narrowest that works: a feature is visible only to the desktop app and its tests, and a daemon crate only to the daemon binaries and the boundary suites.

The earlier H2-0/H2-1 code (`app` (Swift), `bindgen`, `core`, `rehearsal`) is left alone until D9 is answered. The new GUI binary is therefore `desktop`, not `app`.

## Shared
| Package | Crate | What it holds |
|---|---|---|
| `//libs/tca-rust` | `tca` | Store, reducers, effects, navigation, shared state, dependencies, persistence, observation, stream effects, TestStore. Framework-free. |
| `api/proto` | `humata2_proto` | The gRPC messages and services from `api/proto/humata/v1`. |
| `model` | `humata2_model` | Plain domain types shared by the app and the daemon (ids, pointers, capabilities) that aren't generated messages. |

## Desktop app (built only after gate G)
| Package | Crate | What it holds |
|---|---|---|
| `features/<name>` | `humata2_<name>` | One reducer per feature (sidebar, session, review, files, inbox, board, hosts, settings, palette, views). State, actions and effects only, no GUI. Tested with TestStore. |
| `clients` | `humata2_clients` | The dependency clients features use (daemon, clock, clipboard, file system, keychain), each a trait with a live and a fake implementation. |
| `ui` | `humata2_ui` | GPUI views for each feature and the one store bridge between tca and GPUI's executor. |
| `terminal/grid` | `terminal_grid` | The cell types and the `Grid` trait a renderer reads, with no dependencies, so the view needs no engine (D22). |
| `terminal/vt` | `ghostty_vt` | Safe Rust over libghostty-vt (D10). The view reads it through a `Grid` trait, so an `alacritty_terminal` fallback would only have to implement `Grid`. |
| `terminal/view` | `humata2_terminal_view` | The GPUI grid view that draws any `Grid`. |
| `desktop` | `humata2` binary | The app: windows, menus, wiring. |
| `engine` | `humata2_engine` | The one place the GUI engine is chosen (D57): ordinary GPUI, or GPUI Fast with the `fast` feature (`:engine` and `:engine_fast` in Bazel). Headless windows, natural frames and real input for tests, and the parity check of natural frames against fresh windows (D58). |

## Daemon and boundaries
| Package | Crate | What it holds |
|---|---|---|
| `daemon/humatad` | `humatad2` binary | Serves `api/proto` over one SSH or stdio stream. |
| `daemon/holder` | `humata2_holder` binary | The per-session process that holds a PTY and its scrollback, outliving the daemon. |
| `daemon/store` | `humata2_store` | Sessions, journal and pointers on disk, written atomically. |
| `daemon/access` | `humata2_access` | Capability tiers, grants and checks (`api/BOUNDARIES.md`). Every boundary calls it. |
| `daemon/machines` | `humata2_machines` | Providers (scripted `create`/`connect`/`identity`), installing humatad, Lima for tests. |
| `daemon/mcp` | `humata2_mcp` | The MCP server (`api/MCP.md`). |
| `cli` | `humata` binary | The CLI (`api/CLI.md`). |

## Testing
| Package | What it holds |
|---|---|
| `testing/snapshot` | State, view, layout and image snapshots: record and verify commands. `:snapshot` and `:snapshot_fast`, one per engine; `humata2_snapshot_test` in `testing/defs.bzl` makes a test's snapshots its data and adds a `bazel run` `_record` target. |
| `testing/boundary` | `humata2_boundary`: real daemons on Unix sockets with the spec's fixture and fake outside services, for the gRPC, MCP and CLI suites (`TESTING.md` §3). |
| `testing/flow` | The flow runner: scripts of user actions against real state and views with fake clients, or against real daemons. `:flow` and `:flow_fast`, one per engine. |
| `testing/linux` | Runs any test target on Linux. It cross-compiles to static musl (`--config=linux_arm64`) on this Mac and runs the binary in the `humata2-linux` Lima VM. |
| `spike/gui` | The I2 spike, on each engine (`:spike`, `:spike_fast`); its on-screen benchmarks are in its README. |

## Running the suites
- macOS: `bazel test //apps/humata2/... //libs/tca-rust/...`
- Linux: `apps/humata2/testing/linux/run.sh <targets>`, which builds for `linux_arm64` and runs each test in the VM.
