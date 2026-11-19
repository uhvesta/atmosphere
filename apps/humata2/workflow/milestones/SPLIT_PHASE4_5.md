# SPLIT Phases 4 & 5: Detailed Split Plan (`daemon/sessions`, `clients/live`, `api/proto`)

- **Parent doc:** [`SPLIT.md`](./SPLIT.md) (§5.2, §5.3, §8 Phases 4 & 5)
- **Status:** verified design blueprint (read-only static audit + independent Gemini 4 Neon High verification)
- **Scope:**
  - **Phase 4A:** `apps/humata2/daemon/sessions` (36 `.rs` files, 16,482 LOC)
  - **Phase 4B:** `apps/humata2/clients/live` (32 `.rs` files, 17,225 LOC)
  - **Phase 5:** `apps/humata2/api/proto` (21 `humata/v1/*.proto` files, 9,078 LOC + `store.proto`, 123 LOC) and `apps/humata2/daemon/humatad:grpc`

> [!IMPORTANT]
> Static inspection surfaced four Rust/Bazel constraints that refine the high-level sketch in `SPLIT.md`:
> 1. **`daemon/sessions`:** `fork.rs` cannot move into `sessions_desk` because `start.rs`, `recover.rs`, `own_copy.rs`, and `continue_in.rs` call `crate::fork::*` (a cycle). Also, **17 files** across `sessions` write inherent `impl Starter` blocks (`E0116` across crates), and Bazel `*_faults` twins produce distinct nominal Rust types (`store::Store != store_faults::Store`), so every `sessions_*` crate linked into `humatad2_faults` still needs a `*_faults` twin.
> 2. **`clients/live`:** `events.rs` calls into `boards`, `desk`, `inbox`, `terminals`, and `daemon`, so it belongs in `live_daemon` (not `live_core`). `link.rs` is only used by `terminals.rs`, so keeping it in `live_daemon` decouples all other live crates from `terminal_link` and `terminal/wire`. Rust's orphan rule (`E0117`) forbids `impl Daemon/Terminals/DeskClient for LiveDaemon` in another crate, so `LiveConnection` lives in `live_core`, `LiveDaemon` lives in `live_daemon`, and `DeskClient` gets a dedicated `LiveDesk` struct in `live_desk`.
> 3. **`clients/live` leaf isolation:** `diff.rs` is called by `Daemon` (`daemon.rs`), `views.rs`, and `review_chat.rs`. Giving `diff.rs` its own small `clients/live_diff` crate (802 LOC) keeps `clients/live_review` (3,075 LOC) a 100% leaf crate that never rebuilds `live_daemon`.
> 4. **`api/proto`:** All 21 `humata/v1/*.proto` files share `package humata.v1;` (wire-contractual for gRPC service paths and `/humata.v1.Error` status details). Achieving the Phase 5 cache win (`review.proto` edits not invalidating `power_cut_*` or `svc_machines`) requires: (a) a thin same-package prost wrapper rule with explicit `.google.protobuf` extern paths, (b) **4 small wire-identical type moves** into `proto_core` (including all 9 `FileLocation` root messages), and (c) **decoupling `daemon/runtime::events` from `pb::Event` and `svc_peer`'s full-service `relay.rs`/`host_relay.rs` wiring from `humatad2_test_power_cut`**, otherwise `proto_events` and `svc_peer` transitively pull all domain protos back in.

---

## Part 1 — Phase 4A: `apps/humata2/daemon/sessions`

### 1.1 Proposed crates and file mapping

All 36 files under `apps/humata2/daemon/sessions/src/` (16,482 LOC total) map to three crates. `sessions_desk` and `sessions_carry` are parallel siblings that both depend only on `sessions_core`.

```
                    ┌──────────────────────────────────────────┐
                    │ daemon/humatad (or Phase 4 svc_* crates) │
                    └──────┬─────────────────────┬─────────────┘
                           │                     │
              ┌────────────▼──────────┐ ┌────────▼──────────────┐
              │ daemon/sessions_desk  │ │ daemon/sessions_carry │
              │ (2,441 LOC, +_faults) │ │ (5,781 LOC, +_faults) │
              └────────────┬──────────┘ └────────┬──────────────┘
                           └──────────┬──────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │   daemon/sessions_core    │
                        │   (8,260 LOC, +_faults)   │
                        └───────────────────────────┘
```

| Proposed crate (`crate_name`) | Target dir | Files from `daemon/sessions/src/` | LOC | Faults twin? | External daemon deps |
|---|---|---|---:|---|---|
| `sessions_core` (`humata2_sessions_core`) | `apps/humata2/daemon/sessions_core` | `lib.rs` (68, trimmed), `error.rs` (222), `layout.rs` (204), `names.rs` (195), `prefix.rs` (37), `tree_copy.rs` (178), `git.rs` (525), `work.rs` (242), `push.rs` (209), `agents.rs` (2,042), `first_message.rs` (469), `setup.rs` (141), `start.rs` (847), `add_repo.rs` (114), `rename.rs` (108), `restore.rs` (141), `fork.rs` (1,003), `own_copy.rs` (713), `continue_in.rs` (205), `recover.rs` (477), `versions_tests.rs` (120, `#[cfg(test)]`) — **21 files** (20 non-`lib.rs` + trimmed `lib.rs`) | ~8,260 | **Yes** (`sessions_core_faults`) — owns **all 5** files with `humata2_store::fault::point` / `#[cfg(faults_build)]` (`setup.rs`, `start.rs`, `fork.rs`, `own_copy.rs`, `continue_in.rs`) | `conversation`, `durable`, `git_head`, `holder`, `mirror`, `revive`, `store` |
| `sessions_desk` (`humata2_sessions_desk`) | `apps/humata2/daemon/sessions_desk` | `lib.rs` (new), `doc_repo.rs` (1,073), `desk.rs` (924), `desk_move.rs` (256), `channel_desk.rs` (188) — **4 source files** | ~2,441 | **Yes** (`sessions_desk_faults`) — no `fault::point` of its own, but links against `sessions_core_faults` / `store_faults` / `durable_faults` / `revive_faults` for type identity in `humatad2_faults` | `sessions_core`, `durable`, `revive`, `store` |
| `sessions_carry` (`humata2_sessions_carry`) | `apps/humata2/daemon/sessions_carry` | `lib.rs` (new), `carry.rs` (1,908), `bring.rs` (187), `carry_cheap.rs` (363), `carry_chunks.rs` (446), `carry_desk.rs` (272), `carry_fork.rs` (337), `carry_landing.rs` (494), `carry_placed.rs` (260), `carry_roam.rs` (432, `#[cfg(test)]`), `carry_reid_tests.rs` (358, `#[cfg(test)]`), `carry_landing_tests.rs` (724, `#[cfg(test)]`) — **11 source files** | ~5,781 | **Yes** (`sessions_carry_faults`) — no `fault::point` of its own, but links against `sessions_core_faults` / `store_faults` / `durable_faults` / `holder_faults` / `revive_faults` in `humatad2_faults` | `sessions_core`, `conversation`, `durable`, `holder`, `revive`, `store` |

- **Temporary facade (`apps/humata2/daemon/sessions`):** During migration, `daemon/sessions` remains as a thin re-export crate (`pub use humata2_sessions_core::*; pub use humata2_sessions_desk::*; pub use humata2_sessions_carry::*;`) until `daemon/humatad` (or Phase 4 `svc_*` crates) imports the three crates directly, then `daemon/sessions` is retired.
- **Note on `Cargo.toml`:** `daemon/sessions` has no `Cargo.toml` and is not in the root Cargo workspace (Bazel-only), so only `BUILD.bazel` files and visibility lists in `conversation`, `durable`, `git_head`, `holder`, `mirror`, `revive`, and `store` need updates.

### 1.2 Cross-crate references (`daemon/sessions`)

#### Structural boundary 1: Inherent `impl Starter` across crates (`E0116`)

`Starter` is defined in `start.rs` (`sessions_core`). All 8 of its fields are already `pub` (`store`, `layout`, `remote_template`, `agents`, `home`, `revive`, `holders`, `first_message`). **17 files** in `daemon/sessions/src/` define inherent `impl Starter` blocks, of which 10 move outside `sessions_core`:

- **In `sessions_desk` (3 files):** `desk.rs:337` (`start_desk`, `record_desk`, `undo_desk`), `desk_move.rs:131` (`desk_move_remote`, `desk_move_branch`), `channel_desk.rs:55` (`start_channel_desk`, `record_channel_desk`).
- **In `sessions_carry` (7 files):** `carry.rs` (26 methods: `pack`, `pack_for`, `unpack`, `abort_incoming`, `discards`, `forget_moved`, `unstage`, `shell_holders`, plus internal helpers), `carry_cheap.rs:92`, `carry_chunks.rs:228`, `carry_desk.rs:95`, `carry_fork.rs:106,317`, `carry_landing.rs:44`, `carry_placed.rs:47`.

**Resolution & Rust pitfalls to avoid:**
1. **Internal helpers:** Across those 10 files, `impl Starter` holds ~38 private/`pub(super)`/`pub(crate)` helper methods (including `desk.rs:449` `undo_desk`, called from `channel_desk.rs:122`). Rust trait impls cannot span multiple files or carry `pub(crate)` methods, so convert internal helpers into crate-private free functions (or methods on a crate-private wrapper `struct DeskOps<'a>(&'a Starter)` / `CarryOps<'a>(&'a Starter)`).
2. **`Send` futures for `async fn`:** `start_desk`, `start_channel_desk`, and `launch_unlaunched` are `async` methods called inside tonic `#[tonic::async_trait]` handlers (which require `Send` futures). Plain `async fn` in a trait does not bound `+ Send` (`async_fn_in_trait` + `Send` errors). Either return explicit `-> impl Future<Output = ...> + Send` on `DeskStarter`/`CarryStarter` extension traits, or expose `pub struct DeskOps<'a>(&'a Starter)` / `CarryOps<'a>(&'a Starter)` with inherent `pub async fn` methods.

#### Structural boundary 2: `DESK_CHANNEL` and `CHANNELS_ROOT` constants

- `pub const DESK_CHANNEL: &str = "desk";` (`desk.rs:27`) is imported by `carry_desk.rs:17` and `carry_landing_tests.rs:24` (plus 18 files under `daemon/humatad/src/`). Moving `DESK_CHANNEL` into `sessions_core` (`names.rs`, re-exported from both `sessions_core` and `sessions_desk`) eliminates the only `sessions_carry -> sessions_desk` edge.
- `pub const CHANNELS_ROOT: &str = "@root";` (`carry_desk.rs:22`) is imported by `daemon/humatad/src/files/roots.rs`. Moving `CHANNELS_ROOT` into `sessions_core` alongside `DESK_CHANNEL` keeps `svc_files` depending on `sessions_core` only (no `svc_files -> sessions_carry` edge).

#### Exhaustive `crate::` reference table (`sessions_desk` and `sessions_carry` -> `sessions_core`)

Zero references cross in the reverse direction (`sessions_core` has no references to `sessions_desk` or `sessions_carry`). Note that all modules in `sessions/src/lib.rs:7–33` (except `doc_repo`) are private `mod`; `sessions_core/src/lib.rs` must expose them (or a `pub mod internal`) so promoted items are reachable across crates.

| Source file (new crate) | Line(s) | Current `crate::` reference (or `Starter` method call) | Target crate | Visibility change needed in `sessions_core` |
|---|---|---|---|---|
| `doc_repo.rs` (`sessions_desk`) | L14 | `crate::error::GitError` | `sessions_core` | Already `pub` |
| `desk.rs` (`sessions_desk`) | L19 | `crate::error::{GitError, StartError}` | `sessions_core` | Already `pub` |
| `desk.rs` (`sessions_desk`) | L20 | `crate::git::{clone_main, git, remote_url}` | `sessions_core` | Promote `git::git` from `pub(crate)` to `pub` |
| `desk.rs` (`sessions_desk`) | L21 | `crate::names::branch_slug` | `sessions_core` | Already `pub` |
| `desk.rs` (`sessions_desk`) | L22 | `crate::setup::unix_ms` | `sessions_core` | Promote `setup::unix_ms` from `pub(crate)` to `pub` |
| `desk.rs` (`sessions_desk`) | L23 | `crate::start::{Started, Starter}` | `sessions_core` | Already `pub` |
| `desk.rs` (`sessions_desk`) | L411 | `self.launch_session(...)` (`Starter` method in `start.rs`) | `sessions_core` | Promote `Starter::launch_session` from `pub(crate)` to `pub` |
| `desk.rs` (`sessions_desk`) | L805 | `crate::layout::make_dirs` | `sessions_core` | Promote `layout::make_dirs` from `pub(crate)` to `pub` |
| `desk_move.rs` (`sessions_desk`) | L13 | `crate::error::{GitError, StartError}` | `sessions_core` | Already `pub` |
| `desk_move.rs` (`sessions_desk`) | L14 | `crate::git::{default_branch, git, remote_url}` | `sessions_core` | Promote `git::git` to `pub` |
| `desk_move.rs` (`sessions_desk`) | L15 | `crate::names::Branch` | `sessions_core` | Already `pub` |
| `desk_move.rs` (`sessions_desk`) | L16 | `crate::start::{RepoName, Starter}` | `sessions_core` | Already `pub` |
| `desk_move.rs` (`sessions_desk`) | L145 | `self.channel_record(...)` (`Starter` method in `start.rs`) | `sessions_core` | Promote `Starter::channel_record` from `pub(crate)` to `pub` |
| `desk_move.rs` (`sessions_desk`) | L151 | `self.pick_fork(...)` (`Starter` method in `fork.rs`) | `sessions_core` | Promote `Starter::pick_fork` from `pub(crate)` to `pub` |
| `desk_move.rs` (`sessions_desk`, **test**) | L174 | `crate::work::tests::run` (`#[cfg(test)]` helper!) | `sessions_core` | **Cannot import `#[cfg(test)]` across crates.** Inline the 9-line `run()` git helper (`work.rs:146–154`) in `desk_move.rs`'s `mod tests` |
| `channel_desk.rs` (`sessions_desk`) | L15 | `crate::error::StartError` | `sessions_core` | Already `pub` |
| `channel_desk.rs` (`sessions_desk`) | L16 | `crate::names::branch_slug` | `sessions_core` | Already `pub` |
| `channel_desk.rs` (`sessions_desk`) | L17 | `crate::setup::unix_ms` | `sessions_core` | Promote `setup::unix_ms` to `pub` |
| `channel_desk.rs` (`sessions_desk`) | L18 | `crate::start::{Started, Starter}` | `sessions_core` | Already `pub` |
| `channel_desk.rs` (`sessions_desk`) | L129 | `self.launch_session(...)` (`Starter` method in `start.rs`) | `sessions_core` | Promote `Starter::launch_session` to `pub` |
| `carry.rs` (`sessions_carry`) | L31 | `crate::agents::AgentKind` | `sessions_core` | Already `pub` |
| `carry.rs` (`sessions_carry`) | L32 | `crate::error::AgentError` | `sessions_core` | Already `pub` |
| `carry.rs` (`sessions_carry`) | L33 | `crate::layout::{make_dirs, partial}` | `sessions_core` | Promote `layout::make_dirs` to `pub` |
| `carry.rs` (`sessions_carry`) | L34 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry.rs` (`sessions_carry`) | L35 | `crate::tree_copy::{self, Dirs, Rebase, TreeCopyError}` | `sessions_core` | Promote `mod tree_copy`, `Dirs`, `Rebase` (incl. `from`, `to`, `target()`), `TreeCopyError` (incl. `doing`, `path`, `source`), `copy_tree`, `copy_file` from `pub(crate)` to `pub` |
| `carry.rs` (`sessions_carry`) | L123 | `crate::error::GitError` (production `CarryError::Rebuild`) | `sessions_core` | Already `pub` |
| `carry.rs` (`sessions_carry`, **test**) | L1549–L1550 | `crate::agents::Agents`, `crate::layout::Layout` | `sessions_core` | Already `pub` |
| `carry_cheap.rs` (`sessions_carry`) | L24 | `crate::git::{git, sync_tree, text}` | `sessions_core` | Promote `git::{git, sync_tree, text}` from `pub(crate)` to `pub` |
| `carry_cheap.rs` (`sessions_carry`) | L25 | `crate::layout::partial` | `sessions_core` | Already `pub` |
| `carry_cheap.rs` (`sessions_carry`) | L26 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry_chunks.rs` (`sessions_carry`) | L34 | `crate::layout::{make_dirs, partial}` | `sessions_core` | Promote `layout::make_dirs` to `pub` |
| `carry_chunks.rs` (`sessions_carry`) | L35 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry_desk.rs` (`sessions_carry`) | L17 | `crate::desk::DESK_CHANNEL` | `sessions_core` | Move `DESK_CHANNEL` from `desk.rs` to `sessions_core` |
| `carry_desk.rs` (`sessions_carry`) | L18 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry_fork.rs` (`sessions_carry`) | L23 | `crate::error::{GitError, StartError}` | `sessions_core` | Already `pub` |
| `carry_fork.rs` (`sessions_carry`) | L24 | `crate::fork::{ForkCopy, copy_clone, copy_conversation, copy_durably}` | `sessions_core` | Promote `fork::{copy_clone, copy_conversation, copy_durably}` from `pub(crate)` to `pub` |
| `carry_fork.rs` (`sessions_carry`) | L25 | `crate::setup::unix_ms` | `sessions_core` | Promote `setup::unix_ms` to `pub` |
| `carry_fork.rs` (`sessions_carry`) | L26 | `crate::start::{RepoName, Started, Starter, blocking}` | `sessions_core` | Promote `start::blocking` from `pub(crate)` to `pub` |
| `carry_fork.rs` (`sessions_carry`) | L134, L137, L330, L333 | `self.parent(...)`, `self.pick_fork(...)`, `self.launch_with(...)` | `sessions_core` | Promote `Starter::parent`, `Starter::pick_fork`, `Starter::launch_with` from `pub(crate)` to `pub` |
| `carry_landing.rs` (`sessions_carry`) | L23 | `crate::layout::partial` | `sessions_core` | Already `pub` |
| `carry_landing.rs` (`sessions_carry`) | L24 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry_landing.rs` (`sessions_carry`) | L25 | `crate::tree_copy::{Rebase, copy_file}` | `sessions_core` | Promote `tree_copy::{Rebase, copy_file}` (+ `Rebase` fields/`target()`) to `pub` |
| `carry_placed.rs` (`sessions_carry`) | L25 | `crate::agents::AgentKind` | `sessions_core` | Already `pub` |
| `carry_placed.rs` (`sessions_carry`) | L26 | `crate::error::StartError` | `sessions_core` | Already `pub` |
| `carry_placed.rs` (`sessions_carry`) | L27 | `crate::fork::{Through, fork_conversation}` | `sessions_core` | Promote `fork::{Through, fork_conversation}` (incl. `Through`'s fields `format`, `from`, `from_id`, `to`, `to_id`, and `keeps()`) from `pub(crate)` to `pub` |
| `carry_placed.rs` (`sessions_carry`) | L28 | `crate::start::Starter` | `sessions_core` | Already `pub` |
| `carry_roam.rs` (`sessions_carry`, **test**) | L18–L19 | `crate::agents::Agents`, `crate::layout::Layout` | `sessions_core` | Already `pub` |
| `carry_reid_tests.rs` (`sessions_carry`, **test**) | L16–L18 | `crate::agents::Agents`, `crate::fork::ForkCopy`, `crate::layout::Layout` | `sessions_core` | Already `pub` |
| `carry_landing_tests.rs` (`sessions_carry`, **test**) | L23–L25 | `crate::agents::Agents`, `crate::desk::DESK_CHANNEL`, `crate::layout::Layout` | `sessions_core` | Move `DESK_CHANNEL` to `sessions_core`; others already `pub` |

> Note: `bring.rs:15,88` imports 15 items from `crate::carry::*` — both `bring.rs` and `carry.rs` move together into `sessions_carry`. `fork.rs:762`, `own_copy.rs:574`, `push.rs:90`, and `restore.rs:91` import `crate::work::tests::*` — all stay together in `sessions_core`.

### 1.3 Test re-pointing (`daemon/sessions`)

| Current test target | New test target(s) | Crate(s) tested | Notes |
|---|---|---|---|
| `//apps/humata2/daemon/sessions:sessions_unit_test` | `//apps/humata2/daemon/sessions_core:sessions_core_unit_test`<br>`//apps/humata2/daemon/sessions_desk:sessions_desk_unit_test`<br>`//apps/humata2/daemon/sessions_carry:sessions_carry_unit_test` | `sessions_core`<br>`sessions_desk`<br>`sessions_carry` | Only `sessions_core_unit_test` needs `data = ["testdata/fake_cli.sh"]` and `HUMATA2_FAKE_CLI` (used exclusively by `agents.rs` and `versions_tests.rs`). `sessions_desk_unit_test` needs `@crates//:tempfile` only; `sessions_carry_unit_test` needs `@crates//:rusqlite` and `@crates//:tempfile`. |
| `//apps/humata2/daemon/sessions:sessions_faults_unit_test` | `//apps/humata2/daemon/sessions_core:sessions_core_faults_unit_test` | `sessions_core_faults` | Only `fork.rs:766,779` and `own_copy.rs:578,591` contain `#[cfg(faults_build)]` unit tests. Neither `sessions_desk` nor `sessions_carry` needs a faults unit test target. |
| `//apps/humata2/daemon/sessions:kill_matrix_test` (`tests/kill_matrix.rs`) | `//apps/humata2/daemon/sessions_core:kill_matrix_test` | `sessions_core_faults` | Imports only `Agents, Layout, Leftovers, NewChannel, StartRequest, Starter, clone_mains, create_channel, recover` — zero desk or carry usage. |
| `//apps/humata2/daemon/sessions:power_cut_test` (`tests/power_cut.rs`) | `//apps/humata2/daemon/sessions_core:power_cut_test` | `sessions_core_faults` | Imports only `Layout, clone_main, clone_session, partial` — zero desk or carry usage. |
| Downstream Phase 4 daemon service crates (`svc_*`) | `svc_sessions_local`, `svc_review`, `svc_files`, `svc_catalog`, `svc_inbox`, `svc_terminal`, `svc_machines`, `svc_lifecycle` -> `sessions_core` only.<br>`svc_desk` / `svc_channels` -> `sessions_core` + `sessions_desk`.<br>`svc_sessions_transfer`, `svc_sessions_archive`, `svc_transfers` -> `sessions_core` + `sessions_carry`. | Scoped per domain | Editing `carry*.rs` no longer rebuilds `svc_sessions_local`, `svc_desk`, `svc_review`, `svc_files`, `kill_matrix_test`, or `sessions/power_cut_test`. |

### 1.4 Order of moves (`daemon/sessions`)

Every step below compiles and passes all tests on its own:

1. **Prep inside `daemon/sessions` (no new crates yet):**
   - Move `DESK_CHANNEL` (`desk.rs:27`) and `CHANNELS_ROOT` (`carry_desk.rs:22`) to `names.rs` (re-export in `desk.rs`, `carry.rs`, and `lib.rs`).
   - Expose core modules (or `pub mod internal`) and promote all `pub(crate)` items listed in §1.2 — including `TreeCopyError` fields (`doing`, `path`, `source`), `Rebase` fields + `target()`, and `Through` fields + `keeps()` — to `pub`.
   - Replace `crate::work::tests::run` in `desk_move.rs:174` with a local 9-line test helper.
   - Move the ~38 private/`pub(super)`/`undo_desk` helpers out of `impl Starter` in `desk*.rs` and `carry*.rs` into free functions or crate-private wrappers, then convert public `impl Starter` methods into `DeskStarter` / `CarryStarter` extension traits with explicit `-> impl Future<Output = ...> + Send` bounds (or `DeskOps`/`CarryOps` wrappers), updating `daemon/humatad` call sites.
2. **Create `apps/humata2/daemon/sessions_core` (`sessions_core` + `sessions_core_faults`):**
   - Add `//apps/humata2/daemon/sessions_core:__pkg__` to visibility in `conversation`, `durable`, `git_head`, `holder`, `mirror`, `revive`, and `store`.
   - Move the 20 core `.rs` files + trimmed `lib.rs` (21 files total) + `testdata/fake_cli.sh` + `tests/kill_matrix.rs` + `tests/power_cut.rs` into `daemon/sessions_core`.
   - Make `daemon/sessions` depend on `sessions_core` (and `sessions_faults` depend on `sessions_core_faults`), re-exporting `humata2_sessions_core::*`. Keep `desk*.rs` and `carry*.rs` in `daemon/sessions` temporarily, updating their `crate::` imports to `humata2_sessions_core::`.
3. **Extract `apps/humata2/daemon/sessions_desk` (`sessions_desk` + `sessions_desk_faults`):**
   - Add `//apps/humata2/daemon/sessions_desk:__pkg__` visibility to `durable`, `revive`, `store`.
   - Move `doc_repo.rs`, `desk.rs`, `desk_move.rs`, `channel_desk.rs` into `daemon/sessions_desk` with `sessions_desk_unit_test`.
   - Re-export `humata2_sessions_desk::*` from `daemon/sessions`.
4. **Extract `apps/humata2/daemon/sessions_carry` (`sessions_carry` + `sessions_carry_faults`):**
   - Add `//apps/humata2/daemon/sessions_carry:__pkg__` visibility to `conversation`, `durable`, `holder`, `revive`, `store`.
   - Move `bring.rs` and all 10 `carry*.rs` files into `daemon/sessions_carry` with `sessions_carry_unit_test`.
   - Re-export `humata2_sessions_carry::*` from `daemon/sessions`.
5. **Re-point `daemon/humatad` (or Phase 4 `svc_*` crates) and retire facade:**
   - Switch `daemon/humatad/BUILD.bazel` (and individual `svc_*` crates) to depend directly on `sessions_core`, `sessions_desk`, and `sessions_carry` (plus `*_faults` twins where needed).
   - Delete the empty `daemon/sessions` facade package and remove its stale visibility entries.

---

## Part 2 — Phase 4B: `apps/humata2/clients/live`

### 2.1 Proposed crates and file mapping

All 32 files under `apps/humata2/clients/live/src/` (17,225 LOC total) map to **7 focused crates** plus a thin `clients/live` re-export facade:

```
                         ┌────────────────────────────┐
                         │ desktop (app & live tests) │
                         └─┬──────┬──────┬──────┬───┬─┘
           ┌───────────────┘      │      │      │   └────────────────┐
           ▼                      ▼      ▼      ▼                    ▼
 ┌───────────────────┐  ┌──────────────┐ │ ┌──────────────┐ ┌──────────────────┐
 │ clients/live_     │  │ clients/live │ │ │ clients/live │ │ clients/live_    │
 │   machines (LEAF) │  │ _files(LEAF) │ │ │_review(LEAF) │ │   daemon         │
 │   (2,181 LOC)     │  │ (1,674 LOC)  │ │ │ (3,075 LOC)  │ │   (6,290 LOC)    │
 └─────────┬─────────┘  └──────┬───────┘ │ └──┬───────┬───┘ └──┬──────┬─────┬──┘
           │                   │         │    │       │        │      │     │
           │                   │         ▼    │       ▼        │      │     ▼
           │                   │  ┌───────────┴──┐  ┌──────────┴──┐   │  (links
           │                   │  │clients/live_ │  │clients/live_│   │  terminal_link
           │                   │  │desk (988 LOC)│  │diff(802 LOC)│   │  + wire)
           │                   │  └──────┬───────┘  └──────┬──────┘   │
           │                   │         │                 │          │
           └───────────────────┴─────────┴────────┬────────┴──────────┘
                                                  ▼
                                  ┌───────────────────────────────┐
                                  │     clients/live_core         │
                                  │  (2,250 LOC: transport,       │
                                  │   LiveConnection, map, time,  │
                                  │   recorded, finish_operation) │
                                  └───────────────────────────────┘
```

| Proposed crate (`crate_name`) | Target dir | Files from `clients/live/src/` | LOC | Internal live deps | External highlights |
|---|---|---|---:|---|---|
| `live_core` (`humata2_clients_live_core`) | `apps/humata2/clients/live_core` | `lib.rs` (new), `connection.rs` (**transport half**, ~450 LOC: `LiveConnection`, `LiveOptions`, `ConnectError`, `DaemonFacts`, `Wire`, `Credentials`, `MESSAGE_MAX_BYTES`, `spawn`, `error`, `status_error`, `daemon_error`, `said`, `fresh_key`, `key_of`, `not_in_this_release`, `bad_repo` from `daemon.rs:868`, `finish_operation` from `daemon.rs:28–103` with `OpUpdate` sink), `recorded.rs` (346), `local_time.rs` (169, **absorbs** `parse_offset` from `file_manager.rs:526` and `ago` from `boards.rs:156`), `map.rs` (1,164, **absorbs** pure `pb::Comment -> Comment` converter from `comments.rs:222`) — **4 files** | ~2,250 | none | `clients` (or `clients/model` + `clients/api`), `grpc`, `tonic`, `prost`, `tower`, `hyper-util`, `tokio`, `uuid`. **Drops** `terminal_link`, `terminal/wire`, `sha2`. |
| `live_machines` (`humata2_clients_live_machines`) | `apps/humata2/clients/live_machines` | `lib.rs` (new), `machines.rs` (1,887), `machines/providers.rs` (294) — **2 files** | ~2,181 | `live_core` | **100% leaf.** Exports `LiveMachines` (`impl Machines`). |
| `live_files` (`humata2_clients_live_files`) | `apps/humata2/clients/live_files` | `lib.rs` (new), `file_manager.rs` (1,674) — **1 file** | ~1,674 | `live_core` | **100% leaf.** Only crate needing `@crates//:sha2`. Exports `LiveFileManager` (`impl FileManager`). |
| `live_diff` (`humata2_clients_live_diff`) | `apps/humata2/clients/live_diff` | `lib.rs` (new), `diff.rs` (802) — **1 file** | ~802 | `live_core` | Exports `file_tier`, `spec_to_wire`, and the 9 `Daemon` diff/push/file-read RPC helpers taking `&LiveConnection`. Prevents `live_daemon -> live_review` coupling. |
| `live_review` (`humata2_clients_live_review`) | `apps/humata2/clients/live_review` | `lib.rs` (new), `comments.rs` (947), `views.rs` (846), `pull_requests.rs` (660), `review_chat.rs` (622) — **4 files** | ~3,075 | `live_core`, `live_diff` | **100% leaf.** Exports `LiveComments`, `LiveViews`, `LivePullRequests`, `LiveReviewChat`. |
| `live_desk` (`humata2_clients_live_desk`) | `apps/humata2/clients/live_desk` | `lib.rs` (new), `desk.rs` (799), `docs.rs` (189) — **2 files** | ~988 | `live_core` | Exports `LiveDesk` (`impl DeskClient`, owns its `Mutex<Option<HeldPlan>>`), plus `pub async fn load_desk` and `pub fn change` for `live_daemon`. |
| `live_daemon` (`humata2_clients_live_daemon`) | `apps/humata2/clients/live_daemon` | `lib.rs` (new), `state.rs` (`LiveDaemon` + `DaemonState` split from `connection.rs`, ~280 LOC), `daemon.rs` (1,340), `inbox.rs` (1,169), `link.rs` (777), `boards.rs` (529), `terminals.rs` (386), `events.rs` (383), `repos.rs` (331), `marks.rs` (250), `setup.rs` (182), `running.rs` (173), `history.rs` (167), `agent_switches.rs` (108), `read_marks.rs` (91), `settings.rs` (45), `message.rs` (42), `session_repos.rs` (39), `channel_repos.rs` (38) — **17 files** | ~6,290 | `live_core`, `live_diff`, `live_desk` | Only crate needing `clients/terminal_link` and `terminal/wire`. Exports `LiveDaemon` (`impl Daemon`, `impl Terminals`), `history::LiveHistory`, `marks::LiveMarks`. |
| `clients/live` (thin facade) | `apps/humata2/clients/live` | `lib.rs` (~46) | ~46 | All 6 domain live crates | Re-exports `LiveOptions`, `ConnectError`, `DaemonFacts`, `Call`, `Observer`, `LiveDaemon`, `LiveDesk`, `LiveMachines`, `LiveFileManager`, `LiveComments`, `LivePullRequests`, `LiveReviewChat`, `LiveViews`, `history::LiveHistory`, `marks::LiveMarks`. |

- **Note on `Cargo.toml`:** `apps/humata2/clients/live/Cargo.toml` is listed in root `Cargo.toml:46` (Bazel-only because of `:grpc`). Either add the new sub-crate `Cargo.toml`s to root `Cargo.toml` or keep them Bazel-only consistently with `daemon/sessions`.

### 2.2 Cross-crate references (`clients/live`)

#### Structural boundary 1: `Shared` -> `LiveConnection` + `DaemonState`, and orphan rules (`E0117`)

Today, 20 files write inherent `impl Shared` blocks (`connection.rs` defines `Shared` and `LiveDaemon`), `daemon.rs:915` writes `impl Daemon for LiveDaemon`, `terminals.rs:303` writes `impl Terminals for LiveDaemon`, and `desk.rs:616` writes `impl DeskClient for LiveDaemon`.

1. **Split `Shared` into `LiveConnection` (`live_core`) and `DaemonState` (`live_daemon`):**
   - `LiveConnection` (cheaply `Clone`, backed by `Arc`) holds `wire: Wire`, `agents: AgentNames`, `machine: MachineId`, `facts: DaemonFacts`, `runtime: Handle`, and an `OpUpdate` progress sink (`OnceLock<Arc<dyn Fn(OpUpdate) + Send + Sync>>` registered by `LiveDaemon`), plus `connect(LiveOptions)`, `spawn()`, `error()`, `status_error()`, `daemon_error()`, `said()`, `fresh_key()`, `key_of()`, `not_in_this_release()`, `bad_repo()`, and `finish_operation(op, what)`.
   - None of `LiveMachines`, `LiveFileManager`, `LiveComments`, `LivePullRequests`, `LiveReviewChat`, `LiveViews`, `LiveHistory`, `LiveMarks`, `diff.rs`, or `docs.rs` touch `DaemonState`.
   - `LiveDaemon` (`live_daemon`) wraps `{ conn: LiveConnection, state: Arc<Mutex<DaemonState>> }`, implements `AsRef<LiveConnection>` (and `pub fn connection(&self) -> &LiveConnection`), registers its `Event::Operation` progress sink on `conn`, and keeps `impl Daemon for LiveDaemon` + `impl Terminals for LiveDaemon` in `live_daemon`.
   - Every domain client constructor accepts `conn: impl AsRef<LiveConnection>`.
2. **Introduce `LiveDesk` in `live_desk` (`E0117`):**
   - `desk.rs:616` (`impl DeskClient for LiveDaemon`) becomes `impl DeskClient for LiveDesk`, where `LiveDesk` owns `{ conn: LiveConnection, held_plan: Arc<Mutex<Option<HeldPlan>>> }` (removing `desk_plan` from `DaemonState`).
   - **All 7 call sites** currently relying on `impl DeskClient for LiveDaemon` must switch to `LiveDesk::new(&daemon)`:
     1. `apps/humata2/desktop/src/live.rs:310` (`desk: Arc::new(daemon.clone())`)
     2. `apps/humata2/desktop/tests/live_boards.rs:372` (`desk: Arc::new(daemon.clone())`)
     3. `apps/humata2/desktop/tests/live_inbox.rs:472` (`desk: Arc::new(daemon.clone())`)
     4. `apps/humata2/desktop/tests/live_machines.rs:211` (`desk: Arc::new(daemon.clone())`)
     5. `apps/humata2/desktop/tests/live_flows/world.rs:394` (`desk: Arc::new(daemon.clone())`)
     6. `apps/humata2/clients/live/tests/support/real.rs:201` (`let desk: Arc<dyn DeskClient> = live;`)
     7. `apps/humata2/clients/live/tests/file_manager.rs:560` (`live.start(NewDeskSession { ... })`)
3. **Move gRPC client factory methods out of `connection.rs`:**
   - Move the 14 `Shared::*()` client factories (`connection.rs:305–387`) into the domain crate that calls each service (using `conn.wire()` + `MESSAGE_MAX_BYTES`).
4. **Preserve live PROGRESS updates in `finish_operation` (`daemon.rs:28–103`):**
   - `desk.rs:520`, `diff.rs:352`, and `pull_requests.rs:393,428` publish `Event::Operation(OpUpdate)` during `finish()`. Wiring the `OpUpdate` sink on `LiveConnection` (registered when `LiveDaemon` wraps `LiveConnection`) preserves identical progress dialog behavior without `live_core -> live_daemon`.

#### Exhaustive `crate::` reference table (`clients/live`)

| Source file (new crate) | Line(s) | Current `crate::` / `Shared` reference | Target crate | Resolution |
|---|---|---|---|---|
| `connection.rs` (`live_core`) | L29, L159 | `crate::events::{Subscriber, Move}` | `live_daemon` | Moves out of `live_core` into `live_daemon/src/state.rs` (`DaemonState`) |
| `connection.rs` (`live_core`) | L155 | `crate::inbox::Seen` | `live_daemon` | Moves into `live_daemon/src/state.rs` |
| `connection.rs` (`live_core`) | L157 | `crate::desk::HeldPlan` | `live_desk` | Moves into `LiveDesk`'s private `Mutex<Option<HeldPlan>>` in `live_desk` |
| `connection.rs` (`live_core`) | L161 | `crate::read_marks::Viewing` | `live_daemon` | Moves into `live_daemon/src/state.rs` |
| `local_time.rs` (`live_core`) | L81 | `crate::file_manager::parse_offset` | `live_files` | **Reverse cycle!** Move `parse_offset` (`file_manager.rs:526`, 20 LOC) into `live_core::local_time` |
| `machines.rs` (`live_machines`) | L28, L143 | `crate::connection::{LiveDaemon, Shared, Wire, fresh_key, not_in_this_release, said}` + `shared.{error, spawn, operations}` | `live_core` | Replace `LiveDaemon`/`Shared` with `LiveConnection`; build `OperationServiceClient::new(conn.wire())` |
| `machines.rs` (`live_machines`) | L29, L127, L1108, L1249 | `crate::local_time::LocalNow` | `live_core` | `humata2_clients_live_core::local_time::LocalNow` |
| `machines.rs` (`live_machines`) | L128, L1109, L1250 | `crate::map::machine_stop` | `live_core` | `humata2_clients_live_core::map::machine_stop` |
| `machines/providers.rs` (`live_machines`) | L17 | `crate::connection::{Shared, fresh_key}` | `live_core` | `humata2_clients_live_core::{LiveConnection, fresh_key}` |
| `file_manager.rs` (`live_files`) | L34, L144 | `crate::connection::{LiveDaemon, Shared, Wire, fresh_key, said}` + `shared.spawn` | `live_core` | Replace `LiveDaemon`/`Shared` with `LiveConnection`; convert `impl Shared` (`L53–69`) to free functions on `&LiveConnection` |
| `diff.rs` (`live_diff`) | L11–L12 | `crate::connection::{Shared, fresh_key}`, `crate::map` | `live_core` | Take `&LiveConnection`; call `humata2_clients_live_core::map::{parse_repo, repo_text}` |
| `diff.rs` (`live_diff`) | L280, L337, L352, L370, L401, L432, L481, L501 | `self.diffs()`, `self.sessions()`, `self.finish()`, `self.error()` | `live_core` | Build `DiffServiceClient`/`SessionServiceClient` from `conn.wire()`; call `conn.finish_operation(...)` and `conn.error(...)` |
| `comments.rs` (`live_review`) | L19–L20 | `crate::connection::{LiveDaemon, Shared, fresh_key}`, `crate::map` | `live_core` | Take `impl AsRef<LiveConnection>`; call `live_core::map::{parse_repo, repo_text, session_id}` |
| `comments.rs` (`live_review`) | L274–L323 | `impl Shared` (`list_comments`, `watch_comments`) + `self.{comments, error, spawn}` | `live_core` | Methods on `LiveComments` using `LiveConnection` + local `CommentServiceClient` |
| `views.rs` (`live_review`) | L20–L21, L44 | `crate::connection::{LiveDaemon, Shared, Wire, fresh_key}`, `crate::map::{self, AgentNames}` | `live_core` | Use `LiveConnection` + `live_core::map` |
| `views.rs` (`live_review`) | L132 | `crate::diff::file_tier` | `live_diff` | `humata2_clients_live_diff::file_tier` |
| `views.rs` (`live_review`) | L438 | `crate::boards::ago` | `live_daemon` | **Cross-domain helper!** Move `ago` (`boards.rs:156`, ~20 LOC) to `live_core::local_time::ago` |
| `views.rs` (`live_review`) | L450 | `impl Shared` (`list_views`, `watch_views`) | `live_core` | Methods on `LiveViews` using `LiveConnection` |
| `review_chat.rs` (`live_review`) | L20–L21 | `crate::connection::{LiveDaemon, Shared, Wire, fresh_key}`, `crate::map` | `live_core` | Use `LiveConnection` + `live_core::map` |
| `review_chat.rs` (`live_review`) | L21, L187 | `crate::diff::spec_to_wire` | `live_diff` | `humata2_clients_live_diff::spec_to_wire` |
| `pull_requests.rs` (`live_review`) | L23–L24, L73 | `crate::connection::{LiveDaemon, Shared, Wire, daemon_error, fresh_key, said}`, `crate::map` | `live_core` | Use `LiveConnection` + `live_core::map` |
| `pull_requests.rs` (`live_review`) | L393, L428 | `shared.finish(...)` (from `daemon.rs:28`) | `live_core` | Call `conn.finish_operation(...)` in `live_core` |
| `pull_requests.rs` (`live_review`) | L571 | `shared.event_stream()` | `live_core` | Build `EventServiceClient::new(conn.wire())` locally |
| `desk.rs` (`live_desk`) | L17, L19 | `crate::connection::{LiveDaemon, Shared, fresh_key, not_in_this_release}`, `crate::map` | `live_core` | Use `LiveDesk(LiveConnection)` + `live_core::map` |
| `desk.rs` (`live_desk`) | L18, L49 | `crate::daemon::bad_repo` | `live_daemon` | **Move** `bad_repo` (`daemon.rs:868`, 6 LOC) to `live_core` |
| `desk.rs` (`live_desk`) | L44 | `crate::boards::ago` | `live_daemon` | **Move** `ago` (`boards.rs:156`) to `live_core::local_time::ago` |
| `desk.rs` (`live_desk`) | L520 | `self.finish(...)` (from `daemon.rs:28`) | `live_core` | Call `conn.finish_operation(...)` in `live_core` |
| `desk.rs` (`live_desk`) | L557, L581, L599 | `self.state().desk_plan` | `live_desk` | Moves into `LiveDesk`'s own `Mutex<Option<HeldPlan>>` |
| `docs.rs` (`live_desk`) | L9 | `crate::comments::comments` (`comments.rs:222`) | `live_review` | **Breaks `live_desk -> live_review`!** Move pure `pb::Comment -> Comment` mapper (~60 LOC) to `live_core::map::comments` |
| `docs.rs` (`live_desk`) | L10–L11 | `crate::connection::{Shared, fresh_key}`, `crate::map` | `live_core` | Methods on `LiveDesk` / `&LiveConnection` |
| `daemon.rs` (`live_daemon`) | L19–L20, L41, L225 | `crate::connection::*`, `crate::local_time::LocalNow::read`, `crate::map` | `live_core` | `humata2_clients_live_core::*` |
| `daemon.rs` (`live_daemon`) | L312 | `self.load_desk(&ids)` | `live_desk` | `humata2_clients_live_desk::load_desk(&self.conn, &ids)` |
| `daemon.rs` (`live_daemon`) | L916–L974 | `shared.{read_session_file, find_session_files, session_diffs, repo_diff, repo_bases, find_default_branch, mark_reviewed, plan_push, push}` | `live_diff` | `humata2_clients_live_diff::*(&self.conn, ...)` |
| `events.rs` (`live_daemon`) | L11–L12, L229 | `crate::connection::Shared`, `crate::map`, `crate::local_time::LocalNow::read` | `live_core` | `humata2_clients_live_core::*` |
| `events.rs` (`live_daemon`) | L287 | `crate::desk::change(event.change, &wire)` | `live_desk` | `humata2_clients_live_desk::change(event.change, &wire)` |
| `boards.rs`, `inbox.rs`, `terminals.rs`, `link.rs`, `repos.rs`, `running.rs`, `marks.rs`, `history.rs`, `read_marks.rs`, `settings.rs`, `agent_switches.rs`, `channel_repos.rs`, `session_repos.rs`, `setup.rs` (`live_daemon`) | top imports | `crate::connection::*`, `crate::map::*` | `live_core` | Replace `crate::connection` / `crate::map` with `humata2_clients_live_core::*`; `Shared` becomes `LiveDaemon`'s internal `DaemonShared` |

### 2.3 Test re-pointing (`clients/live` and `desktop`)

#### Unit tests (1 -> 7 targets)

- `//apps/humata2/clients/live:live_unit_test` splits into `live_core_unit_test`, `live_machines_unit_test`, `live_files_unit_test`, `live_diff_unit_test`, `live_review_unit_test`, `live_desk_unit_test`, and `live_daemon_unit_test`.

#### Integration tests in `apps/humata2/clients/live/BUILD.bazel` (13 existing targets)

> **Shared test support (`tests/support/{cases,fake,real}.rs`):** `tests/support/real.rs:12,188–203` currently constructs `LiveComments` and `LiveDesk`. Extract only the daemon spawner + `start()`/`start_daemon()` into `//apps/humata2/clients/live_test_support` (deps: `live_core` + `live_daemon` only), and move `start_comments` (+ comment parity cases) into `clients/live_review/tests/` and `start_desk` into `clients/live_desk/tests/`.

| Current target (`tests/*.rs`) | Moves to package (in Step 8, after `live_daemon` & `live_test_support` exist) | New live crate `deps` | Target scoped daemon binary (`SPLIT.md` §3.3 Tier B) |
|---|---|---|---|
| `machines_test` (`tests/machines.rs`) | `clients/live_machines` | `:live_machines`, `live_core`, `live_daemon` (or `LiveConnection` directly) | `humatad2_test_machines` (+ `humatad2` release for fake box) |
| `file_manager_test` (`tests/file_manager.rs`) | `clients/live_files` | `:live_files`, `live_core`, `live_desk`, `live_daemon` | `humatad2_test_files` (`files` + `transfers` + `desk` + `sessions_local` + `channels`) |
| `desk_live_test` (`tests/desk.rs`) | `clients/live_desk` | `:live_desk`, `live_core`, `live_daemon`, `live_test_support` | `humatad2_test_desk` (`desk` + `sessions_local` + `channels`) |
| `diffs_live_test` (`tests/diffs.rs`) | `clients/live_diff` | `:live_diff`, `live_core`, `live_daemon` | `humatad2_test_review_remote` (`review` + `sessions_local` + `channels`) |
| `pull_requests_live_test` (`tests/pull_requests.rs`) | `clients/live_review` | `:live_review`, `live_core`, `live_daemon` | `humatad2_test_review_remote` |
| `review_chat_live_test` (`tests/review_chat.rs`) | `clients/live_review` | `:live_review`, `live_core`, `live_daemon`, `live_test_support` | `humatad2_test_review_remote` |
| `parity_test` (`tests/parity.rs`) — **split into 2** | 1. `clients/live_daemon:parity_daemon_test` (13 `parity!` cases)<br>2. `clients/live_review:parity_comments_test` (7 `parity_comments!` cases) | 1. `:live_daemon`, `live_core`, `live_test_support`<br>2. `:live_review`, `live_daemon`, `live_core`, `live_test_support` | 1. `humatad2_test_sessions`<br>2. `humatad2_test_review_remote` |
| `boards_test` (`tests/boards.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` | `humatad2_test_boards` |
| `inbox_test` (`tests/inbox.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` (+ `//apps/humata2/cli:humata`) | `humatad2_test_inbox` |
| `github_repos_live_test` (`tests/github_repos.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` | `humatad2_test_catalog` |
| `marks_live_test` (`tests/marks.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` | `humatad2_test_catalog` |
| `history_live_test` (`tests/history.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` | `humatad2_test_catalog` |
| `term_restart_live_test` (`tests/term_restart.rs`) | `clients/live_daemon` | `:live_daemon`, `live_core` (+ `ghostty_vt`, `terminal/wire`) | `humatad2_test_terminal` |

#### Desktop targets (`apps/humata2/desktop/BUILD.bazel`)

- `humata2`, `humata2_fakes`, `live_flows_cases`, `live_flows_ssh_test`, `live_machines_test`: depend on `//apps/humata2/clients/live` facade (or all 7 sub-crates).
- `live_inbox_test`, `live_boards_test`: depend on `live_core`, `live_daemon`, `live_desk`, `live_review` — **drop `live_machines` and `live_files`**.

### 2.4 Order of moves (`clients/live`)

Every step builds and passes all tests on its own. **Note:** Keep the 13 integration tests in `clients/live/BUILD.bazel` until Step 8 (when `live_daemon` and `live_test_support` exist), because all 13 import `LiveDaemon`.

1. **Prep inside `clients/live` (zero new crates yet):**
   - Widen Bazel `visibility` from `//apps/humata2/clients/live:__pkg__` to `//apps/humata2/clients:__subpackages__` (or explicit package list) in: `apps/humata2/clients/BUILD.bazel:42,55`, `apps/humata2/clients/terminal_link/BUILD.bazel:10`, `apps/humata2/terminal/wire/BUILD.bazel:9`, `apps/humata2/daemon/machines/BUILD.bazel:75` (`:fake_machine`), and `apps/humata2/daemon/humatad/BUILD.bazel:45,196,227` (`:grpc`, `:holder_for_clients`, `:humatad2_admin`).
   - Move `parse_offset` (`file_manager.rs:526`) and `ago` (`boards.rs:156`) into `local_time.rs`; move `bad_repo` (`daemon.rs:868`) into `connection.rs`; move pure `comments::comments` (`comments.rs:222`) into `map.rs`.
   - Split `Shared` inside `connection.rs` into `LiveConnection` (with `OpUpdate` progress sink + `finish_operation`) + `DaemonState`.
   - Introduce `pub struct LiveDesk` (holding `LiveConnection` + `Mutex<Option<HeldPlan>>`), move `impl DeskClient` from `LiveDaemon` to `LiveDesk`, re-export `LiveDesk`, and update all **7** `DeskClient` call sites listed in §2.2.
   - Move the 14 service client factories out of `connection.rs:305–387` into their calling modules, and update all domain `Live*::new` constructors to accept `impl AsRef<LiveConnection>`.
2. **Extract `apps/humata2/clients/live_core`:** Move `connection.rs` (transport half), `recorded.rs`, `local_time.rs`, and `map.rs` + `live_core_unit_test`. Re-export from `clients/live`.
3. **Extract `apps/humata2/clients/live_machines` (leaf):** Move `machines.rs`, `machines/providers.rs` + `live_machines_unit_test`. Re-export `LiveMachines` from `clients/live`.
4. **Extract `apps/humata2/clients/live_files` (leaf):** Move `file_manager.rs` + `live_files_unit_test` (moving `@crates//:sha2` dep with it). Re-export `LiveFileManager` from `clients/live`.
5. **Extract `apps/humata2/clients/live_diff`:** Move `diff.rs` + `live_diff_unit_test`; update `daemon.rs` to call `humata2_clients_live_diff::*(&self.conn, ...)`.
6. **Extract `apps/humata2/clients/live_review` (leaf):** Move `comments.rs`, `views.rs`, `pull_requests.rs`, `review_chat.rs` + `live_review_unit_test`. Re-export from `clients/live`.
7. **Extract `apps/humata2/clients/live_desk`:** Move `desk.rs`, `docs.rs` + `live_desk_unit_test`; update `daemon.rs:312` and `events.rs:287` to call `humata2_clients_live_desk::{load_desk, change}`. Re-export `LiveDesk` from `clients/live`.
8. **Extract `apps/humata2/clients/live_daemon` + `live_test_support` and migrate all 13 integration tests:**
   - Move the remaining 17 files + `live_daemon_unit_test` into `clients/live_daemon` (carrying `terminal_link` and `terminal/wire` deps), leaving `clients/live` as a 46-line facade.
   - Create `//apps/humata2/clients/live_test_support` (spawner + `start`/`start_daemon`), split `support/{cases,fake}.rs`, and move all 13 integration tests (splitting `parity_test` into `parity_daemon_test` + `parity_comments_test`) into their owning `clients/live_*` packages.
   - Trim `desktop/BUILD.bazel` (`live_inbox_test` and `live_boards_test` drop `live_machines` and `live_files`).

---

## Part 3 — Phase 5: `apps/humata2/api/proto` & `:grpc` by Domain

### 3.1 Technical prerequisites

Every `.proto` file in `apps/humata2/api/proto/humata/v1/` declares `package humata.v1;`. **Do not change the proto package name:** gRPC routes by `/humata.v1.<Service>/<Method>` and `clients/live/src/connection.rs:442` decodes error details matching `/humata.v1.Error`.

1. **Same-package `rust_prost_domain` rule (`apps/humata2/api/proto/prost.bzl`):**
   - Widen visibility of `//apps/humata/proto/plugin:protoc-gen-prost` (`apps/humata/proto/plugin/BUILD.bazel:8`, currently `//apps/humata/proto:__pkg__`) to include `//apps/humata2/api/proto:__pkg__` (or define a local `protoc-gen-prost` target).
   - Run `protoc-gen-prost` on `info.direct_sources` with `--descriptor_set_in=info.transitive_descriptor_sets`, `--prost_opt=no_include,flat_output_dir`, **and `--prost_opt=extern_path=.google.protobuf=::prost_types`** (so `google.protobuf.Timestamp` and `google.protobuf.Duration` resolve uniformly across every domain crate, re-exporting `pub use prost_types::{Duration, Timestamp};` in `core_rust` and updating `daemon/review_diff/src/wire.rs:12`).
   - Wrap the generated `humata.v1.rs` in:
     ```rust
     pub mod humata {
         pub mod v1 {
             pub use humata2_proto_core::humata::v1::*; // + direct domain proto deps
             include!(env!("HUMATA2_PROST_RS"));
         }
     }
     ```
   - Consumers depending on ≥2 sibling domain crates either import from each crate directly or define a 3-line local `mod pb { pub use humata2_grpc_a::humata::v1::*; pub use humata2_grpc_b::humata::v1::*; }`.
2. **Per-domain tonic crates & `cli/descriptors.bzl`:**
   - Move `tonic.bzl` and `protoc-gen-tonic` from `daemon/humatad/` to `apps/humata2/api/grpc/` (keeping an alias in `daemon/humatad` during migration).
   - Update `apps/humata2/cli/descriptors.bzl:27` to accept `protos = attr.label_list(providers = [ProtoInfo])` so `//apps/humata2/cli:descriptors` still embeds every message for JSON output.
3. **Daemon decoupling prerequisite for the Phase 5 cache win:**
   - Because `proto_events` depends on **9 domain protos**, any daemon crate that constructs or matches `pb::Event`/`pb::event::Payload` transitively depends on all 9 domain protos. Today, `daemon/runtime::events` (`events.rs:42,122`) and almost every `svc_*` crate directly touch `pb::event::Payload`, and `svc_peer` (`relay.rs:67+`, `host_relay.rs:283+`) mounts/clients all 12 proto domains.
   - To achieve the headline Phase 5 payoff (`review.proto` / `boards.proto` / `desk.proto` edits **not** invalidating `humatad2_test_power_cut`):
     1. **Decouple `daemon/runtime::events` from `pb::Event`:** store opaque protobuf-encoded event frames `(Topic, EventSeq, Bytes)` in `runtime::events` so leaf services encode only their own domain event payload without importing `proto_events`; only `EventService` (in `daemon/humatad` / `svc_events`) depends on `proto_events`.
     2. **Decouple `svc_peer`'s full-service relay wiring from `humatad2_test_power_cut`:** keep generic `Channel`/`LinkState` supervision in `svc_peer`, and wire per-service gRPC relay forwarding in `daemon/humatad` (or via transport-level raw forwarding) so `humatad2_test_power_cut` only links `sessions`, `peer`, `inbox`, and `core` protos.

### 3.2 Proposed crates and file mapping

All 21 `humata/v1/*.proto` files (**9,078 LOC**) plus `humata/store/v1/store.proto` (123 LOC) map to **11 domain proto+grpc pairs** plus `proto_events` at the top of the DAG (34 gRPC services total):

| Proposed proto + grpc targets (`api/proto:<name>_{proto,rust}` + `api/grpc:<name>`) | `.proto` files absorbed | LOC | Proto `deps` (after §3.3 moves) | gRPC services generated (34 total) | Primary Rust consumers |
|---|---|---:|---|---|---|
| `core` (`humata2_proto_core`, `humata2_grpc_core`) | `common.proto` (309), `build.proto` (45), `capabilities.proto` (207), `errors.proto` (622), `access.proto` (466), `operations.proto` (154) + 4 moved type groups (~153 LOC) | ~1,956 | `@protobuf//:duration_proto`, `@protobuf//:timestamp_proto` | `AccessService`, `CliAuthService`, `AuditService`, `OperationService` (4) | Every domain crate, `daemon/rpc_kit`, `clients/live_core` |
| `terminal` (`humata2_proto_terminal`, `humata2_grpc_terminal`) | `terminal.proto` (355) | 355 | `core` | `TerminalService` (1) | `clients/terminal_link` (prost only), `daemon/svc_terminal`, `clients/live_daemon` |
| `catalog` (`humata2_proto_catalog`, `humata2_grpc_catalog`) | `repos.proto` (128), `history.proto` (133), `settings.proto` (335) | 596 | `core` | `RepoService`, `HistoryService`, `SettingsService`, `AgentService`, `MarkService`, `ViewStateService` (6) | `daemon/svc_catalog`, `clients/live_daemon` |
| `files` (`humata2_proto_files`, `humata2_grpc_files`) | `files.proto` (501) | 501 | `core` | `FileService`, `TransferService`, `TrashService`, `TransferEndService` (4) | `daemon/svc_files`, `daemon/svc_transfers`, `clients/live_files`, `clients/live_diff` |
| `inbox` (`humata2_proto_inbox`, `humata2_grpc_inbox`) | `inbox.proto` (533) | 533 | `core` | `InboxService` (1) | `daemon/svc_inbox`, `proto_peer`, `clients/live_daemon` |
| `board` (`humata2_proto_board`, `humata2_grpc_board`) | `boards.proto` (222) | 222 | `core` | `BoardService` (1) | `daemon/svc_board`, `clients/live_daemon` |
| `review` (`humata2_proto_review`, `humata2_grpc_review`) | `review.proto` (1,286) | 1,286 | `core` | `DiffService`, `CommentService`, `PullRequestReviewService`, `ViewService`, `ReviewChatService` (5) | `daemon/review_diff` (prost only), `daemon/svc_review`, `clients/live_diff`, `clients/live_review` |
| `desk` (`humata2_proto_desk`, `humata2_grpc_desk`) | `desk.proto` (559) | 559 | `core`, `review` (`Comment`, `Hunk`) | `DeskService` (1) | `daemon/svc_desk`, `clients/live_desk` |
| `machines` (`humata2_proto_machines`, `humata2_grpc_machines`) | `hosts.proto` (1,010) | 1,010 | `core` | `MachineService`, `ProviderService`, `SkillsService`, `BridgeService` (4) | `daemon/svc_machines`, `clients/live_machines`, `clients/live_files` (`GetStorage`) |
| `sessions` (`humata2_proto_sessions`, `humata2_grpc_sessions`) | `channels.proto` (327), `sessions.proto` (857) | 1,184 | `core` | `ChannelService`, `ActivityService`, `SessionService`, `SessionCarryService` (4) | `daemon/svc_channels`, `daemon/svc_sessions_*`, `proto_peer`, `clients/live_daemon`, `clients/live_diff` |
| `peer` (`humata2_proto_peer`, `humata2_grpc_peer`) | `peer.proto` (608), `daemon.proto` (220) | 828 | `core`, `inbox` (`HolderSession`, `RequestAnswer`), `sessions` (`Pointer`, `Turn`) | `PeerService`, `DaemonService` (2) | `daemon/svc_peer`, `daemon/svc_lifecycle`, `clients/live_machines` |
| `events` (`humata2_proto_events`, `humata2_grpc_events`) | `events.proto` (201) | 201 | `core`, `board`, `catalog`, `desk`, `files`, `inbox`, `machines`, `review`, `sessions` | `EventService` (1) | `svc_events` / `daemon/humatad`, `clients/live_daemon`, `clients/live_review`, `cli` |
| `humata2_store_proto` (unchanged) | `humata/store/v1/store.proto` (123) | 123 | none | (storage only) | `daemon/store`, `daemon/humatad` |
| **Umbrella facades** | `api/proto:humata2_proto`, `:humata2_rust`, `daemon/humatad:grpc` | ~30 | All 12 domain crates | Re-exports all 34 services under `humata::v1` | `cli`, `daemon/humatad` (thin binary), `svc_mcp`, `remote_rig`, `desktop:live_flows_cases` |

### 3.3 Cross-crate references (`.proto` imports and Rust `humata2_grpc` boundaries)

#### A. Every non-core cross-file `.proto` import across the 21 `humata/v1/*.proto` files

| Source `.proto` (new crate) | Current non-core `import` | Exact symbols referenced | Resolution |
|---|---|---|---|
| `sessions.proto` (`proto_sessions`) | `humata/v1/review.proto` | `DiffSpec`, `PullRequestState` (+ transitive `RepoDiffSpec`, `DiffSpecBase`, `DiffSpecCompare`, `DiffWhitespace`, `ForkPointBase`, `IndexBase`, `PullRequestBaseRef` — 9 types, 75 LOC) | **Move 1 (wire-identical):** Move all 9 types into `common.proto` (`proto_core`). **Severs `proto_sessions -> proto_review`.** |
| `hosts.proto` (`proto_machines`) | `humata/v1/channels.proto` | `SessionDisposition`, `SessionWork`, `ArchiveChoice` (3 types, 20 LOC) | **Move 2 (wire-identical):** Move all 3 types into `common.proto` (`proto_core`). **Severs `proto_machines -> proto_sessions`.** |
| `boards.proto` (`proto_board`) | `humata/v1/inbox.proto` | `BoardChangePreview`, `FieldChange` (2 types, 11 LOC) | **Move 3 (wire-identical):** Move both types into `common.proto` (`proto_core`). **Severs `proto_board -> proto_inbox`.** |
| `desk.proto` (`proto_desk`) | `humata/v1/files.proto` | `FileLocation` + **8 sibling root messages** (`files.proto:56–106`: `SessionClone`, `ChannelShared`, `MainClone`, `ChannelFolder`, `MachineHome`, `SessionPastes`, `SessionFolder`, `MachineReceived` — 9 types, 41 LOC) | **Move 4a (wire-identical):** Move all 9 `FileLocation` messages into `common.proto` (`proto_core`). **Severs `proto_desk -> proto_files`.** |
| `desk.proto` (`proto_desk`) | `humata/v1/review.proto` | `Comment`, `Hunk` | **Keep** as `proto_desk -> proto_review` dep (`DocVersionText` / `DocVersionDiff` genuinely return `Comment`/`Hunk`). |
| `peer.proto` (`proto_peer`) | `humata/v1/inbox.proto` | `HolderSession`, `RequestAnswer` | **Keep** as `proto_peer -> proto_inbox` dep. |
| `peer.proto` (`proto_peer`) | `humata/v1/sessions.proto` | `Pointer`, `Turn` | **Keep** as `proto_peer -> proto_sessions` dep. |
| `daemon.proto` (`proto_peer`) | `humata/v1/peer.proto` | `HostToMachine`, `MachineToHost` | **Same crate** (`proto_peer`). *(Optional: move `PeerLink` RPC to `peer.proto` so `daemon.proto` depends only on `proto_core`, keeping `live_core` free of `proto_peer`.)* |
| `events.proto` (`proto_events`) | 9 domain protos (`boards`, `channels`, `desk`, `files`, `hosts`, `inbox`, `review`, `sessions`, `settings`) | Domain entity payloads | **Keep** `proto_events` at top of DAG. **Move 4b (wire-identical):** Move 4-variant `enum Change` (6 LOC) into `common.proto` (`proto_core`). |

#### B. Rust crate-level `humata2_proto` / `humata2_grpc` boundary changes

Currently 198 files in `daemon/humatad/src` (of 254) and 28 files in `clients/live/src` (of 32) reference `humata2_grpc`. After Phase 4 splits `daemon/humatad` and `clients/live` (and applying the §3.1.3 opaque-event-bus + relay decoupling):

| Consumer crate | Current proto/grpc dep | New narrow proto/grpc dep(s) | Key cross-crate note |
|---|---|---|---|
| `clients/terminal_link` (`src/lib.rs:12`, `tests/link.rs:4`) | `api/proto:humata2_rust` | `api/proto:terminal_rust` | Uses `core` + 10 `terminal.proto` types only. |
| `daemon/review_diff` (`src/wire.rs:11–12`) | `api/proto:humata2_rust` | `api/proto:review_rust` | Uses `core` + 45 `review.proto` types + `Timestamp` (update `wire.rs:12` to `humata2_proto_core::Timestamp`). |
| `clients/live_core` | `daemon/humatad:grpc` | `api/grpc:core` (+ `daemon.proto` for `GetServerInfo`) | Move domain-specific mappers (`channel`, `session`, `order_tabs`, `restart`, `with_operation`, `machine`) out of `live_core/src/map.rs` into `live_daemon` (and `machine_stop` into `live_machines`). |
| `clients/live_machines` | `daemon/humatad:grpc` | `api/grpc:machines`, `api/grpc:peer` (`stop_sessions`/`start_sessions`, `machines.rs:112–113,1260–1261`), `api/grpc:sessions` (`list_channels`/`list_sessions`, `L1018–1019,1055–1056`), `api/grpc:core` | Zero dep on `review`, `board`, `desk`, `files`, `events`. |
| `clients/live_files` | `daemon/humatad:grpc` | `api/grpc:files`, `api/grpc:machines` (`GetStorage`), `api/grpc:core` | Zero dep on `review`, `sessions`, `board`, `desk`, `events`. |
| `clients/live_diff` | `daemon/humatad:grpc` | `api/grpc:review`, `api/grpc:files`, `api/grpc:sessions`, `api/grpc:core` | Zero dep on `machines`, `board`, `desk`, `events`. |
| `clients/live_review` | `daemon/humatad:grpc` | `api/grpc:review`, `api/grpc:events`, `api/grpc:core` | Zero dep on `machines`, `files`, `board`, `desk`, `peer`. |
| `clients/live_desk` | `daemon/humatad:grpc` | `api/grpc:desk`, `api/grpc:review`, `api/grpc:core` | Zero dep on `events` (after Move 4b), `machines`, `sessions`, `peer`, `board`. |
| `clients/live_daemon` | `daemon/humatad:grpc` | `api/grpc:events`, `sessions`, `board`, `inbox`, `catalog`, `terminal`, `machines`, `peer`, `core` | Zero dep on `grpc_review`, `grpc_files`. |
| `daemon/rpc_kit`, `daemon/runtime` | `daemon/humatad:grpc` | `api/grpc:core` (plus moving `DaemonRole`, `Reachability`, `FileChunk`, `ViewState` into `core`, and opaque event frames in `runtime::events` per §3.1.3) | Prevents `rpc_kit`/`runtime` from pulling `events`, `files`, `machines`, `peer`, or `review` into every service. |
| `daemon/svc_review` | `daemon/humatad:grpc` | `api/grpc:review`, `sessions`, `terminal` (`TypeRequest`), `peer` (`review/replay.rs`), `core` | Zero dep on `board`, `desk`, `files`, `machines`, `events` (after §3.1.3). |
| `daemon/svc_board` | `daemon/humatad:grpc` | `api/grpc:board`, `inbox`, `core` (+ replay adapters via `daemon/ports`) | Zero dep on `review`, `desk`, `sessions`, `events`. |
| `daemon/svc_files` | `daemon/humatad:grpc` | `api/grpc:files`, `core` | Zero dep on `review`, `sessions`, `machines`, `board`, `desk`, `events`. |
| `daemon/svc_transfers` | `daemon/humatad:grpc` | `api/grpc:files`, `sessions` (`SessionCarryService`), `inbox` (`CrossMachineCopyPreview`), `core` | Zero dep on `review`, `board`, `desk`, `machines`, `events`. |
| `daemon/svc_inbox`, `svc_away` | `daemon/humatad:grpc` | `api/grpc:inbox`, `peer`, `core` (`svc_away` also routes replay through `daemon/ports` rather than importing `review`) | Zero dep on `board`, `desk`, `files`, `machines`, `events`. |
| `daemon/svc_terminal` | `daemon/humatad:grpc` | `api/grpc:terminal`, `machines` (`RunningHolder`), `sessions` (`ArchiveSummary`), `core` | Zero dep on `review`, `board`, `desk`, `files`, `events`. |
| `daemon/svc_catalog` | `daemon/humatad:grpc` | `api/grpc:catalog`, `files` (`TrashService`), `machines` (`ProviderService`/`SkillsService`), `inbox`, `peer`, `sessions`, `core` | Zero dep on `review`, `board`, `desk`, `events` (or split `svc_repos_skills`, `svc_settings_auth`, `svc_history_marks`). |
| `daemon/svc_machines` | `daemon/humatad:grpc` | `api/grpc:machines`, `peer`, `inbox` (`machine_scripts/live.rs`), `core` | Zero dep on `review`, `board`, `desk`, `files`, `terminal`, `events`. |
| `daemon/svc_desk` | `daemon/humatad:grpc` | `api/grpc:desk`, `review`, `inbox`, `peer`, `sessions`, `core` | Zero dep on `board`, `machines`, `terminal`, `events`. |
| `daemon/svc_channels`, `svc_sessions_*`, `svc_peer` (transport core), `svc_lifecycle` | `daemon/humatad:grpc` | `api/grpc:sessions`, `peer`, `inbox`, `core` (+ `catalog` only for `history_service` client; route `StartReviewRequest`, `ListDeskSessionsRequest`, `LetGoOfHostRequest` through `daemon/ports`) | **Zero dep on `grpc_review`, `grpc_board`, `grpc_desk`, `grpc_machines`, `grpc_files`, `grpc_terminal`, or `grpc_events`!** |
| `daemon/svc_mcp`, `daemon/humatad` (thin binary + relay/event wiring), `cli` | `api/proto:humata2_rust` / `daemon/humatad:grpc` | Umbrella `api/proto:humata2_rust` / `api/grpc:all` | `cli` embeds the full descriptor set via `descriptors.bzl` and speaks all 34 services. |

### 3.4 Test re-pointing (Phase 5)

| Test target(s) | Current proto/grpc dep | New proto/grpc dep(s) | Cache-isolation payoff |
|---|---|---|---|
| `//apps/humata2/clients/terminal_link:link_test` | `api/proto:humata2_rust` | `api/proto:terminal_rust` | Rebuilds/reruns **only** on `core` or `terminal.proto` changes. |
| `//apps/humata2/daemon/review_diff:review_diff_unit_test` + 13 `tests/*.rs` targets | `api/proto:humata2_rust` | `api/proto:review_rust` | Rebuilds/reruns **only** on `core` or `review.proto` changes. |
| `//apps/humata2/daemon/svc_*:*_test` (Tier A in-process) | `:grpc` | Matching narrow `api/grpc:<domain>` + `api/grpc:core` | Each service's tests only rerun when its own `.proto` or `core` changes. |
| `//apps/humata2/daemon/humatad:humatad2_test_power_cut` (`power_cut_*`, `stop_box`, `upgrade_*`, `move_session`, `rebuild_session`, `fork_remote`, `remote_archive`) | `:grpc` | `api/grpc:sessions`, `api/grpc:peer`, `api/grpc:inbox`, `api/grpc:core` (after §3.1.3 opaque event bus + relay decoupling) | **Adding fields to `review.proto`, `boards.proto`, `desk.proto`, `hosts.proto`, `files.proto`, `terminal.proto`, `repos.proto`, `history.proto`, `settings.proto`, or `events.proto produces a 100% Bazel cache hit on all power-cut / move / upgrade suites.** |
| `//apps/humata2/daemon/humatad:peer_link_latency_bench` | `:grpc` | `api/grpc:peer`, `api/grpc:core` | Only depends on `peer`/`daemon` protos. |
| `//apps/humata2/clients/live_machines:machines_test` | `daemon/humatad:grpc` | `api/grpc:machines`, `api/grpc:peer`, `api/grpc:sessions`, `api/grpc:core` | Immune to `review`, `boards`, `desk`, `files`, `events` proto edits. |
| `//apps/humata2/cli:cli_unit_test`, 30 `cli/tests/*.rs` targets (`29` glob + `hosts_live_test`) | `api/proto:humata2_rust` (+ `:grpc` for `hosts_live_test`) | Umbrella `api/proto:humata2_rust` (`hosts_live_test` can narrow to `machines` + `peer` + `core`) | Full CLI JSON descriptor coverage preserved. |
| `//apps/humata2/desktop:live_flows_cases` & `live_flows_ssh_test` | `daemon/humatad:grpc` | Umbrella `api/grpc:all` | Full-stack E2E smoke stays on umbrella facade. |

### 3.5 Order of moves (Phase 5)

1. **Wire-identical `.proto` type relocations (still under single `humata2_proto`):**
   - Move `DiffSpec` (+ 8 transitive types) from `review.proto` -> `common.proto`; drop `import "humata/v1/review.proto"` from `sessions.proto`.
   - Move `SessionDisposition`, `SessionWork`, `ArchiveChoice` from `channels.proto` -> `common.proto`; drop `import "humata/v1/channels.proto"` from `hosts.proto`.
   - Move `BoardChangePreview`, `FieldChange` from `inbox.proto` -> `common.proto`; drop `import "humata/v1/inbox.proto"` from `boards.proto`.
   - Move all **9** `FileLocation` messages (`files.proto:56–106`) + `enum Change` (`events.proto`) -> `common.proto`; drop `import "humata/v1/files.proto"` from `desk.proto`.
   - *(Optional small shared-type cleanup: move `DaemonRole`, `Reachability`, `FileChunk`, `ViewState` into `common.proto` so `rpc_kit`/`runtime` depend only on `core`.)*
   - Zero Rust changes needed; entire repo builds identically.
2. **Add same-package prost + tonic Bazel rules and `proto_core` / `grpc_core`:**
   - Widen `//apps/humata/proto/plugin:protoc-gen-prost` visibility; add `apps/humata2/api/proto/prost.bzl` (with `--prost_opt=extern_path=.google.protobuf=::prost_types`).
   - Relocate `tonic.bzl` and `protoc-gen-tonic` to `apps/humata2/api/grpc/` (keeping an alias in `daemon/humatad`).
   - Define `core_proto`, `core_rust`, and `api/grpc:core`; update `cli/descriptors.bzl` to accept `protos = [...]`.
3. **Define the 11 domain `proto_library` + `*_rust` + `api/grpc:<domain>` targets in DAG order** (leaves first, then `desk`, `peer`, `events`), converting `api/proto:humata2_rust` and `daemon/humatad:grpc` into thin re-export facades.
4. **Re-point pure prost consumers first:** `clients/terminal_link` -> `terminal_rust`; `daemon/review_diff` -> `review_rust` (updating `wire.rs:11–12`).
5. **Re-point `clients/live_*` crates:** Move domain-specific mappers out of `live_core/src/map.rs` into `live_daemon`/`live_machines`, then switch each `clients/live_<domain>` crate to its narrow `api/grpc:<domain>` deps (§3.3B).
6. **Decouple `runtime::events` and `svc_peer` relay wiring (§3.1.3), then re-point daemon `svc_*` crates and `humatad2_test_power_cut`:** Switch leaf services one PR at a time, then orchestration services + `humatad2_test_power_cut` to `api/grpc:{sessions,peer,inbox,core}`.
7. **Lock umbrella visibility:** Restrict `api/proto:humata2_rust` and `api/grpc:all` strictly to `cli`, `daemon/humatad`, `svc_mcp`, `remote_rig`, and `desktop:live_flows_cases`.
