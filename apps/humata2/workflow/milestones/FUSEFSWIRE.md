# FUSEFSWIRE: finish 0.6.0 FUSEFS (T11–T12, humatad2 and Settings wiring)

- **Status:** open (owner, 2026-11-20). Follows [FUSEFS](FUSEFS.md) T1–T10 (12305aba, 127b766c).
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
(empty: the manager fills this in)
