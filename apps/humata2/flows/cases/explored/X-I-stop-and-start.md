# Stop and start: stopping a machine's sessions, the banners, and starting them again

Layer: explored · macOS · harness: desktop/tests/native/stop.rs (frames 72 to 76, headless), desktop/tests/native/session_menu.rs, desktop/tests/live_flows/stop_and_start.rs (real humatad2), cli/tests/stop.rs (fake daemon), features/hosts (Hosts row)

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2.

## Setup

- Native frames: `app_with(...)` in desktop/tests/native/stop.rs. A fake workspace from `fixture::design()`; devbox-1 is the machine, with six sessions (ledger, brin, rate, partial-brin, benchmark, postgres) and three channel folders. The stop is set on the fake machine. Frame 73 and 75 use a stop made "at 14:02, by you, delivered". Frame 76 uses an undelivered stop on an offline devbox-1 ("lost_at 14:02").
- Live: `this_macs_page_stops_its_sessions_only_when_confirmed_and_starts_them_again` in stop_and_start.rs starts a real daemon (World::start), one running session "Cache warmup", opens This Mac's page.
- CLI: cli/tests/stop.rs drives `humata stop` and `humata start` against FakeDaemon. No real daemon.
- Throwaway daemon for PENDING RUN steps: see the last section of X-I-restart-and-link-drop.md (same serve command).

## Steps

### 1. devbox-1's machine page, nothing stopped (the entry point)
- **Screen:** Sidebar lists This Mac, devbox-1, gpu-box, Settings. Document "Machine devbox-1": heading, `Open a shell`, `New session on devbox-1`, `More`. Tabs Overview (selected) and Storage. Overview has Health (Disk 97% used, variable rows), the six session buttons, the folders with Open in Files, the four Agent access rows, `Change in Agent access`. (native/72-stop-confirm.view.txt)
- **Offered:** Open a shell, New session on devbox-1, More (menu), Overview/Storage tabs, Set value…/Remove/New variable…, each session button, Open in Files, Change in Agent access.
- **Action:** Open the page (`AppAction::Switch(Route::Machine(devbox))`).
- **Result:** Page shows; no banner.
- **Evidence:** desktop/tests/__snapshots__/native/72-stop-confirm.view.txt (the page part)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=stop_all_sessions_confirm_reference_image` (the test asserts `machine-overview` is present)

### 2. More, then Stop all sessions… opens the confirm
- **Screen:** The confirm is a Dialog "Stop all sessions" over the page, with buttons Cancel and Stop all sessions. Code text (ui/src/machine_page.rs:1377-1425, the body is not in the a11y tree so it is not in the .view.txt): title "Stop all sessions on devbox-1?"; "Every agent and shell in devbox-1's 6 sessions gets 2 seconds to quit, then is killed."; part "Kept: Scrollback, conversations, todos, rewind points and open tabs."; part "While stopped: Agents can't type, call Humata or be answered. Their open requests end."; the note "devbox-1's sessions stay stopped, even after it restarts, until you choose Start sessions."
- **Offered:** Cancel, Stop all sessions (red border). The menu item is `Stop all sessions…`.
- **Action:** Click More, then `Stop all sessions…`.
- **Result:** The confirm opens. Nothing is sent: the test asserts the daemon still has no stop. No banner yet (`machine-stop-banner` absent).
- **Evidence:** desktop/tests/__snapshots__/native/72-stop-confirm.view.txt (Dialog "Stop all sessions", Cancel, Stop all sessions)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=stop_all_sessions_confirm_reference_image`

### 3. Cancel on the confirm
- **Screen:** Back to the page as in step 1.
- **Offered:** Same as step 1.
- **Action:** Click Cancel.
- **Result:** Dialog closes. The live test asserts `stopped_on_daemon` is false and the session is not Stopped.
- **Evidence:** PENDING RUN (live test only; the native frame does not click Cancel).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=this_macs_page_stops_its_sessions`

### 4. Stop all sessions, confirmed
- **Screen:** The page after the stop. Banner Status "Sessions stopped" with button `Start sessions`. The stop banner's body (ui/src/machine_page.rs:1625, not in the a11y tree): "Sessions stopped by you at 14:02: agents and shells ended, everything else is kept, and they stay stopped after restarts." In the Hosts row the same stop reads "Sessions stopped by {who} at {time}; everything is kept." (ui/src/hosts.rs:336).
- **Offered:** Open a shell, New session on devbox-1, More (no stop option now), Start sessions (banner), the Overview content.
- **Action:** Click Stop all sessions in the confirm.
- **Result:** The daemon holds the stop (`StopSessionsRequest` via the machine page). Live test: the page shows delivered stop, the confirm closes, the daemon reports the session as Stopped. The session's status reads Stopped on the sidebar and in the session row.
- **Evidence:** desktop/tests/__snapshots__/native/73-machine-sessions-stopped.view.txt (Status "Sessions stopped", Button "Start sessions", Dark theme). PNG at desktop/tests/__snapshots__/native/73-machine-sessions-stopped.png.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=machine_page_with_sessions_stopped_reference_image`; live: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=this_macs_page_stops_its_sessions`

### 5. More after the stop
- **Screen:** Same page as step 4.
- **Offered:** More opens the menu without `Stop all sessions…`.
- **Action:** Click More.
- **Result:** The stop item is absent: no second stop from the menu. The live test then closes the menu with `MachinePageAction::CloseMenu`.
- **Evidence:** PENDING RUN (the native frame does not open the menu after a stop).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=this_macs_page_stops_its_sessions`

### 6. Open a shell on the stopped machine
- **Screen:** Page as in step 4, with Status "Start sessions first" shown beside the header (frame 73 view.txt shows Status "Start sessions first" after the buttons). The button Open a shell is still enabled in frame 73.
- **Offered:** Open a shell (enabled in frame 73), New session on devbox-1, More, Start sessions.
- **Action:** Click Open a shell.
- **Result:** A refusal note Status "Start sessions first". No session sheet opens (`state.overlay()` is None).
- **Evidence:** desktop/tests/__snapshots__/native/73-machine-sessions-stopped.view.txt and 75-start-confirm.view.txt (both show Status "Start sessions first")
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=this_macs_page_stops_its_sessions`

### 7. Start sessions (asks first)
- **Screen:** Dialog "Start sessions" over the page. Its body (ui/src/machine_page.rs:1444-1450): "Humata brings back the N sessions{when} as after a restart: agents resume their conversations, idle, and shells open fresh in the same folder." Buttons Cancel and Start sessions. The confirm's accessible name is "Start N sessions" (or "Start 1 session") per `start_name(count)`; the banner button also reads Start sessions. Frame 75 view.txt shows the dialog button as Button "Start sessions" (see Surprises).
- **Offered:** Cancel, Start sessions (confirm). The banner is still behind the dialog.
- **Action:** Click Start sessions on the banner.
- **Result:** Dialog opens. The live test asserts that opening it starts nothing (stop still held).
- **Evidence:** desktop/tests/__snapshots__/native/75-start-confirm.view.txt (Dialog "Start sessions", Cancel, Start sessions; banner still shown)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=start_sessions_confirm_reference_image`

### 8. Confirm Start sessions
- **Screen:** Page with the banner gone; sessions back.
- **Offered:** Open a shell, New session, More (with Stop all sessions… again), the six session buttons.
- **Action:** Click the confirm (`Start N sessions`, N = 6 in the fixture).
- **Result:** The daemon clears the stop. The page drops the banner (no "Sessions stopped"), and the agent no longer reads Stopped. Open a shell works again.
- **Evidence:** PENDING RUN. The live test is the only check (the native fixture stops at the dialog).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=this_macs_page_stops_its_sessions`

### 9. Stop not delivered (devbox-1 unreachable)
- **Screen:** Page with Status "Stop not delivered" and buttons `Reconnect now` and `Cancel stop`. Body (ui/src/machine_page.rs:1635): "Stop not delivered yet: devbox-1 can't be reached, so its sessions may still be running. Humata stops them as soon as it connects again." Open a shell and New session on devbox-1 are disabled. No Start sessions button.
- **Offered:** Reconnect now, Cancel stop, More (menu), Overview/Storage. Open a shell and New session disabled.
- **Action:** None (state only).
- **Result:** The stop waits on the host and is delivered when devbox-1 is back (per the body text).
- **Evidence:** desktop/tests/__snapshots__/native/76-stop-not-delivered.view.txt (Status "Stop not delivered", Reconnect now, Cancel stop, Open a shell disabled)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=stop_not_delivered_reference_image`

### 10. Hosts page: the stopped row
- **Screen:** Hosts with devbox-1 selected. Row shows Status-like text "Sessions stopped" and the row's `Start sessions` button (`host-start devbox-1`). The card has the same button. Banner not shown on the Hosts page (`machine-stop-banner` absent in the test).
- **Offered:** Start sessions (row and card), host reconnect, shell.
- **Action:** Select devbox-1 in Hosts.
- **Result:** The stop is visible in Hosts only as the row's "Sessions stopped" line and the Start button.
- **Evidence:** desktop/tests/__snapshots__/native/74-hosts-sessions-stopped.view.txt
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_page_with_sessions_stopped_reference_image`

### 11. humata stop and humata start (CLI, this machine)
- **Screen:** Terminal output. No screen.
- **Offered:** `humata stop [--reason <text>]` and `humata start`. Both are on the top level. `humata host stop <machine> [--reason]` and `humata host start <machine>` are the machine forms (args.rs:806-816). `humata session stop <id>` stops one session.
- **Action:** `humata stop --reason "agents looping"` (sends DaemonService stop with via=Cli and a new stop_id each invocation, cli/src/commands/mod.rs:111-120).
- **Result:** The CLI asks the daemon once; exit 0 (`exit::OK`). The stop id is new per call, so a retry is a second stop; the code comment says the same id on retry would make it the same stop, but the id is generated inside the command, so a retry from the shell does not reuse it. `humata start` calls START_SESSIONS and exits 0.
- **Evidence:** PENDING RUN (throwaway daemon; see the restart record's setup)
- **Replay check:** `bazel test //apps/humata2/cli:stop_test` (stop_asks_the_daemon_once_from_the_cli_with_the_reason_and_a_stop_id; start_asks_the_daemon_to_start_the_sessions)

### 12. An agent calls humata stop
- **Screen:** The agent's terminal (no page).
- **Offered:** The agent's CLI call only.
- **Action:** Agent runs `humata stop --reason ...` with its session token.
- **Result:** Denied with `exit::DENIED` (4) and a fix string when the rule says no (cli/tests/stop.rs `an_agents_stop_denied_by_its_rule_exits_denied_with_the_fix`). Asks with `exit::NEEDS_USER` (5) when the user must answer (`a_start_refused_for_want_of_the_user_exits_needs_user`).
- **Evidence:** cli/tests (no screen)
- **Replay check:** `bazel test //apps/humata2/cli:stop_test`

## Not reachable headless

- The stop dialog body text and the banner body text are not in the a11y `.view.txt`; they come from the code, not from a snapshot. Only the PNGs (72 to 76) show them. The PNGs are viewable with the Read tool at desktop/tests/__snapshots__/native/72-stop-confirm.png (and 73 to 76).
- Real stop on a real remote machine (SSH devbox) is in `away_box.rs` and `remote.rs` and is not part of this record.
- The stop's effect on the agents (2 seconds to quit, then killed; kept state after a reboot) is in humatad2 tests (`daemon/humatad/tests/stop_sessions.rs`), which this explorer did not run.
- The design-golden frames 72 to 76 are the same screens as the native snapshots; they are drawn in desktop/tests/design.rs and are not compared here.

## Surprises

- The Start sessions confirm: frame 75's a11y dump shows its button as `Button "Start sessions"`, the same name as the banner button. The code sets `accessibility_label(start_name(count))` (ui/src/machine_page.rs:1462-1470) so it should read "Start N sessions", and the live test clicks "Start N sessions". Either the snapshot predates that change or the label is not applied in the native render. Check in phase 2 (see the PENDING RUN in step 8 and the native run in step 7).
- Open a shell is enabled after a delivered stop (frame 73), but the same button is disabled when the stop is not delivered (frame 76). Clicking it after a delivered stop shows "Start sessions first".
- `humata stop` from a terminal runs at once with no confirm. The machine page asks first. The CLI help says "Stop every session on this machine, and keep them stopped until `humata start`." It does not mention that no confirm is asked.
- The CLI's stop id is made fresh inside each `humata stop` call (cli/src/commands/mod.rs:113), and its comment says a retry with the same id is the same stop. A person who retypes the command gets a second stop.
- The stop dialog says the stop "stays stopped, even after it restarts". The banner body says the same. The gap list records that the restart report does not name a stop the user made (flows/cases/README.md item 30).
- Frame 76 hides the Start sessions button while the stop is not delivered; the only actions are Reconnect now and Cancel stop. A person who wants the sessions back at once while devbox-1 is offline has to cancel the stop first.
