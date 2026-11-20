# FUSEFSWIRE: finish 0.6.0 FUSEFS (T11–T12, humatad2 and Settings wiring)

- **Status:** open; release target 0.10.0 (owner, 2026-11-20). Follows [FUSEFS](FUSEFS.md) T1–T10 (12305aba, 127b766c).
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

## Owner addition: fall back to the files tools (2026-11-20; part of exit)
- **Fallback (owner, 2026-11-20):** "if FUSE does not work we should fallback to the GRPC/MCP things."
  - **Clarified (owner, 2026-11-20):** "if it fails to even setup FUSE on the remote machines it should do this … Not if fuse fails one operation but if fuse doesn't even work to begin with."
    - The trigger is the setup: FUSE can't be mounted on that machine at all. That covers no FSKit or `/dev/fuse`, the helper not starting, and the mount being refused, on any machine and remote machines especially.
    - A single operation failing on a working mount is **not** a trigger. That stays an ordinary error for that operation, with no switch-over.
  - In that case every location stays reachable through the humata files CLI and MCP tools (gRPC). These cover shared, channel, main, other sessions, Desk and trash.
  - The session's agent is told the folder isn't mounted and to use those tools instead.
  - Nothing that works today breaks.
  - Tests: one where the mount can't be set up on a machine, which asserts the files tools still read and write each location; and one asserting that the agent is told.

- **Gap (manager, 2026-11-20):**
  - Degraded mode today: the supervisor starts the helper but does not mount, and a mount failure is only logged. The `humata files` CLI is unchanged and works without the mount.
  - Three things are wrong today:
    - B2 retired the MCP byte tools (`file_read`, `file_write`, `file_fetch`, `files_list`, `files_find`, `folder_create`, `session_read`). Without the mount, an agent's MCP route to the locations is gone.
    - `HUMATA_FUSE_ROOT` is set at every launch, even when nothing is mounted.
    - The skill and the MCP instructions point at the mount and say nothing about a missing one.
- **Plan:**
  - **FB1 (sonnet):** undo B2. Restore the 7 tools, their schemas, tools.txt and the three humatad tests, and the D1 doc hunks that removed them. Keep them always on, whether mounted or not. The skill says to use the mount when it is there and these tools otherwise. D-entry: the retirement is withdrawn.
  - **FB2 (sonnet):**
    - The trigger is only that the mount can't be set up on a machine: no FSKit or /dev/fuse (degraded), the helper won't start, or the mount is refused. A single operation failing on a working mount stays an ordinary error. The supervisor records whether setup succeeded, on every machine including remote ones.
    - `HUMATA_FUSE_ROOT` goes in a launch env only while the mount is live.
    - The MCP `initialize` instructions gain a line when the folder isn't mounted: unset, or an empty mountpoint. The line names the files tools and `humata files`.
    - Tests: (a) a humatad test where the mount is on but can't be set up (the helper fails to start or the machine is degraded), and the files tools still read and write shared, channel, main, another session, Desk and trash; (b) an in-process test that an unmounted session is told (env and instructions).

## Lane

Manager: minion FUSEFSWIRE (Opus 5.5), started 2026-11-20. Workers never run bazel or git; main builds.

**State: resumed 2026-11-20 after 0.9.0 (5e86d620); target 0.10.0.** Workers A1, A2, A3, B1, B2, C1, R1 and F1 were spawned on resume. Step 0 is committed (d9579b71). D895 is logged.

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
| A3a | Sync + fusefs_enabled persisted, `ConfigSync` registry, rage-quit in place, `saved_fusefs_enabled`, `with_fusefs_switch` | `daemon/humatad/src/settings_service.rs`, `settings_service/sections.rs`, new `daemon/humatad/src/config_sync.rs`, own hunks in lib.rs/serve.rs, new `daemon/humatad/tests/settings_sync.rs` (BUILD stanza by manager) | sonnet A3 | done (unbuilt); settings files released to TODO2WIRE |
| A3b | Composed session env (`TabEnvPort::session_compose_env`), contained native agent_state, auth read-only, history/Agents resolve composed home | `contained.rs`, `session_service.rs`, `wire_stop_restart.rs`, `wire_machines.rs`, `daemon/svc_stop_restart/src/{ports.rs,restart.rs}`, `config_sync.rs` | sonnet A3b | running |
| B1 | MoveSession onto `fusefs_cas` | `daemon/sessions_carry/{BUILD.bazel,src/{lib.rs,carry.rs,carry_cheap.rs,carry_chunks.rs}}`, `daemon/sessions/src/lib.rs`, `daemon/svc_transfers/{BUILD.bazel,src/{carry.rs,transfer_end_service.rs,remote_end.rs}}` | sonnet B1 | running |
| B2 | Retire 7 MCP byte tools | `daemon/mcp/src/tools.rs`, `daemon/svc_mcp/src/{mcp.rs,mcp/history.rs}`, `daemon/mcp/tests/{tools.txt,schemas.rs}`, `daemon/humatad/tests/{matrix.rs,mcp_desk_folder.rs,history_tools.rs}` | sonnet B2 | done (unbuilt); docs follow-up in D1 |
| C1 | Clients + live client + Settings feature (Sync section, rows, optimistic + rollback) | `clients/src/daemon.rs` (own hunks), new `clients/src/daemon/sync.rs`, `clients/src/daemon/fake.rs` (own hunks), new `clients/src/daemon/fake/sync.rs`, `clients/live_daemon/src/{lib.rs,daemon.rs,settings.rs}`, `features/settings/src/lib.rs` (own hunks), new `features/settings/src/sync.rs`, new `features/settings/tests/sync.rs`, `features/settings/BUILD.bazel`, `features/app/tests/every_action.rs` | sonnet C1 | done (unbuilt) |
| C2a | Settings › Sync design frame (candidate), Fable judges | new `desktop/tests/design/settings_sync.rs`, `desktop/tests/design.rs` (CANDIDATES hunk) | fable F1 | done: Fable signed off with edits (D904); main re-records 211, 29 and 117 after C2b |
| C2b | Native Settings › Sync page copying the signed-off frame | `ui/settings/src/{lib.rs,settings.rs}` (own hunks), new `ui/settings/src/sync.rs` | — | done, unbuilt: popup menus, a native snapshot, 211 in FRAMES, the Sync nav row in frames 29 and 117; waits on main's build and goldens, then a Fable check |
| R1 | Review notes: narrow `//libs/fusefs/*` visibility, rewrite-script timeout kills the process group, cow fsync off hot paths | `libs/fusefs/*/BUILD.bazel`, `daemon/config_compose/src/rewrite.rs`, `libs/fusefs/cas/src/cow.rs` | haiku R1 | done (unbuilt): visibility narrowed, process-group kill + test, cow fsync documented (no hot-path caller) |
| R2 | Review note: `VfsError` typed fields + sources | `libs/fusefs/**`, `daemon/fusefs_adapter/src/**` | haiku R2 | done (unbuilt) |

### Claims
As in the task table above, plus `libs/fusefs/**` (review notes) and `milestones/FUSEFSWIRE.md`. Since 2026-11-20 (ruled by main) it also covers `clients/live_daemon/src/events.rs` and `features/app/src/events.rs`, for the reconnect race (G4). Also `sessions_core/src/start.rs` (ruled by main 2026-11-20), for the composed conversation_file record (A3f). Shared-file hunks are listed per batch report.

### Log
- 2026-11-20: Step 0 written and reported to main. Asked main for hand-over of `daemon/humatad/src/{lib.rs,serve.rs,error.rs}` (BROWSER) for A2 wiring and `daemon/mcp/src/tools.rs` + `daemon/svc_mcp/**` for B2.
- 2026-11-20: main committed Step 0 (d9579b71). Rulings: own hunks allowed in humatad `lib.rs`/`serve.rs`/`error.rs`; svc_mcp `mcp.rs`/`mcp/history.rs` ours for B2.
- 2026-11-20: PAUSED at main's request before any worker started. On resume: spawn A1, A2, A3, B1, B2, C1, R1 and F1 in parallel. A3 doesn't edit `daemon/humatad/BUILD.bazel`; it reports its test stanza instead, so it stays clear of A1. A1 also adds `//libs/fusefs/host` to SHARED_DEPS for A2.
- 2026-11-20: RESUMED (main approved the plan and extended the claims). Logged D895. Spawned the 8 workers. A3 does settings_service.rs + sections.rs first and reports, so main can hand them to TODO2WIRE.
- 2026-11-20: spawned A1 (haiku), A2, A3a, B1, B2, C1 (sonnet), R1 (haiku), F1 (fable). Interface contracts: A3a provides `config_sync::saved_fusefs_enabled(state)` and `SettingsSvc::with_fusefs_switch(Arc<dyn Fn(bool)+Send+Sync>)`; A2 provides `FuseFsSupervisor::mount_root()` used as the fuse root.
- 2026-11-20: F1 done. Candidate `211-settings-sync`, because 205–210 were taken by wide candidates. Spec: one card holds the master switch, with skills (segmented Host wins/Host only + switch), instructions, MCP and settings as indented rows; then a Machines override table with Default/On/Off popups; then a mount card (`settings-fusefs-enabled`, off). Sync is third in the nav, icon FolderTree. Open point: dim the child rows when the master is off (proposed: dim, keep values).
- 2026-11-20: R1 done. Left as R1 wrote it: no group kill after a normal exit. Once the leader is reaped, its pid could be reused by an unrelated group. A child left behind can still hold the pipes open on success (follow-up).
- 2026-11-20: A3a, B2 and C1 are done (unbuilt). settings_service.rs and sections.rs were released to TODO2WIRE through main. Manager hunks: humatad BUILD needs-map `settings_sync`; a placeholder `Section::Sync` arm in ui/settings/src/settings.rs (not in the nav). Batch 1 targets were sent to main. Spawned A3b (sonnet: composed homes at launch) and D1 (haiku: docs that still name the retired MCP tools). Open: serve.rs fuse root `state.join("mnt")` becomes A2's mount_root() plus with_fusefs_switch; svc_mcp tokio-stream dep may be unused.
- 2026-11-20: A2 done (unbuilt). It adds the FuseFsSupervisor in humatad/src/fusefs.rs, control Extension + 0600 socket, mount pid Option, adapter HostUpdate/probe_mount_mode, and --socket/--mountpoint required for serve. Manager hunk: ConfigSync's fuse root is now `fusefs.mount_root()`. Gaps: remote machines show online=false in the topology (the store has no reachability); the helper has no shutdown, so off only unmounts; the degraded mode doesn't mount. D1 docs done. R2 spawned (haiku).
- 2026-11-20: A1 done (unbuilt): fusefs is the fifth helper (tools.bzl, BUILD SHARED_DEPS, app.bzl, machines install/bundle, desktop install, live.rs, release scripts). Spawned A1b (sonnet) for the 4-file fixtures and lists across the machines, humatad, desktop and cli tests, ports/release.rs, the svc_upgrade TOOLS Migrate item and notices `_FOLLOWED`. Batch 3 sent to main.
- 2026-11-20: B1 done (unbuilt). Carry uses fusefs_cas, with the store at `<root>/.cas`, FastCDC chunks, BLAKE3 manifests and frames from AdaptiveCodecPlanner. A missing chunk now fails the move with Missing. Spawned B1b (sonnet) to loosen move_cheap.rs's 1 MiB bounds. Batch 4 sent. Follow-ups: the svc_peer have_chunks still uses the flat meaning; the files.proto HaveChunks comment is stale; the docs still name .carry/have.
- 2026-11-20: B1b done: move_cheap.rs bounds no longer assume fixed 1 MiB chunks; main told move_cheap_test is runnable.
- 2026-11-20: A1b done (fixtures/lists to 5 helpers, FuseFs in both TOOLS → HostTools Migrate, notices). Spawned A1c (sonnet) for the upgrade rigs + FUSEFS test need in humatad BUILD. Batch 5 sent.
- 2026-11-20: A3b done (unbuilt). Compose env goes through `Tokens::env` (launch, resume and revive); contained.rs has a composed path with read-only auth; the ConfigSync block moved earlier in serve.rs. The manager made ConfigSync::apply drop sessions whose composed home is gone, instead of adding archive/delete hooks. BLOCKER: history and resume still read the global cli_home, so sessions_core/src/agents.rs (BROWSER's) needs `with_session_home`, plus callers in history_service:328, the sessions_carry launch callers and revive. Ruling asked of main.
- 2026-11-20: A1c done: upgrade rigs + FUSEFS need (my BUILD hunks); batch 7 sent. Open: R2, agents.rs ruling, frame 211 recording → Fable judge → C2b.
- 2026-11-20: main compile run: the upgrade tables were typed for 4 entries and now hold 5 (fixed by the manager); R2 is still mid-change in volume.rs (errors sent to it). Rulings: A3c (sonnet) adds agents.rs with_session_home as a listed hunk and wires the callers; config_sync pruning is approved; extra A1b claims are approved.
- 2026-11-20: frame 211 recorded by main and Fable judge spawned. R2 has to fix VfsTarget E0433; all batches wait on R2.
- 2026-11-20: A3c, first pass: agents.rs with_session_home and its test (listed hunk), agents_for_session in config_sync, history learn, session_service learn sites. A3c was sent back for revive/resume (rebuild/start_fresh), fork/turns/conversation_read, svc_stop_restart restart.rs:356 through a TabEnvPort::session_home, and the sessions_carry callers.
- 2026-11-20: R2 done (unbuilt). VfsError is now struct variants {op,target}, plus JsonIo/JsonInvalid with a source, and the errno mapping is unchanged. Main was told to run batches 1-7.
- 2026-11-20: Fable signed off frame 211 with edits, logged as D904. The manager corrected the host wording to "The host is this Mac." C2b (sonnet) is building the native page.
- 2026-11-20: A3c second pass: done. Rebuild/start_fresh/fork/turns/conversation_read resolve the composed home. svc_stop_restart goes through TabEnvPort::session_home. The MACHINE_ROOT static is kept, because no ConfigSync handle is reachable from the Starter or session services. The batch is sent to main. Open: carry packs from and lands in the global home, so a moved session would not resume. A3d (sonnet) is fixing the humatad carry callers and the landing.
- 2026-11-20: C2b first pass: done. It adds ui/settings sync.rs and the nav entry, promotes 211 to FRAMES, and adds the nav to frames 29 and 117. It was sent back for:
  - real Default/On/Off popup menus, with the open state and PickMachine in features/settings sync, instead of click-to-cycle;
  - a native snapshot case.
- 2026-11-20: C2b second pass: done. It has real popup menus (PickMachine in the reducer) and the native settings_sync tests. The batch is sent to main with targets and goldens to record (211, 29, 117 and native settings-sync).
- 2026-11-20: A3d done. SessionCarrySvc now resolves homes through SessionHomes (humatad ComposedHomes): packing reads the composed home, and the target creates it before unpacking. Compose keeps the conversation folders (tested). The batch is sent to main. Follow-up: an incoming session with no agent known makes all three agent homes; reading the agent from the manifest would fix that. Every worker batch is now with main; next: build fixes, then T12.
- 2026-11-20: main: all of //apps/humata2/... and //libs/fusefs/... compiles. The one error was C2b's recursion limit, fixed by main with recursion_limit=256 in ui/settings/src/lib.rs (a main hunk). Fast-tier gate running. Next: failures, the 211 and native PNGs, a Fable judgement, then goldens.
- 2026-11-20: Fast gate early failures, all from the fifth helper:
  - continue_in_remote and desk_roam_upgrade need FUSEFS;
  - upgrade_box release b doesn't stage fusefs;
  - the machines fake_ssh count;
  - the lifecycle_restoration ssh script.
  G1 (sonnet) is fixing them.
- 2026-11-20: G1 done. FUSEFS is added to 4 more TEST_NEEDS entries; upgrade_box release b stages fusefs; fake_ssh expects 2 installs (holder and fusefs deleted); lifecycle_restoration scripts the fusefs install. Rerun targets sent to main.
- 2026-11-20: Fallback plan accepted by main; FB1 and FB2 running. More gate failures at 499/831, assigned:
  - fork_remote and move_cheap home mismatch → A3e;
  - svc_upgrade fixtures → G2 (haiku);
  - live_flows stop_and_start timeout → G3.
- 2026-11-20: FB1 done. The 7 MCP files tools are restored (svc_mcp mcp.rs, history.rs and the 3 humatad tests are back to HEAD; tools.rs is HEAD plus BROWSER); the files.md fallback section is added; D907 withdraws B2. The batch is sent to main.
- 2026-11-20: G2 done. svc_upgrade fixtures (local.rs, run.rs) stage 5 helpers; no product change. Sent to main.
- 2026-11-20: FB2 done:
  - the supervisor's mounted flag;
  - HUMATA_FUSE_ROOT only while mounted;
  - an MCP initialize NO_MOUNT notice, from the env the server inherits (main-seam hunks in mcp/src/server.rs);
  - files_without_mount_test, covering CLI locations, trash and env.
  Sent to main. FB2 round 2 adds MCP tools to that test.
- 2026-11-20: G3: live_flows stop_and_start timeout is not our lane. The app's events_ended races load() against follow() across the stop.exec relaunch, and no Resync comes on a fresh subscribe. The suggested fix is Resync on subscribe in clients/live_daemon events.rs. Asked main to rule on the claim.
- 2026-11-20: G4 (sonnet) is fixing the reconnect race: Resync on subscribe, with a failing-first store test. FB2 round 2: an MCP files-tools test across locations (writes own clone and shared; reads another session through approval; lists channel, main and the session folder). A3e: move_cheap fixed (incoming homes go by agent name, not format). fork_remote's root cause is launch_with recording the global conversation_file before compose. Asked main for the sessions_core start.rs claim; the record-rewrite heuristic is to be replaced.
- 2026-11-20: main gave us sessions_core start.rs. A3f (sonnet): Starter gets a session-home seam so launch_with records the composed conversation_file; A3e's settle_recorded_file and its lookup fallback are removed; plus a sessions_core unit test.
- 2026-11-20: G4 done. live_daemon subscribe sends Resync first; the fake gets opt-in knobs; new reconnect_reload_test, a fake_daemon test and a boards live test. Sent to main.
- 2026-11-20: A3f done. The Agents session-home seam (get_for_session) means launch_with records the composed conversation_file; A3e's workarounds are removed. Sent to main. Follow-up: when compose fails and the agent falls back to its real home, the record still names the composed path. G4 round 2 (a deterministic held-load test) is running, per main.
- 2026-11-20: G4 round 2 done. The fake hold_next_subscribe gate makes reconnect_reload deterministic, and Ready is proven by a request only the Resync load can see. Sent to main. A3g (per main, before close): record the home the agent was actually launched in, deciding once at plan time; compose failure means the record names the global home. Sonnet worker started.
- 2026-11-20: A3g round 1 reviewed. The plan composes once and the record follows; spawn only refreshes an existing dir. Sent back because materialize_session creates the dir before steps that can fail, so a failed plan left a dir that env would use. Round 2: plan_home removes a dir it created on failure and keeps a pre-existing one.
- 2026-11-20: A3g round 2 done. plan_home removes a dir it created on failure and keeps a pre-existing one; 2 more tests. Sent to main with targets humatad2_unit_test, fork_remote, move_cheap and sessions_core_unit.
- 2026-11-20: Main's state-machine sweep found four fusefs.rs bugs, all verified in code: the reaper only logs; one failed push sets live=false for good; switch threads race; Settings shows only the saved flag. FB3 (Sonnet) is fixing 1-3 and adding mount_status() in fusefs.rs. Fable is drawing candidate 211b, the CannotSetUp state, as D908, with Off unchanged so goldens 211 and native settings-sync hold. FB4, which wires the status to Settings (proto output field, sections.rs, clients, features, ui), follows both.
- 2026-11-20: Fable D908 signed off: Off unchanged; Starting/Mounted get a settings-fusefs-status note; CannotSetUp gets a settings-fusefs-problem needs-you strip. Candidates 211b and 211c are added; render requested from main.
- 2026-11-20: FB3 done (fusefs.rs). Exit notice plus bounded respawn; a failed push recovers instead of disabling; desired+generation switch; mount_status(); 11 tests. Sent to main. FB4 (Sonnet) started: proto fusefs_status=16 (main seam), the daemon hook plus status-change event, clients, features, ui per D908, and native tests.
- 2026-11-20: FB4 done. Proto Settings.fusefs_status=16 (main seam); status-change notice via settings tell; new clients Event::SettingsChanged (live subscribes to Topic::Settings); ui note/strip per D908; native id tests. Sent to main with targets.
- 2026-11-20: Rerun batch (log .bq/logs/20261120-093858-a5180a.log). G1, G2, A3f, A3g, G4 and FB1 pass. The fusefs unit hang was the test Rig dropping _dir before the launcher's fake helper, so ControlServerHandle stop_inner's wake-connect failed and join hung. Fixed by moving _dir last (manager hunk, test only). Workers are on files_without_mount (session never Working) and native settings_sync menu id + todo_step_context triage + view-diff audit; Fable is judging renders1.
- 2026-11-20: files_without_mount fixed. The fake agent idles, so it now uses wait_running; it also asserts the MCP initialize instructions name the files tools (mcp_client keeps the instructions, additive). Sent to main.
- 2026-11-20: Native triage. The settings_sync menu lacked .test_support() on pick_menu's container (fixed). todo_step_context is TODO2WIRE's (opacity-0 link clicked without hover); routed via main. View diffs are only +Tab "Sync" across 7 snapshots, plus the new settings-sync.
- 2026-11-20: Fable approved 211, 211b, 211c, 29 and 117 (211b/c moved to FRAMES; sign-offs in D904/D908). Native held() now gives devbox-1 mcp On to match 211. Sent main the record list: 5 design goldens and 8 native snapshots.

## T13 — resilience tests on Lima remotes (owner, 2026-11-20: "make it a part of the milestones"; part of the exit)
Before T13, coverage was unit-level only: a fake helper in humatad/src/fusefs.rs and the libs/fusefs tests.

**Rig: real Lima VMs as remote machines.**
- **Build on `desktop/tests/live_flows/real_vm.rs`.** It covers the host `humatad2`, real ssh, `humatad2 --role machine` installed by the Add machine flow, and a throwaway user per run. Its ssh wrapper holds the link down, and that is how the disconnect cases cut the link.
- **Link-down cases** (1, 3, 4) may use the shared `h2-terminal-c1`, but never stop, reboot or resize it.
- **Power-cut and real-mount cases** (2, 5) get their own Lima VM, `h2-fuse-r1`. Make it from the cached image (`TERMINAL_VM_IMAGE`), with 1 CPU, 1 GiB and 6 GiB disk, and FUSE (`/dev/fuse`, fuse3) available.
  - Only these tests may `limactl stop --force` it and start it again. That is the remote power cut.
- **Tags:** `manual` + `external` + `slow`. Run in the slow tier before each release, with `--strategy=TestRunner=local` (CONFIG.md › terminal live targets).
- **Gates:** each test returns at once when its VM env var is unset, like real_vm.rs.
- **Waits:** no sleeps. Wait on what the app, the host or the VM shows.

**Cases.** Write each failing-first.
1. **Host daemon restarts in place with FUSE on.** An exec swap or upgrade of humatad2 keeps the helper and the mount. The supervisor adopts the live helper and starts no second one. Running sessions keep `HUMATA_FUSE_ROOT`. In-process part: `humatad2_test_power_cut` profile. Remote part: a Lima remote.
2. **Remote power cut with FUSE on** (`h2-fuse-r1` force-stop then start). The host shows the machine offline. On boot the remote daemon comes back, the mount comes back, Settings reads Mounted, and the sessions resume with the mount env. No session is ended (LIVEHOLDER).
3. **Remote link drop, then reconnect** (ssh wrapper). While the link is down the status changes and Settings shows it. The files-tools note goes to agents only if FUSE can't be set up again. On reconnect the mount comes back and no session ends.
4. **Moving a mounted session to and from a Lima remote.** It gets the mount env at its new place, and the old mount drops it.
5. **Real-mount smoke test on `h2-fuse-r1`.**
   - Mount, then read and write through the mount from an agent shell.
   - Kill the helper; it comes back once.
   - Kill it again within 30 s; Settings shows "can't set up" and agents get the files-tools note.
6. **A Lima remote with no FUSE at all** (fuse module or `/dev/fuse` missing; use `h2-terminal-c1` if it has none). Settings shows the reason. Agents are told to use the files tools, and those tools reach every location, live.

Close only when cases 1–6 pass in a slow-tier run.
- 2026-11-20: T13 plan (superseding the first T13 note). Claims:
  - testing/fuse_vm/ (new): the h2-fuse-r1 setup script; main runs it.
  - desktop/tests/live_flows/fuse_vm.rs (new) and the real_vm.rs fixture, opened up to pub(super) and parametrised on the host and ssh config.
  - desktop/BUILD.bazel: a new live_flows_fuse_vm_test target (manual, external, slow, exclusive).
  - daemon/humatad/tests/fusefs_restart.rs (new): case 1, in-process part, on humatad2_test_power_cut. Main-seam hunks: its TEST_NEEDS entry (FUSEFS) and POWER_CUT_SCOPED_TESTS.
  - VM picks:
    - Cases 1 (remote part), 3 and 4 need a working mount, so they run on h2-fuse-r1 too; only 2 and 5 stop it.
    - Case 6 runs on h2-terminal-c1 when that VM has no /dev/fuse. If it does have one, the test runs on h2-fuse-r1 with /dev/fuse hidden for the throwaway user.
  - Gates: FUSE_VM_SSH_CONFIG (h2-fuse-r1) and TERMINAL_LAB_SSH_CONFIG (c1).
  - Workers: W-vm writes the script, W-c1 writes the in-process case 1, W-live writes the live cases 1–6.
- 2026-11-20: T13: W-vm wrote testing/fuse_vm/h2_fuse_r1.sh (Lima template:debian cache; no TERMINAL_VM_IMAGE var exists). W-c1 wrote tests/fusefs_restart.rs (+ POWER_CUT_SCOPED_TESTS / TEST_NEEDS hunks); I added a wait-for-Off before its helper kill. W-live still running.
- 2026-11-20: main created h2-fuse-r1 (Debian, kernel 6.12 cloud-arm64): /dev/fuse and fuse3 present, user_allow_other set, 4623 MiB free. fusefs_restart_test is queued.
- 2026-11-20: T13: W-live wrote desktop/tests/live_flows/fuse_vm.rs (6 cases), the real_vm.rs fixture opening (pub(super), start_on, rig_of, refresh_link_config, an fstab line for the fuse VM user), mod fuse_vm in live_flows.rs, and the live_flows_fuse_vm_test target. Not compiled. Known product gap: the host can't read a remote's fusefs_status, so the tests read it on the VM through humata settings get.
- 2026-11-20: gate fixes:
  - setup_test: answer every payloads() install (machines/tests/setup.rs).
  - ports_unit_test: the release.rs fixture names humata2_fusefs.
  - files_without_mount: moved from TRANSFERS_SCOPED_TESTS to Tier C (it runs humatad2 mcp; matrix §3.7).
  - continue_in: the fake agents wrote under the shared home, but the record (since A3f) names the session's composed home. They now resolve $CODEX_HOME/$COPILOT_HOME. The 50 ms poll is now 2 ms.
- 2026-11-20: fusefs_restart_test PASSED (8.9s). Main has queued the rerun of setup/ports/files_without_mount/continue_in and lifecycle_restoration/fake_ssh. live_flows_fuse_vm_test is still building.
- 2026-11-20: main fixed the raw-string delimiters in real_vm.rs CLEANUP (r##). I removed the unused provider::self import in machines/src/manage.rs:43 (committed code from 267831ca, not this lane's, but cleared under fix-every-failure).
- 2026-11-20: continue_in: the json! macro arrays are bound to locals. files_without_mount: the test passed repo as {owner,name}, but the MCP schema takes "owner/name"; the wrong-branch error came from oneOf reporting its last branch. The test is fixed; the product already reaches every root (D907).
- 2026-11-20: Lima run 1: fuse_vm_6 passed. The other 5 failed at the add-machine check because the VM had no git. Main installed git. h2_fuse_r1.sh now installs git, and its marker is now provisioned-2, so 'create' sets up an older VM again.
- 2026-11-20: continue_in root cause, a product bug from A3f: the readers of an existing session's conversation used the shared-home kind, so they never found the conversation in the session's own home. Fix:
  - a read-only seam, Agents::with_existing_session_homes and get_as_ran (it never composes);
  - config_sync::existing_home_of, wired in serve.rs;
  - readers moved onto it: turns_list, rename_folder x2, continue_in source, fork parent readers, and carry locate/stage;
  - targets stay on get_for_session;
  - 2 new unit tests in sessions_core agents.rs.
  Follow-up: a session launched before A3f that later got a composed home would now look in the composed home.
- 2026-11-20: the 2 new agents.rs tests failed. The ids 019a-own and 019a-old aren't UUIDs, and the Codex adapter's check_id only accepts a 36-character UUID, so locate failed and turns returned Ok(None). Fixed by giving them UUID ids. The line shapes already matched conversation/testdata/codex/three_turns.jsonl; thread_id added.
- 2026-11-20: continue_in_test went from 15 failures to 4. The 4 left checked for the copy in the shared ~/.codex and ~/.copilot. Since A3f the copy lands in the new session's composed home, so the assertion now uses in_own_home: .sessions/<new id>/config/{codex/sessions|copilot/session-state}.
- 2026-11-20: Lima run 2 got past add-machine. Cases 1–5 then timed out waiting for the mount.
  - Root cause: the Linux mount was never implemented. PosixMountBackend runs /sbin/mount -t linux, and nothing serves /dev/fuse.
  - The VM is fine: /dev/fuse is 666, fusermount3 is setuid, user_allow_other is set.
  - The refused mount does reach CannotSetUp through expect_mount, so agents fall back.
  - A Sonnet worker is writing the fusermount3 mount, wire codec and serve loop in libs/fusefs/linux, plus LinuxFuseBackend.
  - fuse_vm.rs mount waits now fail fast when the helper reports a failed mount, printing what the daemon logged.
  - macOS FSKit serve is split out to FSKITSERVE (0.10.5, D911).
- 2026-11-20: the Linux FUSE mount is written, not yet built:
  - libs/fusefs/linux: mount.rs (fusermount3 + SCM_RIGHTS, unmount -u/-z); wire.rs (protocol 7.31 codec, INIT, FORGET, ENOSYS, CREATE, MKNOD for regular files); the serve loop; FuseDispatcher::for_owner.
  - vfs: Volume::create, defaulting to ENOSYS, implemented in InMemoryVolume.
  - fusefs_adapter: linux_mount.rs LinuxFuseBackend; run_serve picks it for --mode linux; layout.rs delegates create.
  - Follow-ups: files created in the mount live only in the helper's in-memory volume, so they are lost when the helper restarts; a single-threaded serve loop means one slow call stalls the mount; with the access hook attached, create needs a lease.
- 2026-11-20 (manager): T13 cases 1-5 failed because the cases open a Terminal session and only agent tabs got HUMATA_FUSE_ROOT (svc_stop_restart restart.rs compose_env returned early for Shell tabs; ComposeEnv::env returned nothing for a non-agent). Fixed: session shells now get HUMATA_FUSE_ROOT (only while mounted) and HUMATA_MACHINE_NAME, no config home; machine shells none (D916). Unit tests updated in wire_stop_restart.rs and restart.rs.
