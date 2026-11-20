# FUSEFSWIRE: finish 0.6.0 FUSEFS (T11–T12, humatad2 and Settings wiring)

- **Status:** open, paused for 0.9.0; release target 0.10.0 (owner, 2026-11-20). Follows [FUSEFS](FUSEFS.md) T1–T10 (12305aba, 127b766c).
- **Release:** 0.6.0 (RESUME.md › Release plan, step 17). Full suite + launch check.
- **Outcome sentence:** FUSEFS is actually on: humatad2 runs the mount as a fifth helper, sessions get native per-session config homes with a zero-restart sync switch, moves carry chunks through the shared chunk store, and Settings › Sync controls it all.

## Review notes before starting (Claude review, 2026-11-20)

The brief below says T1–T10 are "committed and GREEN". Their tests pass, but the 12305aba review found these, which this milestone fixes first:

- Nothing is wired into humatad2 yet; nothing mounts. Lane A is that wiring.
- `humata2_fusefs materialize` used to print "Materialized" without copying anything. It now exits with code 3 and "not implemented yet" (uncommitted fix, 2026-11-20). Make it real here, or keep it failing.
- `fusefs_host` supervisor (supervisor.rs:106) reports humatad's own pid as the mount's pid.
- The default control socket is `/tmp/humata2-fusefs.sock`, which is world-writable. Use `<state>/fusefs.sock` with mode 0600, as Lane A says.
- `cow.rs` fsync can block on the network for up to 5 s. Keep it off any session's hot path.
- `VfsError` variants carry Strings with no source. Give them typed fields and sources (AGENTS.md › Errors).
- `//libs/fusefs/*` targets use `//visibility:public`. Narrow them to the packages that use them.
- rewrite.rs:1497/1531: the rewrite-script timeout kills only `bash`, not its children. Use `process_group(0)` + `killpg`.
- The HOOKCLEAR regressions in the same commit go to [SCREENPARSE](SCREENPARSE.md), not here.

## Brief (owner, 2026-11-20, as given)

Work in `/Users/avestabarzegar/oss/atmosphere-khodr` on branch `atmosphere-khodr`.

`0.6.0 FUSEFS` foundations (`T1`–`T10`) are committed:
- `//libs/fusefs/{vfs,cas,host,fskit,linux}` (zero `humata2_*` deps)
- `//apps/humata2/daemon/fusefs_adapter` (`fusefs_adapter_test` + `:humata2_fusefs` binary)
- `//apps/humata2/daemon/config_compose` (`config_compose_test`, all 4 agents `claude`/`codex`/`copilot`/`agy`, `[sync]` toggles, instant rage-quit, `SecretDenylist`, 3-stage rewrite pipeline)

The job: complete `0.6.0 FUSEFS` (`T11`–`T12` + `humatad2` / Settings wiring), verify it end to end with Bazel, and commit cleanly.

### Hard constraints
- **Bazel only**, through `tools/bq/bq` and `apps/humata2/tools/tests/presubmit.sh`. Never run `cargo`, `bazel clean` or `bazel shutdown`.
- **Boundary invariant:** `//libs/fusefs/*` keeps zero `//apps/humata2/...` deps.
  - Local session folders, git clones, `shared/` and per-session `config/<agent>/` stay on native APFS/ext4, so local work has zero FUSE overhead.
  - FUSE is mounted only at `$HUMATA_FUSE_ROOT`, and only for cross-machine read-through, `channels/`, `requests/` and `control/`.
- **Strict 4-agent scope:** `claude`, `codex`, `copilot`, `agy`. Never modify the real `~/.claude`, `~/.codex`, `~/.copilot`, `~/.gemini/antigravity-cli` or `~/.gemini/config`.
- **Never** name outside projects in code, comments, docs or commits.
- **Headless only:**
  - never open the app or GUI windows;
  - never kill running terminals or holders;
  - never replace launchd `app.humata2.humatad2`.
- **Git hygiene:**
  - Stage only with explicit `git add -- <paths>`.
  - Never commit the owner's foreign files: `apps/humata/proto/plugin/BUILD.bazel`, `apps/humata2/desktop/tests/__snapshots__/native/away-left-out-before.png`, `.claude/`, `.signals/`, `tools/skills/dag-walk-gemini/`.

### Step 0: lock `apps/humata2/api/proto/humata/v1/settings.proto`
The highest tag on `Settings` and `UpdateSettingsRequest` is `13` (`todo_automation`). Add:
- `enum SyncSkillsMode { SYNC_SKILLS_MODE_UNSPECIFIED = 0; SYNC_SKILLS_MODE_HOST_WINS = 1; SYNC_SKILLS_MODE_HOST_ONLY = 2; }`
- `message MachineSyncOverride { optional bool enabled = 1; optional bool skills = 2; optional bool instructions = 3; optional bool mcp = 4; optional bool settings = 5; optional SyncSkillsMode skills_mode = 6; }`
- `message SyncSettings { bool sync_enabled = 1; bool skills = 2; bool instructions = 3; bool mcp = 4; bool settings = 5; SyncSkillsMode skills_mode = 6; map<string, MachineSyncOverride> machines = 7; }`
- On `Settings`: `SyncSettings sync = 14;`, `bool fusefs_enabled = 15;`
- On `UpdateSettingsRequest`: `SyncSettings sync = 14;`, `optional bool fusefs_enabled = 15;`

Then split the work into 3 lanes with no shared files.

### Lane A: helper packaging, `humatad2` supervision, `config_compose` wiring
1. **Package `humata2_fusefs` as the 5th helper binary**, so a `humatad2` exec swap never tears down mounts:
   - `daemon/humatad/tools.bzl` and `BUILD.bazel`:
     - add `"fusefs": "humata2_fusefs"` to `_FILES`;
     - add `fusefs = "//apps/humata2/daemon/fusefs_adapter:humata2_fusefs"` on `:tools`, `:tools_mac` and `:tools_linux`;
     - add `config_compose` and `fusefs_adapter` to `SHARED_DEPS`.
   - `app/app.bzl`: put `humata2_fusefs` in `Contents/Helpers/`.
   - `desktop/install/src/lib.rs` (+ its tests) and `desktop/src/live.rs`: stage `humata2_fusefs` into `~/.humata2/bin/`.
   - `daemon/machines/src/{bundle.rs,install.rs}` (+ `tests/{fake_ssh.rs,stage_next.rs}`): add `InstalledFile::FuseFs => "humata2_fusefs"` and bump `ALL` to 5.
   - `tools/release/{check.sh,release.sh}`: sign and verify `humata2_fusefs`.
2. **`humatad2` supervision** (`daemon/humatad/src/{lib.rs,serve.rs,error.rs,fusefs.rs (new)}`):
   - Add `--fusefs <PATH>`, defaulting to `beside_this_binary("humata2_fusefs")`.
   - Add `--fusefs-mount <PATH>`, defaulting to `<state>/mnt`.
   - `FuseFsSupervisor` spawns `humata2_fusefs serve --socket <state>/fusefs.sock --mountpoint <mnt> --mode <fskit|linux|degraded>` in its own session and process group.
   - It falls back cleanly to `degraded` when FSKit or `/dev/fuse` is unavailable.
   - It reconnects through `fusefs_host::ControlClient` across `humatad2` restarts without unmounting.
   - It pushes `HumataTopology` / `DaemonLinkState` updates.
3. **`ConfigComposer` lifecycle, session env and zero-restart rage-quit.** Files: `settings_service.rs`, `settings_service/sections.rs`, `contained.rs`, `session_service.rs`, `wire_stop_restart.rs`, `wire_machines.rs`, `svc_stop_restart/src/{ports.rs,restart.rs}`.
   - **Persist** `<home>/config/config-sync.toml` through `humata2_config_compose::SyncConfig`, and `fusefs_enabled` in `config/settings.toml`.
   - **`update_settings`:**
     - save atomically and publish `pb::event::Payload::Settings`;
     - when `sync.sync_enabled` flips, call `ConfigComposer::disable_sync` / `enable_sync` / `materialize_session` on every active local session;
     - do it in place, with zero holder restarts (`holder_restart_required == false`).
   - **Session env:** extend `TabEnvPort` with `session_compose_env` (`svc_stop_restart/src/ports.rs`, implemented in `wire_stop_restart.rs`). For local `TabKind::Agent` tabs (`Claude`/`Codex`/`Copilot`/`Agy`) it:
     - runs `ConfigComposer::materialize_session` with `ProtectedHumataMcp` from `session_service::hooks::install_mcp_configs`;
     - injects `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `COPILOT_HOME` / `GEMINI_CLI_HOME`, pointing at the native `<machine-root>/.sessions/<id>/config/<agent>/`;
     - injects `HUMATA_FUSE_ROOT` and `HUMATA_MACHINE_NAME`.
   - **`contained.rs`:**
     - retire `seed_claude_copy` and `flow_back` for composed sessions;
     - point the sandbox's writable `agent_state` at the native `<machine-root>/.sessions/<id>/config/<agent>/`;
     - mount the real auth targets read-only behind `auth_passthrough_links`, for all 4 agents;
     - make sure `sessions_core::Agents` and `history_service` resolve the composed `cli_home`.

### Lane B (`T11`): `MoveSession` → `fusefs_cas` cutover and MCP byte-tool retirement
1. **`MoveSession` onto `fusefs_cas`.** Files: `sessions_carry/{BUILD.bazel,src/{lib.rs,carry.rs,carry_cheap.rs,carry_chunks.rs}}`, `sessions/src/lib.rs`, `svc_transfers/{BUILD.bazel,src/{carry.rs,transfer_end_service.rs,remote_end.rs}}`.
   - Keep the `carry_cheap.rs` git clone recipes (`clones.json`).
   - Replace the `<channel>/.carry/have` fixed 1 MiB SHA-256 chunks with:
     - the machine-wide `fusefs_cas::ChunkStore` (`<root>/.cas` or `<state>/cas`);
     - FastCDC `chunk_bytes`;
     - BLAKE3 `FileManifest`;
     - `AdaptiveCodecPlanner`.
   - When an empty legacy `<channel>/.carry/have` is found, remove it. Remove the `HAVE_DIR` export.
   - Keep `Chunks::Planned`, `Chunks::Moved` and `fault::hold_blocking("move.copy.chunk_sent")`, so `move_session_test` and `move_cheap_test` stay deterministic.
2. **Retire the 7 byte-oriented MCP file tools.** Do NOT remove the `FileService` gRPC RPCs that the desktop UI and CLI use.
   - Remove `FilesList`, `FileRead`, `FilesFind`, `SessionRead`, `FileWrite`, `FolderCreate` and `FileFetch` from:
     - `daemon/mcp/src/tools.rs`;
     - `daemon/svc_mcp/src/{mcp.rs,mcp/history.rs}`;
     - `daemon/mcp/tests/tools.txt`.
   - Keep `SessionsSearch`, `DocVersionsList` and `DocVersionRead`.
   - Update the affected tests:
     - `daemon/mcp/tests/schemas.rs`: `file_write` → `sessions_delete_many`; `file_read` → `doc_versions_list` / `doc_version_read`;
     - `daemon/humatad/tests/matrix.rs:244`;
     - `daemon/humatad/tests/mcp_desk_folder.rs:118-146`;
     - `daemon/humatad/tests/history_tools.rs:547-815`.

### Lane C: clients + Settings feature and GPUI UI (`Settings › Sync`)
- **Clients:** in `clients/src/daemon.rs` + `daemon/sync.rs` (new) + `daemon/fake.rs` + `daemon/fake/sync.rs` (new), add:
  - the types `SyncSkillsMode`, `SyncSettings`, `MachineSyncSettings` and `SyncChange`;
  - `Daemon::sync_settings` and `Daemon::set_sync_settings`.
- **Live client:** `clients/live_daemon/src/{lib.rs,daemon.rs,settings.rs}` calls the gRPC `GetSettings` / `UpdateSettings`.
- **Settings feature:** in `features/settings/src/{lib.rs,sync.rs (new)}`, `features/settings/tests/sync.rs` and `features/app/tests/every_action.rs`, add `Section::Sync` and `Row::{SyncEnabled, FuseFsEnabled}`. The toggles update optimistically and roll back on error.
- **UI:** `ui/settings/src/{lib.rs,settings.rs,sync.rs (new)}` renders `Settings › Sync` with:
  - a master rage-quit switch (`settings-sync-master`) whose copy says it is zero-restart and never touches the real dotfiles;
  - category toggles for `skills`, `instructions`, `mcp` and `settings`, plus a `SkillsMode` segmented control;
  - per-machine overrides;
  - the FUSEFS mount toggle (`settings-fusefs-enabled`).
- **Frames:** Settings › Sync needs a signed-off frame first. Fable draws it; the native scene copies it exactly.

### Verification gate (`T12`)
1. Narrow tests per lane, through `tools/bq/bq`:
   - **FUSEFS crates:** `//libs/fusefs/...`, `//apps/humata2/daemon/fusefs_adapter:fusefs_adapter_test`, `//apps/humata2/daemon/config_compose:config_compose_test`
   - **humatad:** `//apps/humata2/daemon/humatad:{humatad2_unit_test,mcp_test,mcp_desk_folder_test,history_tools_test,matrix_test}`
   - **Moves and MCP:** `//apps/humata2/daemon/sessions_carry:sessions_carry_unit_test`, `//apps/humata2/daemon/svc_transfers:svc_transfers_unit_test`, `//apps/humata2/daemon/mcp:all`, `//apps/humata2/daemon/svc_mcp:svc_mcp_unit_test`
   - **Packaging:** `//apps/humata2/daemon/machines:all`, `//apps/humata2/desktop/install:all`
   - **Clients and UI:** `//apps/humata2/clients:clients_unit_test`, `//apps/humata2/clients/live_daemon:all`, `//apps/humata2/features/settings:all`, `//apps/humata2/features/app:every_action_test`, `//apps/humata2/ui/settings:settings_unit_test`
   - **Slow moves:** `apps/humata2/tools/tests/presubmit.sh --with-slow //apps/humata2/daemon/humatad:move_cheap_test //apps/humata2/daemon/humatad:move_session_test`
2. Run the full gate: `apps/humata2/tools/tests/presubmit.sh --full` (strict `--config=lint`) and `tools/bq/bq --build //apps/humata2/app:app`.
3. Verify each of these:
   - local sessions stay on native disk;
   - `SecretDenylist` returns `EACCES`;
   - the rage-quit switch flips without restarting holders;
   - `move_cheap_test` reuses FastCDC BLAKE3 chunks and creates no `.carry/have`.
4. Commit with explicit `git add -- <paths>`. No AI attribution trailers.

## Lane

Manager: minion FUSEFSWIRE (Opus 5.5), started 2026-11-20. Workers never run bazel or git; main builds.

**State: PAUSED 2026-11-20 (main: full-suite 0.9.0 release first; this lane now targets 0.10.0).** No worker was spawned. Every task row marked "running" below is planned and not started: nothing was written for A1–C2b, R1 or R2. On resume, spawn the workers in the table as written. Uncommitted files from this lane: only this milestone file. Step 0 is committed (d9579b71).

### Exit
The T12 gate in this file passes (narrow targets, `presubmit.sh --full`, `:app` build) and the four checks in T12.3 hold.

### Design notes
- **Step 0 (done 2026-11-20):** `settings.proto` gains `SyncSkillsMode`, `MachineSyncOverride`, `SyncSettings`, `Settings.sync = 14`, `Settings.fusefs_enabled = 15`, `UpdateSettingsRequest.sync = 14`, `optional fusefs_enabled = 15`. Additive; every Rust constructor already uses `..Default::default()`. Released to BROWSER once main builds it.
- **Safe defaults (D-entry to log, Provisional):** `fusefs_enabled` is **off** by default. With it off humatad2 spawns no helper and nothing mounts, so the installed daemon and tests never touch `/sbin/mount`. Sync (`config-sync.toml`) follows `SyncConfig::default()` (on; skills + instructions).
- **Composed homes only for new launches:** a session gets a composed `config/<agent>/` home when its agent tab is launched (or relaunched) after this ships. The env it got is recorded with the tab so resume and history look in the same home. Running holders are never restarted; the rage-quit switch re-materializes in place.
- **Helper:** `humata2_fusefs` is the fifth helper, beside `humatad2`, in its own session and process group, so an exec swap of `humatad2` keeps the mount. `FuseFsSupervisor` (new `daemon/humatad/src/fusefs.rs`) finds a live helper through `fusefs_host::ControlClient` on `<state>/fusefs.sock` (0600) before spawning one.
- **Topology push:** `libs/fusefs/host` gets a generic `ControlRequest::Extension { body }` forwarded to an optional handler, so the lib keeps zero Humata types; `humata2_fusefs serve` decodes `HumataTopology` / `DaemonLinkState` from it.
- **MCP byte tools (Lane B2):** main handed us `daemon/svc_mcp/src/{mcp.rs,mcp/history.rs}` (BROWSER adds only a registration hunk in mcp.rs). `daemon/mcp/src/tools.rs` is a main seam: our hunks are listed for main.

### Tasks
| ID | Task | Files | Worker | Status |
|---|---|---|---|---|
| S0 | settings.proto fields | `api/proto/humata/v1/settings.proto` | manager | done, reported to main |
| A1 | Package `humata2_fusefs` as 5th helper | `daemon/humatad/tools.bzl`, `daemon/humatad/BUILD.bazel` (tools targets + SHARED_DEPS hunks), `app/app.bzl`, `desktop/install/src/lib.rs` (+tests), `desktop/src/live.rs`, `daemon/machines/src/{bundle.rs,install.rs}`, `daemon/machines/tests/{fake_ssh.rs,stage_next.rs}`, `tools/release/{check.sh,release.sh}` | haiku A1 | running |
| A2 | `FuseFsSupervisor`, socket 0600, real mount pid, topology push | new `daemon/humatad/src/fusefs.rs`, `libs/fusefs/host/src/{supervisor.rs,control.rs}`, `daemon/fusefs_adapter/src/{main.rs,layout.rs}`; own hunks in `daemon/humatad/src/{lib.rs,serve.rs,error.rs}` (BROWSER's files; main allowed own hunks, listed in report) | sonnet A2 | running |
| A3 | Sync settings persisted, rage-quit in place, composed session env, contained homes | `daemon/humatad/src/settings_service.rs`, `settings_service/sections.rs`, new `daemon/humatad/src/config_sync.rs`, `contained.rs`, `session_service.rs`, `wire_stop_restart.rs`, `wire_machines.rs`, `daemon/svc_stop_restart/src/{ports.rs,restart.rs}`, new `daemon/humatad/tests/settings_sync.rs` | sonnet A3 | running |
| B1 | MoveSession onto `fusefs_cas` | `daemon/sessions_carry/{BUILD.bazel,src/{lib.rs,carry.rs,carry_cheap.rs,carry_chunks.rs}}`, `daemon/sessions/src/lib.rs`, `daemon/svc_transfers/{BUILD.bazel,src/{carry.rs,transfer_end_service.rs,remote_end.rs}}` | sonnet B1 | running |
| B2 | Retire 7 MCP byte tools | `daemon/mcp/src/tools.rs`, `daemon/svc_mcp/src/{mcp.rs,mcp/history.rs}`, `daemon/mcp/tests/{tools.txt,schemas.rs}`, `daemon/humatad/tests/{matrix.rs,mcp_desk_folder.rs,history_tools.rs}` | sonnet B2 | running (main 2026-11-20: svc_mcp files are ours; tools.rs hunks listed for main) |
| C1 | Clients + live client + Settings feature (Sync section, rows, optimistic + rollback) | `clients/src/daemon.rs` (own hunks), new `clients/src/daemon/sync.rs`, `clients/src/daemon/fake.rs` (own hunks), new `clients/src/daemon/fake/sync.rs`, `clients/live_daemon/src/{lib.rs,daemon.rs,settings.rs}`, `features/settings/src/lib.rs` (own hunks), new `features/settings/src/sync.rs`, new `features/settings/tests/sync.rs`, `features/settings/BUILD.bazel`, `features/app/tests/every_action.rs` | sonnet C1 | running |
| C2a | Settings › Sync design frame (candidate), Fable judges | new `desktop/tests/design/settings_sync.rs`, `desktop/tests/design.rs` (CANDIDATES hunk) | fable F1 | running |
| C2b | Native Settings › Sync page copying the signed-off frame | `ui/settings/src/{lib.rs,settings.rs}` (own hunks), new `ui/settings/src/sync.rs` | — | waits on C1 + frame sign-off |
| R1 | Review notes: narrow `//libs/fusefs/*` visibility, rewrite-script timeout kills the process group, cow fsync off hot paths | `libs/fusefs/*/BUILD.bazel`, `daemon/config_compose/src/rewrite.rs`, `libs/fusefs/cas/src/cow.rs` | haiku R1 | running |
| R2 | Review note: `VfsError` typed fields + sources | `libs/fusefs/**`, `daemon/fusefs_adapter/src/**` | — | after A2 |

### Claims
As in the task table above, plus `libs/fusefs/**` (review notes) and `milestones/FUSEFSWIRE.md`. Shared-file hunks are listed per batch report.

### Log
- 2026-11-20: Step 0 written and reported to main. Asked main for hand-over of `daemon/humatad/src/{lib.rs,serve.rs,error.rs}` (BROWSER) for A2 wiring and `daemon/mcp/src/tools.rs` + `daemon/svc_mcp/**` for B2.
- 2026-11-20: main committed Step 0 (d9579b71). Rulings: own hunks allowed in humatad `lib.rs`/`serve.rs`/`error.rs`; svc_mcp `mcp.rs`/`mcp/history.rs` ours for B2.
- 2026-11-20: PAUSED at main's request before any worker started. On resume: spawn A1, A2, A3, B1, B2, C1, R1 and F1 in parallel. A3 doesn't edit `daemon/humatad/BUILD.bazel`; it reports its test stanza instead, so it stays clear of A1. A1 also adds `//libs/fusefs/host` to SHARED_DEPS for A2.
