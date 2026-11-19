# SPLIT M2: Evidence-Based Test Capability Matrix (`daemon/humatad` & `clients/live_*`)

- **Scope:** All **216** `apps/humata2/daemon/humatad/tests/*.rs` integration targets and all **14** `apps/humata2/clients/live_*/tests/*.rs` targets.
- **Method:** Static source inspection of actual gRPC `*ServiceClient` usage, `HUMATA2_STOP_AT`/`HUMATA2_HOLD_AT` fault injection, `HUMATAD2_RELEASE`/`remote_rig` usage, `HUMATA` CLI invocations, `HUMATA2_HOLDER` spawning, and `HUMATA2_STACK` usage.
- **Design rule:** Every test is assigned to the **narrowest** scoped binary profile whose linked services cover its actual runtime calls — never by nominal ownership alone.

---

## 1. Minimum Scoped Binary Profiles (Tier B + Tier C)

| Profile | Twins | Linked Services | Excluded Services (Zero Cache Invalidation) | Assigned `humatad` Tests | Assigned `live_*` Tests |
|---|---|---|---|---:|---:|
| **`humatad2_test_power_cut`** | `_faults`, `_release` | `svc_sessions_{common,local,transfer,archive}`, `svc_channels`, `svc_desk`, `svc_terminal`, `svc_inbox`, `svc_machines`, `svc_upgrade`, `svc_stop_restart`, `svc_peer`, `svc_transfers` | **`svc_review`, `svc_board`, `svc_files`, `svc_mcp`, `svc_catalog`, `svc_away`** | **95** | **2** (`live_daemon/inbox`, `live_daemon/term_restart`) |
| **`humatad2_test_machines`** | `_release`, `_full` (+ `sessions`/`channels`/`faults`) | `svc_machines`, `svc_peer`, `svc_upgrade`, `svc_stop_restart` (+ `svc_sessions_local`, `svc_channels`, `svc_terminal` in `_full`) | `svc_review`, `svc_board`, `svc_files`, `svc_mcp`, `svc_away` | **10** | **0** |
| **`humatad2_test_review_remote`** | `_faults` | `svc_review`, `svc_board`, `svc_files`, `svc_sessions_{common,local,transfer}`, `svc_channels`, `svc_transfers`, `svc_peer` | `svc_mcp`, `svc_catalog`, `svc_away`, `svc_upgrade` | **22** | **4** (`live_diff/diffs`, `live_review/{parity_comments,pull_requests,review_chat}`) |
| **`humatad2_test_catalog_remote`** *(alias → `humatad2_test_machines`)* | *(none)* | `humatad_core` + `svc_catalog` (same binary as `humatad2_test_machines`) | `svc_review`, `svc_board`, `svc_mcp` | **35** | **3** (`live_daemon/{github_repos,history,marks}`) |
| **`humatad2_test_away_board`** | `_release` | `humatad_core` + `svc_board` + `svc_catalog` | `svc_review`, `svc_mcp` | **21** | **4** (`live_daemon/boards`, `live_desk/desk`, `live_files/file_manager`, `live_machines/machines`) |
| **`humatad2_test_transfers`** *(alias → `humatad2_test_machines`)* | *(none; `_faults` removed)* | `humatad_core` + `svc_catalog` (same binary as `humatad2_test_machines`) | `svc_review`, `svc_board`, `svc_mcp` | **6** | **0** |
| **Tier C (`:humatad2` / `:humatad2_admin`)** | full daemon | All services (CLI, auth, MCP stdio, full parity, cross-cutting upgrade E2E) | None (explicit full-daemon E2E set) | **27** | **1** (`live_daemon/parity_daemon`) |
| **Total** | | | | **216** | **14** |

> [!IMPORTANT]
> **`HUMATAD2_RELEASE` cache-isolation rule (`remote_rig.rs:199`):** Every test with `needs_release = true` spawns `HUMATAD2_RELEASE` for the fake box daemon. Each Tier-B profile that runs `remote_rig` (`power_cut`, `machines`, `away_board`) MUST provide a scoped `*_release` twin (non-faults, non-admin) so edits to excluded services (e.g., `svc_review`, `svc_board`, `svc_files`) never invalidate `remote_rig` tests via monolithic `:humatad2`.

---

## 2. `apps/humata2/clients/live_*/tests/*.rs` Capability Matrix (14 targets)

| Test Target | Required Services | `admin` / `faults` / `release` | `cli` / `holder` / `stack` | Assigned Daemon Profile |
|---|---|---|---|---|
| `clients/live_daemon/tests/boards.rs` | `board`, `channels`, `sessions` | `admin` | — | **`humatad2_test_away_board`** |
| `clients/live_daemon/tests/github_repos.rs` | `catalog`, `channels` | `admin` | — | **`humatad2_test_catalog_remote`** |
| `clients/live_daemon/tests/history.rs` | `catalog`, `channels`, `sessions` | `admin` | — | **`humatad2_test_catalog_remote`** |
| `clients/live_daemon/tests/inbox.rs` | `inbox`, `channels`, `sessions` | `admin` | — | **`humatad2_test_power_cut`** |
| `clients/live_daemon/tests/marks.rs` | `catalog`, `channels`, `sessions` | `admin` | — | **`humatad2_test_catalog_remote`** |
| `clients/live_daemon/tests/parity_daemon.rs` | full workspace parity (`channels`, `sessions`, `board`, `inbox`, `files`, `machines`, `catalog`) | `admin` | `holder` | **Tier C (`:humatad2_admin`)** |
| `clients/live_daemon/tests/term_restart.rs` | `terminal`, `sessions`, `channels`, `lifecycle` | `admin` | `holder` | **`humatad2_test_away_board`** (the workspace load reads board items) |
| `clients/live_desk/tests/desk.rs` | `desk`, `channels`, `sessions`, `board` (workspace load) | `admin` | `holder` | **`humatad2_test_away_board`** (2026-11-19: the workspace load reads board items) |
| `clients/live_diff/tests/diffs.rs` | `review`, `sessions`, `channels` | `admin` | — | **`humatad2_test_review_remote`** |
| `clients/live_files/tests/file_manager.rs` | `files`, `transfers`, `trash`, `desk`, `channels`, `board` (workspace load) | `admin` | — | **`humatad2_test_away_board`** (2026-11-19: the workspace load reads board items) |
| `clients/live_machines/tests/machines.rs` | `machines`, `peer`, `lifecycle`, `catalog` (providers), `board` (workspace load) | `admin`, `release` | — | **`humatad2_test_away_board`** (+ `_release`; 2026-11-19: the workspace load reads board items) |
| `clients/live_review/tests/parity_comments.rs` | `review`, `sessions`, `channels`, `boards` | `admin` | — | **`humatad2_test_review_remote`** |
| `clients/live_review/tests/pull_requests.rs` | `review`, `sessions`, `channels` | `admin` | — | **`humatad2_test_review_remote`** |
| `clients/live_review/tests/review_chat.rs` | `review`, `sessions`, `channels` | `admin` | — | **`humatad2_test_review_remote`** |

---

## 3. `apps/humata2/daemon/humatad/tests/*.rs` Capability Matrix (216 targets)

### 3.1 `humatad2_test_power_cut` (+ `_faults`, `_release`) — 95 tests
Excludes `svc_review`, `svc_board`, `svc_files`, `svc_mcp`, `svc_catalog`, `svc_away`.

- **Power-cut, move, rebuild, fork, archive, stop & upgrade fault/rig tests (55 tests — `needs_faults`, `needs_release`, `needs_holder`, `needs_stack` where noted):**
  - `move_session`, `move_session_freeze`, `move_cheap`, `rebuild_session`, `fork_remote`, `own_copy_remote` (+ `stack`), `remote_archive`, `archive`, `archive_live`, `continue_in`, `continue_in_journal` (+ `stack`), `continue_in_remote` (+ `stack`), `start_fresh`, `context_fork`, `fork` (`sessions`, `channels`, `transfers`, `peer`, `faults`, `release`, `holder`)
  - `inbox_copy` (`inbox`, `transfers`, `channels`, `peer`, `release`): moved from transfers 2026-11-19, which lacks inbox and channel relay
  - `stop_box`, `stop_sessions`, `upgrade_box` (+ `stack`), `upgrade_cut` (+ `stack`), `upgrade_holders` (+ `stack`), `upgrade_inbox` (+ `stack`), `upgrade_kinds` (+ `stack`), `upgrade_refusals` (+ `stack`), `upgrade_remote_holders` (+ `stack`) (`lifecycle`, `sessions`, `inbox`, `peer`, `faults`, `release`, `holder`, `stack`)
  - `change_root`, `channel_delete_remote`, `channel_rename_roam`, `desk_close_remote`, `desk_events_remote`, `desk_into_channel_roam`, `desk_migrate_remote`, `desk_picked_remote`, `desk_roam`, `desk_roam_upgrade` (+ `stack`), `desk_start_host_cut`, `desk_stuck_remote`, `desk_versions_remote`, `remove_machine_cut`, `remove_machine_live`, `restart_operations`, `restore_channel_cut`, `channel_desk`, `channel_rename_live`, `desk_into_channel`, `desk_move_live`, `detach_live`, `hooks_remote`, `host_variables_move`, `pointers`, `push_remote`, `push_remote_restart`, `remote_rename`, `start_resume_failed_remote`, `start_stuck_remote`, `stop_launching` (`channels`, `desk`, `sessions`, `peer`, `lifecycle`, `faults`, `release`)
- **Lifecycle, terminal, inbox & local session/channel tests (51 tests — `sessions`/`channels`/`desk`/`terminal`/`inbox`/`lifecycle`):**
  - Lifecycle subprocess/signal: `daemon`, `doctor`, `ready` (`holder`), `restart`, `socket_paths`, `state_lock`, `stop`, `faults` (`faults`)
  - Terminal & holders: `adopt_holder` (`holder`), `checkpoint`, `destroy_holders` (`holder`), `holder_gone` (`holder`), `holder_swap_identity` (`holder`), `lagged_attach`, `paste`, `resume_exits_at_once`, `resume_exits_at_once_remote` (`release`), `running_holders` (`holder`), `running_holders_remote` (`release`, `holder`), `tab_tools` (`cli`), `terminal` (`holder`, `faults`), `terminal_program`
  - Inbox & screen questions: `inbox`, `ready_gate` (`cli`), `screen_questions` (`cli`, `holder`), `screen_questions_remote` (`release`, `holder`)
  - Local channels/desk/sessions: `channel_add`, `channel_delete_live`, `channel_desk_close`, `channel_desk_retarget`, `channel_folder_follows`, `channel_grid`, `channel_repo_propose`, `channels`, `conversation_follows`, `delete_launching`, `desk_agent_missing`, `desk_events`, `desk_folder_files`, `desk_folder_migration`, `desk_hide_live`, `desk_move`, `doc_history`, `doc_version_diff`, `doc_versions`, `fork_channel`, `gh_repos`, `hooks`, `push`, `push_base`, `push_many`, `remove_machine_plan`, `replies`, `repo_library_adds`, `session_add_repo`, `session_events`, `session_model`, `session_notes`, `sessions`, `turns`

### 3.2 `humatad2_test_machines` (+ `_release`, `_full`) — 10 tests
`machines` + `peer` + `lifecycle` (+ `sessions`/`channels`/`faults` in `_full`).
- Core machines/peer: `host_variables`, `machines`, `host_variables_remote` (`release`), `machine_shell_remote` (`release`), `machine_shell_remote_linux` (`release`), `storage_remote` (`release`), `two_machines` (`release`)
- Full machines/peer (`_full`): `forget_machine` (`faults`, `release`), `peer_stdio` (`release`), `relay` (`release`), `remote` (`release`), `remote_history` (`release`)

### 3.3 `humatad2_test_review_remote` (+ `_faults`) — 22 tests
`review` + `sessions` + `channels` + `transfers` + `peer`.
- `svc_board` and `svc_files` are linked (2026-11-19): `LiveDaemon::load` lists board items, so the `live_*` parity tests need BoardService; an unserved BoardService answers Unimplemented and the load is refused.
- Local review/PR/views: `pr_base`, `pr_commits`, `pr_diff_stress`, `pr_refresh`, `review`, `review_bases`, `review_file_trees`, `review_per_repo`, `review_snapshots`, `review_stack`, `views_forks`, `views_plan_notes`
- Comments and boards outside review (moved from power_cut 2026-11-19, which has no CommentService/BoardService): `delete_channel` (`board`, `trash`, `faults`), `desk` (`board`, `trash`, `faults`), `doc_comments` (`comments`, `desk`), `move_typing` (`comments`, `faults`; its `release` box falls back to `:humatad2`, since this profile has no `_release` twin)
- Faults/remote review: `comments` (`faults`), `pr_review_move` (`faults`, `release`), `pr_review_remote` (`faults`, `release`), `views_remote` (`faults`, `release`)

### 3.4 `humatad2_test_catalog_remote` — 35 tests
`catalog` + `machines` + `peer` + `sessions` + `channels`.
- Local catalog: `branch_names`, `branch_settings`, `codex_trust`, `history`, `marks`, `provider_import`, `provider_remove`, `read_marks`, `repo_owners`, `repos`, `trash`
- Faults/tools/remote catalog: `access` (`faults`), `settings_sections` (`faults`), `login`, `providers`, `skills`, `read_marks_remote` (`release`)
- (Plus any `channels`/`sessions` tests that also query `Repo`/`Settings`/`History`/`Mark` services.)

### 3.5 `humatad2_test_away_board` (+ `_release`) — 21 tests
`away` + `board` + `inbox` + `catalog/access` + `peer` + `stop_restart`.
- Away suite: `away_approved_board`, `away_boundary`, `away_contain_copy`, `away_forward`, `away_inbox`, `away_left_out`, `away_link`, `away_login`, `away_login_pipe`, `away_policy_copy`, `away_replay` (`release`, `cli`)
- Board & cross-boundary access: `boards`, `agent_send_machine` (`release`, `cli`; moved from power_cut 2026-11-19: the box answers its agents' machine, file and transfer calls through BoardService's host calls), `flow_26_10` (`release`), `stop_access` (`release`)

### 3.6 `humatad2_test_transfers` (+ `_faults`) — 6 tests
`transfers` + `files` + `machines` + `catalog/trash` + `peer`.
- Files & transfers: `file_changes`, `file_ops`, `files`, `find_files`, `received_root`, `transfer_end`, `transfers` (`faults`), `remote_files` (`release`), `remote_transfers` (`release`)

### 3.7 Tier C — Explicit Full-Daemon E2E (`:humatad2` / `:humatad2_admin`) — 27 tests
Require full daemon wiring, CLI auth/login, MCP stdio across all services, or full-stack release upgrades:
- CLI / Auth: `auth`, `cli_actor`, `cli_files`, `cli_login`, `cli_waiting`
- MCP: `mcp`, `mcp_desk_folder`, `mcp_doc_versions`, `session_delete_other`
- MCP tools (moved back from Tier B 2026-11-19: they run `humatad2 mcp`, which only `svc_mcp` builds): `board_tools`, `comment_tools`, `history_tools`, `matrix`, `pr_review` (`faults`), `pr_stopped` (`faults`), `repo_tools`
- Power cuts across every service (moved back from power_cut 2026-11-19: the shared `common/power_cut.rs` scenario makes draft comments, board items and view state): `power_cut_both`, `power_cut_focus`, `power_cut_host`, `power_cut_machine`, `power_cut_start_box` (`faults`, `release`, `holder`)
- Away view drafts (moved back from away_board 2026-11-19: it calls ViewService, which only `svc_review` builds): `away_board_journal` (`release`)
- Bridge / Full-stack upgrade E2E: `bridge`, `remote_stack` (`stack`, `release`), `upgrade_holders_move` (`holder`, `release`), `upgrade_identity` (`release`), `upgrade_local` (`release`)
