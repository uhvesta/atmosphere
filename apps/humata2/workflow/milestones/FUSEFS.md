# FUSEFS: one filesystem-shaped view of every machine

- **Status:** planning (spec and spike only)
- **Depends on:** `MASSDELETE` (protected roots use one planned session delete);
  `LIVEHOLDER` (native work, config toggles and holder state survive rename and
  upgrade); `AGENTS2` (Claude, Codex and Copilot structured hooks/config
  adapters are complete); `TODO2` (scheduled before `FUSEFS` so To-do/PR/session
  lifecycle sync is settled prior to filesystem cutover). `MACH`, `AWAY`, `NOPATH`, `UPGRADE`, `MACHINESCRIPTS`
  and the peer transfer journal already provide machine identity, `PeerUplink`
  away state, roots, script safety rules and resumable transport.
- **Owner outcome, 2026-11-19:** "A cross-platform filesystem that gives
  every agent and the owner one unified view of all Humata state, across all
  machines," built surgically as standalone generic filesystem crates consumed
  via public APIs, with zero host-sync blocking on remote sessions and
  Settings-configurable host-canonical non-auth config sync (skills, MCP,
  instructions, settings, path/MCP rewrites via TOML/`Steel`/`bash`, and an
  instant rage-quit switch).
- **Unblocks:** removal of byte-oriented file MCP tools; removal of per-machine
  agent-config bootstrap; stable-by-id session paths; remote file inspection
  without a transfer command.
- **Layer:** L14 · **Lane:** `//libs/fusefs/*` standalone crates + daemon
  adapter + native config composer + platform packaging

## Outcome

**The owner and every agent experience all Humata sessions as if they were on
one machine—without remote sessions ever blocking on the local Mac.** The
filesystem completes the one-machine behavior the terminal system already gives.

Every machine exposes the same logical tree at `~/.humata/fuse/main/`. The agent
can `ls`, `cat`, `grep`, `rg`, `cp` and inspect any visible session without
naming an SSH host, uploading, downloading or choosing a physical path. Machine
placement remains available under `machines/` for inspection, not required for
ordinary work.

To keep the implementation surgical and cache-isolated, the virtual filesystem,
`BLAKE3` content-addressed chunk engine, sparse copy-on-write overlays, subtree
lease table and OS backends (`FSKit` on macOS, `/dev/fuse` + degraded skeleton
on Linux) live in **completely standalone, domain-agnostic crates under
`//libs/fusefs/*`** with zero Humata dependencies. Humata consumes only their
public Rust traits in a thin `humata2_fusefs` adapter process (supervised
independently of `humatad2`) and `humata2_config_compose`.

Live agents run with native working directories and **native per-session config
homes** (`<machine-root>/.sessions/<id>/config/<agent>/`), wired locally to
*that machine's own auth* while exposing a secret-scrubbed view at
`s/<id>/config/<agent>` in the mount. The Mac host is canonical for shared
non-auth config, controlled by per-category Settings toggles (`skills` and
`instructions` on by default; `mcp` and `settings` opt-in; per-machine overrides
and a master **"rage-quit" kill switch** that instantly reverts to machine-local
`~/.<agent>` config without restarting holders), plus customizable path and MCP
rewrite pipelines in declarative TOML, embedded `Steel` Scheme (`steel-core`),
or target-local `bash`.

## Exit criteria

- **Surgical crate boundary:** `//libs/fusefs/{vfs,cas,fskit,linux,host}` builds
  and passes unit/loopback tests with zero dependencies on `//apps/humata2/...`.
  Humata integrates strictly through `//libs/fusefs/*` public traits/types.
- **One tree:** `s/`, `m/`, `channels/`, `desks/` and `machines/` return a
  coherent snapshot, with stable symlinks by id; channel/desk rename is a
  metadata change and no holder restarts.
- **Native config, local auth & configurable sync:** a fresh Claude Code, Codex
  and Copilot session gets its config-home env var pointing to native
  `<machine-root>/.sessions/<id>/config/<agent>/`, uses **its own machine's
  local login** (including file-backed OAuth refresh on headless Linux and
  Keychain on macOS), and sees synced host skills/instructions/MCP/settings
  according to `[sync]` toggles and configured TOML/`Steel`/`bash` rewrite
  rules. Flipping any facet or the master rage-quit switch OFF atomically
  falls back to that machine's native `~/.claude`, `~/.codex`, and `~/.copilot`
  without mutating those real home directories or restarting holders.
- **Zero secrets leaked:** no credential file (`.credentials.json`, `auth.json`,
  cookies, tokens) or inline MCP secret env/header crosses the peer link, enters
  `SessionSnapshot`, or is readable through `s/<id>/config/` in FUSE
  (`CredentialOutsideMount`).
- **Remote autonomy (`AWAY` parity, no host-sync blocking):** with `PeerUplink`
  severed, throttled, or actively streaming a 1 GiB read to the Mac, a remote
  box launches a fresh session (using its latest cached `ConfigBundle`), edits
  files, reads sibling sessions' `work/` and locally generated `context/` on the
  same box, journals same-box cross-session asks locally (`D225`), and runs
  `git status`/`rg` with **0 host round-trips** and within **1.05×** of
  idle-link latency. When offline, uncached remote lookups fail in `< 1 ms`
  without waiting out a 250 ms revalidation timer.
- **Local writes & MASSDELETE:** the agent edits/deletes ordinary files in its
  own session freely. Unlink/rename of `s/<id>`, a channel, a repository root or
  another protected topology root returns `EPERM` and identifies the lifecycle /
  `MASSDELETE` RPC.
- **POSIX-safe approved writes:** another-session or cross-machine write creates
  one Inbox request and returns `EACCES` with
  `~/.humata/fuse/main/requests/<id>.json` in `< 50 ms`. Once approved, a
  turn-bounded (up to 10-minute) target-directory subtree lease allows multi-step
  POSIX operations (`sed -i` random temp-file + `fsync` + `rename`, `cp -r`) to
  succeed under that single approval, checking per-file `expected_generation`
  (`ESTALE` on concurrent edit). Remote `fsync` uses a 1,000 ms *progress-stall*
  bound (returning `0` on active drain completion, `ENETUNREACH`/`EIO` on link
  loss, never `EAGAIN`).
- **Lazy `BLAKE3` data plane & cache stubs:** opening a 20 GiB file reads no
  content. Reads fetch coalesced 256 KiB `BLAKE3`-verified chunks ($\le 1\text{ MiB}$
  batch) with adaptive `lz4`/`zstd`/`none` codecs (dropping to `lz4`/`none` when
  source CPU is busy). Remote `BuildCache` subtrees expose a synthetic
  `.humata-omitted.json` stub so `rg` and `find` exit `0` with zero cache bytes
  transferred.
- **Push invalidation:** remote file changes invalidate via bounded peer
  generation push with zero steady-state polling.
- **Liveness & `MoveSession`:** killing `humatad2` or `humata2_fusefs` never
  terminates a running agent or corrupts its native config/SQLite WAL files.
  `MoveSession` preserves `carry_cheap.rs`'s Git `CloneRecipe` bundle shortcut
  while moving bundle and dirty-file bytes over `fusefs_cas` and retiring
  per-channel `<channel>/.carry/have`.
- **Platforms:** macOS 26.7+ mounts via signed/notarized `fusefs_fskit` with
  `MNT_DONTBROWSE` and `$HUMATA_FUSE_ROOT` capability roots (emitted consistently
  by `sessions_search` / `topology_get`). Enrolled Linux worker boxes mount via
  `fusefs_linux` (`/dev/fuse`), falling back cleanly to the native local symlink
  skeleton in unprivileged containers without `/dev/fuse`.

## Pointers

- Design: `apps/humata2/FUSEFS_SPEC.md`.
- Decisions: D63, D217, D225, D313, D336, D485, D543/D544, D680–D688,
  D755/D756/D770/D777, D801, D876 and D881–D886, D890.
- Standalone pattern precedents: `libs/tca-rust/`, `apps/humata2/terminal/`,
  `apps/humata2/chunk/`.
- Agent state & containment: `daemon/conversation/src/{claude,codex,copilot}.rs`,
  `daemon/contain/src/{claude,home,login,macos}.rs`, `daemon/sessions_carry/`.
- Settings & scripts: `daemon/humatad/src/settings_service/`,
  `daemon/machines/src/machine_scripts.rs`, `daemon/svc_peer/src/peer_skills.rs`.
- Access/MCP: `daemon/access/`, `daemon/mcp/`, `daemon/svc_mcp/`.
- Remote I/O & away state: `daemon/svc_peer/`, `daemon/humatad/src/away/`,
  `daemon/transfers/`, `daemon/svc_sessions_transfer/`.
- Destructive boundaries: `MASSDELETE.md`.

## Out of scope

- A distributed compiler cache or transparent remote build sandbox.
- Syncing credentials or inline MCP secret tokens across machines.
- Blocking remote session launches or turns on host reachability.
- Replacing SSH, peer enrollment, holders or `MoveSession` lifecycle ordering.
- Mounting over the owner's real `~/.claude`, `~/.codex`, or `~/.copilot`.
- Linux desktop UI or Linux primary host (`LINUX` and `ANYHOST` remain cut per
  `D485`; `fusefs_linux` is strictly for headless Linux worker boxes).
- Supporting legacy macOS kext FUSE stacks in production.

## One-day throwaway spike (Task 1)

T1 produces evidence, not product code, and is the first gate.

| Probe | What to do | Record |
|---|---|---|
| Kill mid-turn | Test native config/work + mounted remote views while `kill -9`ing the Rust server and FSKit adapter mid-turn, including dirty remote CoW overlays at `close` and `fsync`. | process state, open fd survival, restart recovery time, journaled CoW state |
| FSKit mount & identity | Mount on macOS 26.7+ with `objc2-fs-kit`: test mounting directly at `~/.humata/fuse/main` vs `/Volumes` + symlink, verify `MNT_DONTBROWSE` hides Finder volume clutter, and inspect XPC `audit_token_t` / request context across two same-uid processes. | mountpoint rules, Finder visibility, whether per-caller PID exists or `cap_<id>` roots are required |
| Auth & OAuth refresh | Launch Claude Code, Codex (`auth.json` + keyring) and Copilot against native per-session config dirs with local auth pass-through on macOS and in the Linux VM. Verify login, OAuth refresh and `Secret` exclusion on the FUSE view. | per-agent local auth plumbing on macOS & headless Linux |
| WAL | Run two-process concurrent SQLite WAL read/insert/checkpoint + `SIGKILL` on native config path (baseline) and through FSKit/fuse-t to gate whether SQLite must remain strictly native-only. | VFS lock behavior, `-wal/-shm`, `integrity_check` |
| Git & local symlink | Time 20 warm/cold `git status --porcelain=v2` and Bazel input scans on native disk, via mounted native symlink, through fuse-t, and through the FSKit proof. | p50/p95/max, syscall count, CPU |
| `BLAKE3` chunk wire | Compare fixed `256 KiB` vs `FastCDC` (`64K`/`256K`/`1M`) with `BLAKE3` + `none`/`lz4`/`zstd` under idle/busy source CPU and fast/constrained link, plus small edit upload bytes. | round trips, wire bytes, hash+codec CPU, boundary-shift dedup, planner shedding under load |
| Events, leases & `fsync` | Test `sed -i` temp-file `rename`, `cp -r`, progress-bounded `fsync` over a throttled link, and watcher/invalidation delivery. | POSIX tool exit codes, delivered/coalesced events, lock/sync behavior |

## Tasks

Tasks after T1 are test-first, build narrow Bazel targets via `tools/bq/bq`, use
explicit error enums (no `Box<dyn Error>` / `anyhow`), and enforce the strict
`//libs/fusefs/*` zero-Humata dependency boundary.

1. **T1 - Run the one-day spike (kill, FSKit mount/identity, local auth, WAL, `BLAKE3`/CDC, POSIX write sequences).**
   Record measurements and finalize Provisional decisions; no production code.
2. **T2 - Create standalone `//libs/fusefs/vfs` (`fusefs_vfs`).**
   Define generic `Volume`, `AccessHook`, `CallerContext`, `NodeId`,
   `VirtualPath` (with `..` capability-root clamping), symlink/stub tree
   builder, subtree `WriteLease` table, and typed `VfsError`. Unit tests prove
   normalization, `..` clamping, synthetic `.omitted.json` stubs, and lease
   expiry/generation checks with zero Humata deps.
3. **T3 - Create standalone `//libs/fusefs/cas` (`fusefs_cas`).**
   Build paged `DirManifest`/`FileManifest`, sharded `BLAKE3` chunk store,
   adaptive `lz4`/`zstd`/`none` planner with source-CPU shedding, LRU budget
   eviction, sparse local CoW range overlays, resumable commit journal, and
   `ChunkTransport` trait. In-process loopback tests cover random/sequential
   reads, codec shedding, CoW partial-chunk reads, progress-bounded `fsync`, and
   eviction priority.
4. **T4 - Create standalone `//libs/fusefs/host` + `//libs/fusefs/fskit` (macOS 26.7+).**
   Build the generic supervisor/control-socket runtime and signed FSKit adapter
   (`objc2-fs-kit` + minimal entry shim if required, `MNT_DONTBROWSE`,
   capability-root routing). Tests cover mount/remount, server kill/restart,
   lookup/read/readdir/write/rename, and progress-bounded `fsync`.
5. **T5 - Create standalone `//libs/fusefs/linux` (worker boxes & container fallback).**
   Add the `/dev/fuse` backend (`fuser` + kernel PID caller binding) and
   explicit degraded native-symlink skeleton writer when `/dev/fuse` is absent.
   Tests run in the Linux VM covering normal mounts, missing `/dev/fuse`
   container fallback, and PID binding.
6. **T6 - Build `//apps/humata2/daemon/fusefs_adapter` (`humata2_fusefs` binary).**
   Consume `//libs/fusefs/*` public APIs to project `s/`, `m/`, `channels/`,
   `desks/`, `machines/`, `requests/`, `control/`, local-authority `context/`,
   and `.humata-omitted.json` `BuildCache` stubs; wire stable-by-id native
   `.sessions/<id>/` placement and daemon reconnect. Tests prove `humatad2` kill
   and same-machine rename survival (`LIVEHOLDER` PID unchanged).
7. **T7 - Wire peer transport, `AWAY` autonomy, and background QoS.**
   Implement `fusefs_cas::ChunkTransport` over the authenticated peer link with
   `PeerUplink` `host_reachable` awareness (`0 ms` away revalidation skip,
   `< 1 ms` offline peer fail, coalescing fire-and-forget invalidation ring,
   lock-free background-QoS source chunk workers). Tests verify zero host
   round-trips and unthrottled local work on a remote box while `PeerUplink` is
   cut or streaming 1 GiB to the host.
8. **T8 - Build `//apps/humata2/daemon/config_compose`: native config, local auth, Settings sync switches, and TOML/`Steel`/`bash` rewrites.**
   Materialize `<machine-root>/.sessions/<id>/config/<agent>/` natively with
   machine-local auth pass-through and sanitized `s/<id>/config/<agent>` FUSE
   views (`Secret` denylisted). Add `[sync]` Settings toggles (`skills` &
   `instructions` default on, `mcp` & `settings` opt-in, per-machine overrides,
   instant non-destructive rage-quit switch) and the 3-stage rewrite pipeline
   (declarative TOML path/MCP rules, capability-free `steel-core` Scheme AST
   transforms, and target-local `~/.humata2/scripts/*.sh` `bash` filters with
   post-transform schema & protected-`humata`-MCP enforcement).
9. **T9 - Enforce caller capabilities, subtree write leases, and `AWAY` Inbox journaling.**
   Wire Linux PID groups, macOS `$HUMATA_FUSE_ROOT` `cap_<id>` roots (including
   consistent `$HUMATA_FUSE_ROOT` emission in `sessions_search` /
   `topology_get`), `< 50 ms` `EACCES` + `requests/<id>.json`, turn-bounded
   (up to 10m) target-directory `WriteLease`s (revoked on `AGENTS2` turn-end
   hook), per-file `expected_generation` `ESTALE` checks, and local `away/journal.rs`
   recording when `host_reachable == false`. Tests prove `sed -i` atomic
   temp-rename and `cp -r` succeed under one approval and concurrent edits fail
   `ESTALE`.
10. **T10 - Wire destructive safety (`MASSDELETE`) and explicit cache materialization.**
    Enforce `EPERM` on session, repo, config, channel, desk, machine and alias
    roots (`rm -rf` / `mv` cannot bypass `PlanDeleteSessions` or `MoveSession`),
    and add `humata2 fs status` / `humata2 fs materialize` with byte-count
    confirmation.
11. **T11 - Unify `MoveSession`, copies, and MCP/bootstrap retirement.**
    Switch `MoveSession` (`SessionSnapshot` preserving `carry_cheap.rs` Git
    `CloneRecipe` bundles + missing `BLAKE3` chunks, retiring
    `<channel>/.carry/have`), owner copy/upload/download, and checkout placement
    onto `fusefs_cas`. Retire same-machine transcript path rewrites,
    `peer_skills.rs` direct home mutation, `seed_claude_copy`, and byte-oriented
    MCP file tools (`files_list`, `file_read`, `files_find`, `file_write`,
    `folder_create`, `file_fetch`, `session_read`).
12. **T12 - Run the kill, `AWAY` autonomy, rewrite-safety, and performance gate.**
    Verify `humata2_fusefs` kill, `humatad2` restart, `AWAY` severed/throttled
    uplink autonomy, malformed `Steel`/`bash` transform recovery, stale subtree
    leases, cache eviction, and `FUSEFS_SPEC.md` performance targets on narrow
    Bazel targets before the full tier.

## Milestone order

```text
T1
 |
 +--> T2 --> T3 --> T4 (macOS FSKit)
 |            \
 |             +--> T5 (Linux /dev/fuse + container skeleton)
 |
 +--> T8 (native config_compose + sync toggles + TOML/Steel/bash)
 |
 (T2,T3,T4,T5) --> T6 --> T7 --> T9 --> T10 --> T11 --> T12
                            \                  /
                             +--- (T8) -------+
```

## Acceptance notes

- `//libs/fusefs/*` stays strictly generic: zero `humata2_*` imports, proto
  types, or agent-specific schemas inside `libs/fusefs/`.
- T1 is disposable measurement code; product code is Bazel-clean Rust (`tools/bq/bq`).
- Remote sessions never wait on host config sync, topology revalidation when
  `AWAY`, or outbound invalidation/mirror delivery.
- `fsync` never returns `EAGAIN`; active progress waits to completion (`0`) and
  stalled/broken links return `ENETUNREACH`/`EIO` with journaled CoW state.
- Flipping config sync OFF ("rage-quit") is non-destructive, leaves real
  `~/.<agent>` untouched, and restarts no terminal holder.
