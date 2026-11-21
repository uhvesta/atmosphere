# BROWSER: agents drive my own browser, asking me first

- **Status:** open (owner, 2026-11-20). Core crates committed in 12305aba: `daemon/browser_copilot`, `browser_extension/{chromium,firefox,shared}`.
- **Release:** 0.7.0 (RESUME.md › Release plan, step 19). Full suite + launch check.
- **Depends on:** FUSEFSWIRE (0.6.0) and SCREENPARSE (0.6.1) shipping first, since releases go out in order. The lanes may start earlier if their files don't overlap.
- **Outcome sentence:** Claude Code, Codex, Copilot and agy can read my real browser tabs and, with my per-action approval, click, type and navigate in them. Read access can be granted for 30 minutes; write access never can.

## Review notes before starting (Claude review, 2026-11-20)
- **Frames first.** Fable draws and signs off Settings › Browser and the two Browser request cards (read and write) before Lane C starts. The native scenes then copy them exactly. Re-recording `29-settings` is only allowed against a signed-off frame.
- **Install hygiene.** `InstallBrowserHost` writes into browser profile folders under the owner's home. It must:
  - be undoable through `UninstallBrowserHost`;
  - touch only the native-messaging manifest it owns;
  - never touch `~/.claude`, `~/.codex`, `~/.copilot` or `~/.gemini`.
- **Remote machines (owner, 2026-11-20, D891).** An agent on a remote machine gets the same browser access to the host Mac as a local session: the same switches, the same read and write grants, the same Inbox requests. The request card still names the machine the agent runs on, as it does for any other request.
- **Tests.** Use a fake browser home in a temp dir for every install test. Nothing may write to the real browser folders during tests.

## Brief (owner, 2026-11-20, as given)

Work in `/Users/avestabarzegar/oss/atmosphere-khodr` on branch `atmosphere-khodr`.

`0.7.0 BROWSER` core crates and extensions are committed:
- **`//apps/humata2/daemon/browser_copilot`** (`browser_copilot_test` green):
  - `BrowserDetector`;
  - `ManifestInstaller` (`NATIVE_HOST_NAME = "app.humata2.browser_host"`);
  - the stdio codec: 4-byte native-endian length prefix, 1 MiB max (`read_frame` / `write_frame`);
  - typed `BrowserAction` / `BrowserRequest` / `BrowserReply`;
  - `PermissionGate`: `SupportedAgent::{Claude, Codex, Copilot, Agy}`, 30-minute `Read` grants via `READ_GRANT_TTL_MS`, and strict per-action `Write` that rejects `AllowFor30Min`;
  - `McpDispatcher::tool_schemas()` (6 tools).
- **`apps/humata2/browser_extension/{chromium,firefox,shared}/`:** MV3 Chromium and Firefox WebExtensions that call `connectNative("app.humata2.browser_host")`, plus a Shadow-DOM guide/highlight overlay.

The job: wire `0.7.0 BROWSER` end to end across proto, `humatad2`, `svc_mcp`, `svc_inbox`, `clients`, `features` and `ui`; verify with loopback integration tests and `presubmit.sh --full`; commit cleanly.

### Hard constraints
- **Bazel only**, through `tools/bq/bq` and `apps/humata2/tools/tests/presubmit.sh`. Never `cargo`, `bazel clean` or `bazel shutdown`.
- **Strict 4-agent scope:** `claude`, `codex`, `copilot` and `agy` only.
  - Never expose `browser_*` tools to `opencode`, `zsh` or custom agents.
  - Never embed a webview or an internal browser.
- **Strict permission boundary:**

  | | Actions | Answers offered |
  |---|---|---|
  | **Read** | `browser_list_tabs`, `browser_read_tab`, `browser_guide_highlight` | Allow once, Allow 30 min, Deny |
  | **Write** | `browser_click`, `browser_type`, `browser_navigate` | Allow once, Deny only |

  - A Write card shows the exact URL and a preview of the target.
  - The server MUST reject `AllowFor30Min` and `AlwaysLet` for Write.
  - `Approve all` MUST skip `BrowserWrite`.
- **Never** name outside projects in code, comments, docs or commits.
- **Headless only:**
  - never open the app or GUI windows;
  - never kill running terminals or holders;
  - never replace launchd `app.humata2.humatad2`;
  - never touch the real `~/.claude`, `~/.codex`, `~/.copilot` or `~/.gemini`.
- **Git hygiene:** stage only with explicit `git add -- <paths>`. Never commit the owner's foreign files.

### Plan: 3 lanes with no shared files, after locking `inbox.proto` + `settings.proto`

#### Lane A: proto, `humatad2 browser-host`, daemon bridge, host settings RPCs
Owned files:
- `api/proto/humata/v1/{inbox.proto,settings.proto}`;
- `daemon/humatad/{BUILD.bazel,Cargo.toml}`;
- `daemon/humatad/src/{lib.rs,main.rs,error.rs,identity.rs,serve.rs,settings_service.rs,browser_host.rs (new),browser_service.rs (new)}`;
- `daemon/humatad/tests/browser_host.rs (new)`;
- `daemon/agent_hooks/src/mcp.rs`;
- `daemon/sessions_core/src/agents.rs`;
- `daemon/humatad/src/session_service/hooks.rs`.

1. **`inbox.proto` (additive only):**
   - In `RequestKind`, add `REQUEST_KIND_BROWSER_READ = 9;` and `REQUEST_KIND_BROWSER_WRITE = 10;`.
   - Add `message BrowserPreview { string url = 1; string action_summary = 2; string target_preview = 3; bool supports_30m_grant = 4; }`.
   - In `RequestPreview.preview`, add `BrowserPreview browser_read = 18; BrowserPreview browser_write = 19;`.
   - Add `message AllowFor30Min {}`, and in `RequestAnswer.answer` add `AllowFor30Min allow_for_30_min = 7;`.
   - In `InboxService`, add `rpc RequestBrowserApproval(RequestBrowserApprovalRequest) returns (WaitResponse);`. The request carries `idempotency_key`, `bool is_write`, `BrowserPreview preview`, `string action_key` and `uint32 wait_seconds`.
2. **`settings.proto` (additive only):**
   - Add:
     - `enum BrowserKind` (`Chrome`..`Firefox` = 1..7);
     - `enum BrowserFamily` (`Chromium = 1`, `Firefox = 2`);
     - `message DetectedBrowser` with kind, family, display_name, binary_path, native_messaging_dir, manifest_path, host_installed, host_matches_current, connected and extension_path.
   - On `SettingsService`, add `ListDetectedBrowsers`, `InstallBrowserHost` and `UninstallBrowserHost`.
   - Add `service BrowserBridgeService` with:
     - `rpc ConnectBrowserHost(stream BrowserHostFrame) returns (stream BrowserDaemonFrame);`
     - `rpc ExecuteBrowserAction(ExecuteBrowserActionRequest) returns (ExecuteBrowserActionResponse);`
     - `rpc GetBrowserBridgeStatus(GetBrowserBridgeStatusRequest) returns (GetBrowserBridgeStatusResponse);`
3. **`humatad2 browser-host` and the `argv[0]` symlink** (`main.rs`, `browser_host.rs`):
   - Native Messaging manifests have a `"path"` and no `"args"` array. The browser passes `<extension-origin>` as `argv[1]`.
   - `InstallBrowserHost` atomically creates the symlink `~/.humata2/bin/humata2-browser-host -> <current_humatad2_exe>` and writes that path into the manifest.
   - At the top of `daemon/humatad/src/main.rs`, before `Cli::parse()`, detect an `argv[0]` basename of `humata2-browser-host`.
   - Also expose `Command::BrowserHost` (`--socket`, `--token-file`).
   - The host runs on `tokio::runtime::Builder::new_current_thread().enable_all()` (~5 MB RSS).
   - It connects over UDS to `ConnectBrowserHost`, then pumps 4-byte native-endian JSON frames (`read_frame` / `write_frame`) between stdio and gRPC until EOF.
4. **UDS auth** (`identity.rs`):
   - Mint `<state_dir>/browser-host.token` with mode `0o600`.
   - A caller becomes `Caller::BrowserHost` only if it sends the `humata-browser-host-token` header, has the same uid, and is not inside a session (`!in_a_session(pid)`).
   - `Caller::BrowserHost` is authorized ONLY for `/humata.v1.BrowserBridgeService/ConnectBrowserHost`.
5. **Bridge and `agy` MCP parity.** Files: `browser_service.rs`, `settings_service.rs`, `serve.rs`, `agent_hooks/src/mcp.rs`, `sessions_core/src/agents.rs`, `session_service/hooks.rs`.
   - Register `BrowserBridgeServiceServer` in `serve.rs`. Correlate replies to requests with `request_id` oneshots, with a 30 s timeout.
   - On `SettingsSvc`, implement `ListDetectedBrowsers` / `InstallBrowserHost` / `UninstallBrowserHost` (user-only) with `BrowserDetector` and `ManifestInstaller`.
   - Wire the `agy` MCP launch config alongside `claude`, `codex` and `copilot`.

#### Lane B: `svc_mcp` + `svc_inbox` wiring
Owned files:
- `daemon/mcp/{src/tools.rs,tests/{tools.txt,schemas.rs,support/mod.rs}}`;
- `daemon/svc_mcp/{BUILD.bazel,src/{mcp.rs,browser.rs (new)}}`;
- `daemon/svc_inbox/{BUILD.bazel,src/inbox.rs,src/inbox/{browser.rs (new),replay.rs,deferred.rs}}`;
- `daemon/humatad/tests/browser_mcp.rs (new)`.

1. Register the 6 `Operation::Browser*` tools in `daemon/mcp/src/tools.rs` and `daemon/mcp/tests/tools.txt`: `browser_list_tabs`, `browser_read_tab`, `browser_click`, `browser_type`, `browser_navigate` and `browser_guide_highlight`. There is NO `browser_submit`.
2. **`svc_mcp` (`GrpcDaemon`):**
   - Mirror `delete_many_offered` with `browser_offered: Arc<AtomicBool>`, set in `bind()` only if `SupportedAgent::parse(agent_id).is_ok()`.
   - Gate both `serves(op)` and `call(call)` on it.
   - For each call:
     1. Check `GetBrowserBridgeStatus`. If the browser is disconnected, return an actionable "Open Settings › Browser" error.
     2. Call `RequestBrowserApproval` + `WaitForAnswer`, with `InFlight` auto-withdraw.
     3. Execute through `ExecuteBrowserAction`.
3. **`svc_inbox`:** wire `PermissionGate` in `inbox/browser.rs`.
   - On `RequestBrowserApproval`:
     - require `Caller::Agent` and a supported agent (`SupportedAgent::parse`);
     - clamp `supports_30m_grant = !req.is_write` on the server;
     - if an active read grant younger than 30 min exists, return `AllowFor30Min` at once;
     - otherwise raise a `BrowserRead` or `BrowserWrite` card.
   - In `decide`, `fits_card`, `replay.rs` and `deferred.rs`:
     - reject `AllowFor30Min` on `BrowserWrite`;
     - reject `AlwaysLet` on every browser request.
   - Skip `BrowserWrite` in `answer_all` (`Approve all`).

#### Lane C: clients, features, desktop UI
Owned files:
- `clients/src/{model.rs,daemon.rs,daemon/fake.rs,daemon/fake/inbox.rs,fixture/requests.rs}`;
- `clients/live_daemon/src/{inbox.rs,settings.rs,daemon.rs}`;
- `features/notify/src/lib.rs`, `features/inbox/src/lib.rs`;
- `features/settings/src/{lib.rs,browser.rs (new)}`;
- `features/app/{src/events.rs,tests/every_action.rs}`;
- `ui/workspace/src/request_card.rs`;
- `ui/settings/src/{settings.rs,settings_browser.rs (new)}`;
- `desktop/tests/{native.rs,native/browser_settings.rs (new)}`.

1. **Clients:** extend `model.rs`, `daemon.rs`, the fake and `live_daemon` with `RequestKind::{BrowserRead, BrowserWrite}`, `BrowserPreview`, `Answer::AllowFor30Min`, `DetectedBrowser` and the browser host RPCs.
2. **Features:**
   - `features/notify`;
   - `features/inbox`: offer `Offer::AllowFor30Min` only for `BrowserRead` with `supports_30m_grant`, and leave `BrowserWrite` out of `ApproveAll`;
   - `features/settings`: `Section::Browser` plus `BrowserState` / `BrowserAction`;
   - `features/app/tests/every_action.rs`.
3. **Request cards** (`ui/workspace/src/request_card.rs`):
   - **Browser read** offers Allow once, Allow 30 min and Deny.
   - **Browser write** offers Allow once and Deny ONLY, and highlights the exact URL and the target selector preview.
4. **Settings › Browser** (`ui/settings/src/settings_browser.rs`):
   - a table of the browsers found on this Mac;
   - manifest and live connection status;
   - one-click "Set up host", "Uninstall" and "Install all";
   - instructions to unpack and load `apps/humata2/browser_extension/{chromium,firefox}`.

   Regenerate the `29-settings` snapshot if the sidebar's height or items change, copying the signed-off frame (see the review notes above).

### Verification gate
1. Build and unit tests, through `tools/bq/bq`:
   - **Core:** `//apps/humata2/daemon/browser_copilot:browser_copilot_test`
   - **MCP and inbox:** `//apps/humata2/daemon/mcp:all`, `//apps/humata2/daemon/svc_mcp:svc_mcp_unit_test`, `//apps/humata2/daemon/svc_inbox:all`
   - **Daemon:** `//apps/humata2/daemon/agent_hooks:mcp_test`, `//apps/humata2/daemon/humatad:humatad2_unit_test`
   - **Clients:** `//apps/humata2/clients:clients_unit_test`, `//apps/humata2/clients/live_daemon:all`
   - **Features:** `//apps/humata2/features/inbox:all`, `//apps/humata2/features/settings:all`, `//apps/humata2/features/app:every_action_test`
   - **UI:** `//apps/humata2/ui/settings:settings_unit_test`, `//apps/humata2/ui/workspace:workspace_unit_test`, `//apps/humata2/desktop:unit_test`
2. New loopback integration tests:
   - **`//apps/humata2/daemon/humatad:browser_host_test`:** the real `humatad2-browser-host` symlink, 4-byte framing, and UDS token + `!in_a_session` auth rejection.
   - **`//apps/humata2/daemon/humatad:browser_mcp_test`:**
     - the 4-agent scope, with `opencode` and `zsh` refused;
     - a 30-minute Read grant being reused;
     - Write rejecting `AllowFor30Min` and being skipped by `Approve all`.
3. Desktop UI and the full gate:
   - `apps/humata2/tools/tests/presubmit.sh --with-slow //apps/humata2/desktop:shell_test //apps/humata2/desktop:design_test //apps/humata2/desktop:look_test //apps/humata2/desktop:native_test`
   - `apps/humata2/tools/tests/presubmit.sh --full`
4. Commit with explicit `git add -- <paths>`. No AI attribution trailers.

## Lane
**State: resumed 2026-11-20 (0.9.0 shipped at 5e86d620); release target 0.10.0. A1 compiled; A2, B1, A3, A4, A5, B2, B3 and loopback integration tests (`browser_host_test`, `browser_mcp_test`) are written and awaiting build.** b2 re-signed by Fable for the per-session grant (D897). Fable signed off b1–b3 in `workflow/milestones/mockups/browser/` (FRAMES.md). Picks logged as D896–D899.

**Goal / exit:** the outcome sentence above holds, and the verification gate's targets pass.

**Design notes (manager picks, to log in DECISIONS.md on resume):**
- **New `api/proto/humata/v1/browser.proto`**, added to `inbox_proto` srcs (one-line seam in `api/proto/BUILD.bazel` for main). It holds `BrowserBridgeService` with `ConnectBrowserHost`, `ExecuteBrowserAction`, `GetBrowserBridgeStatus` and also `ListDetectedBrowsers` / `InstallBrowserHost` / `UninstallBrowserHost` (user-only), plus `DetectedBrowser`, `BrowserKind`, `BrowserFamily`. So `SettingsService` and `settings_service.rs` stay untouched (no hand-over wait). settings.proto is not edited.
- **inbox.proto (additive):** kinds 9/10, `BrowserPreview`, preview 18/19, `AllowFor30Min` answer 7, `RequestBrowserApproval`. The request carries the action itself (`action_json`); the server derives preview, `is_write` and the action key, so a card can't show a different action than the one that runs.
- **No bypass:** `ExecuteBrowserAction` from an agent needs an approval id the inbox approved for that session with a matching action key (one use), or an active read grant for a read action. Read grants are per session, in memory, 30 min; lost on daemon restart.
- **No `Caller::BrowserHost` in `daemon/ports`** (shared, many exhaustive matches): identity.rs checks the `humata-browser-host-token` header (token file 0o600, same uid, `!in_a_session`, method == ConnectBrowserHost only) and inserts a humatad-local marker (`BrowserHostCaller`) the bridge service checks.
- **Core crate fixes:** `HighlightGuide` classifies as Read (brief's table); drop `expect`/`unreachable!` in `policy.rs`; add a stable action key.
- **svc_mcp:** browser tools in new `svc_mcp/src/mcp/browser.rs`; only a registration hunk in `mcp.rs` (FUSEFSWIRE owns mcp.rs).
- **Remote (D891):** after local works, a machine daemon forwards bridge execute/status to the host over the peer link; card raised on the machine and mirrored as today.

**Tasks:**
- A1 proto — **compiled** (manager). Files: `api/proto/humata/v1/inbox.proto` (additive), `api/proto/humata/v1/browser.proto` (new), `api/proto/BUILD.bazel` (`inbox_proto` srcs line), `daemon/svc_inbox/src/inbox.rs` (mod line, `request_browser_approval` trait method, browser arms in `decide`/`fits_card`, `approving`), `daemon/svc_inbox/src/inbox/browser.rs` (new: stub RPC, `is_read`, `decide`), `daemon/svc_inbox/src/inbox/permission.rs` (AllowFor30Min in two refusal arms), `clients/live_daemon/src/inbox.rs` (kind/preview/answer arms, placeholder until Lane C).
- A2 browser_copilot fixes — **done, awaiting build**. Files: `daemon/browser_copilot/{BUILD.bazel, src/policy.rs, src/protocol.rs, src/mcp_tools.rs, src/lib.rs}`.
- B1 `daemon/mcp` 6 browser Operations + tools.txt + schemas — **done, awaiting build**. Files: `daemon/mcp/src/tools.rs`, `daemon/mcp/tests/tools.txt`, `daemon/mcp/tests/schemas.rs`.
- A3 + A4 + A5 + B2 + B3 (daemon local browser wiring) — **done, awaiting build**:
  - A3: `daemon/humatad/src/browser_service.rs` (new: `BrowserBridgeSvc`, fake home via `HUMATA2_BROWSER_HOME`, atomic symlink, `request_id` oneshot correlation w/ 30 s timeout, approval verification via `Inbox::consume_browser_approval`, unit tests).
  - A4: `daemon/humatad/src/browser_host.rs` (new: `argv[0]` `humata2-browser-host` detection, `Command::BrowserHost`, 4-byte native-endian JSON frame pump), minimal hunks in `daemon/humatad/src/{main.rs,identity.rs,serve.rs,lib.rs,error.rs}` and `daemon/humatad/BUILD.bazel` (`browser_copilot` dep + `browser_host` in `POWER_CUT_SCOPED_TESTS`).
  - A5: `daemon/agent_hooks/src/{mcp.rs,lib.rs}`, `daemon/agent_hooks/tests/mcp.rs`, `daemon/sessions_core/src/agents.rs` (`Builtin::Agy` via `--settings=<state>/humata-mcp-agy.json`, never touching `~/.gemini`), `daemon/humatad/src/session_service/hooks.rs`.
  - B2: `daemon/svc_mcp/src/mcp/browser.rs` (new: `browser_offered`, `GetBrowserBridgeStatus`, `RequestBrowserApproval` + `WaitForAnswer` with `WithdrawOnDrop`, `ExecuteBrowserAction`, unit tests), minimal registration hunks in `daemon/svc_mcp/src/mcp.rs`, `daemon/svc_mcp/BUILD.bazel`.
  - B3: `daemon/svc_inbox/src/inbox/browser.rs` (per-session 30-min in-memory read grants, server-clamped `supports_30m_grant`, `consume_browser_approval`, unit tests), `daemon/svc_inbox/src/inbox.rs` (`browser` field, `record_browser_answer`, `answer_all` skips `BrowserWrite`), `daemon/svc_inbox/src/inbox/replay.rs` (browser answer validation & preview clamping), `daemon/svc_inbox/BUILD.bazel`.
  - Loopback tests: `daemon/humatad/tests/browser_host.rs` (`browser_host_test`, scoped `PC_BASE`), `daemon/humatad/tests/browser_mcp.rs` (`browser_mcp_test`, `BASE` for `svc_mcp`).
- Lane C (clients, features, desktop UI) — **done, awaiting build**:
  - Clients: `clients/src/{model.rs,daemon.rs,daemon/fake.rs,daemon/fake/browser.rs (new),daemon/fake/inbox.rs,fixture.rs,fixture/requests.rs}`, `clients/live_daemon/src/{state.rs,inbox.rs,settings.rs,daemon.rs}` (`BrowserBridgeService` RPCs wired to `browser_bridge()`, `RequestKind::{BrowserRead, BrowserWrite}`, `BrowserPreview`, `Answer::AllowFor30Min`, `DetectedBrowser`, `BrowserKind`, `BrowserFamily`, `BrowserBridgeStatus`, unit tests in `fake/browser.rs`, `live_daemon/src/inbox.rs`, `live_daemon/src/settings.rs`).
  - Features: `features/notify/src/lib.rs`, `features/inbox/src/lib.rs` (`Offer::AllowFor30Min` only for `BrowserRead` with `supports_30m_grant`, `ApproveAll` skips `BrowserWrite`), `features/inbox/tests/browser.rs (new)`, `features/settings/src/{lib.rs,browser.rs (new)}`, `features/settings/tests/browser.rs (new)`, `features/app/{src/events.rs,tests/every_action.rs}`.
  - UI: `ui/core/src/lib.rs` (added `Globe` icon asset), `ui/workspace/src/request_card.rs` (frames `b2`/`b3` Browser read & write cards + unit tests), `ui/settings/src/{lib.rs,settings.rs,settings_browser.rs (new)}` (frame `b1` Settings › Browser + unit tests). Note: `29-settings` snapshot will need re-recording because the Settings sidebar now includes the `Browser` row after `Agent access`.
  - Desktop native tests: `desktop/tests/{native.rs,native/browser_settings.rs (new)}`.
- Targets to build & test:
  - `//apps/humata2/daemon/browser_copilot:browser_copilot_test`
  - `//apps/humata2/daemon/mcp:all`
  - `//apps/humata2/daemon/agent_hooks:mcp_test`
  - `//apps/humata2/daemon/sessions_core:all`
  - `//apps/humata2/daemon/svc_inbox:svc_inbox_unit_test`
  - `//apps/humata2/daemon/svc_mcp:svc_mcp_unit_test`
  - `//apps/humata2/daemon/humatad:humatad2_unit_test`
  - `//apps/humata2/daemon/humatad:browser_host_test`
  - `//apps/humata2/daemon/humatad:browser_mcp_test`
  - `//apps/humata2/clients:clients_unit_test`
  - `//apps/humata2/clients/live_daemon:live_daemon_unit_test`
  - `//apps/humata2/features/notify:all`
  - `//apps/humata2/features/inbox:all`
  - `//apps/humata2/features/settings:all`
  - `//apps/humata2/features/app:every_action_test`
  - `//apps/humata2/ui/settings:settings_unit_test`
  - `//apps/humata2/ui/workspace:workspace_unit_test`
  - `//apps/humata2/desktop:native_test`
- Remote (D891) — **done, awaiting build**:
  - Forwarded call table & host dispatch: `daemon/svc_away/src/registry.rs` (`GetBrowserBridgeStatus`, `ExecuteBrowserAction`), `daemon/svc_board/src/{ports.rs,replay.rs}` (`HostBrowser` in `HostCalls`), `daemon/humatad/src/{lib.rs,plugins.rs,wire_board.rs,serve.rs}`.
  - Machine-role bridge: `daemon/humatad/src/browser_service.rs` (`BrowserBridgeOnMachine`: 4-agent scope, local `consume_browser_approval` after reachability check, Uplink forwarding for `GetBrowserBridgeStatus`/`ExecuteBrowserAction`, `FailedPrecondition` host-only refusal for `ConnectBrowserHost`/`ListDetectedBrowsers`/`InstallBrowserHost`/`UninstallBrowserHost`).
  - Host-side mirrored approval & race-free replay: `daemon/svc_inbox/src/inbox/replay.rs` (records 30-min read grants on replayed `AllowFor30Min`) and `daemon/svc_inbox/src/inbox/browser.rs` (waits on mirrored pending watch channel when `ExecuteBrowserAction` races ahead of `JOURNAL_ANSWER`, plus unit test).
  - Tests: `daemon/humatad/tests/browser_mcp.rs` (two-daemon `remote_rig` test proving remote `codex` session reads/clicks host Mac's browser with Inbox machine attribution, 30-min read grant reuse, and strict per-action write), `daemon/humatad/tests/away_forward.rs` (machine-role host-only & 4-agent scope checks), `daemon/humatad/BUILD.bazel` (`TEST_NEEDS["browser_mcp"]`).
- Next: build & test Lanes A/B/C + Remote (D891), update `29-settings` snapshot if needed.

**Workers:** Fable frames (running). No others spawned.
