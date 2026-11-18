# WIRE: every step that works on fakes works against the real daemon

- **Goal:** each flow step tagged "Fakes only" in `apps/humata2/flows/*.md` and `flows/cases/` runs in the app against a real `humatad2`, and is retagged "Works" with the test that proves it.
- **Why now:** opened 2026-11-16 because the owner wanted more in parallel. 571 case steps are Fakes only, and most are a finished UI waiting on a live client or a daemon RPC that already exists.
- **Lanes:**
  - WIRE-A: flows 01–05 (Slack ping → Desk, bug fix channel, on-call, proposal doc, long-running project) and their cases.
  - WIRE-B: flows 06–09 (PR review, after a reboot/upgrade, stop a machine, SSH terminal) and their cases.
- **Exit:** every Fakes-only step in the lane's flows is either Works, with a live test named in the step (clients/live, desktop live_flows or humatad), or Not built with a milestone named. The flow and case files are retagged in the same script.
- **Pointers:** flows/README.md (tag rules); clients/live/tests/support (the live rig); desktop/tests/live_flows; clients/src/daemon.rs (fake vs live seam); OVERVIEW.md; GAPS_REVIEW.md.
- **Out of scope:** steps owned by an open milestone's lane (TERM, ROAM, PR, SAFE, STOP, UPGRADE, VIEWS); leave those to that lane and note it in the step. Not-built features. Goldens.
- **Depends on:** nothing open. It can't change the protos without a SEAM hunk.

## Lane (written by the manager)

### WIRE-A (flows 01–05 and cases/macro/01–05), minion WIRE-A, 2026-11-16

Inventory: Fakes-only steps in flows 01: 8, 02: 9, 03: 5, 04: 7, 05: 9; macro cases 01: 11, 02: 14, 03: 14, 04: 4, 05: 10. Grouped by the missing live piece:
- G1 Desk screens: the client was live; the app never read the Desk again after a change (D380). Live tests `live_flows/desk.rs`.
- G2 New channel sheet: the rig's fake gh listed no repos. World's fake gh now lists the repos the world makes. `live_flows/new_channel.rs`.
- G3 Undo after archive: `restore_session` was a `later` stub; now RestoreSession. Parity case + `live_flows/lifecycle.rs`.
- G4 Fork dialog: client live; `live_flows/lifecycle.rs`.
- G5 Board clicks: `live_boards.rs` (BOARD's file, additive).
- G6 Inbox Always allow + Home Needs you: `live_inbox.rs` (additive). Always allow is offered on an ask-ahead only.
- G7 Channel page (palette, Pin, Sessions tab): `live_flows/channel_page.rs`.
- G8 daemon-client `set_pinned`/`set_vip`: now SetMarks. `clients/live/tests/marks.rs`.
Left to lanes: TERM (text and image paste in a session tab), ROAM (move dialogs), STOP (Stop all). WIRE-B: `reconnect`, `mark_read` (same file `clients/live/src/daemon.rs`; `mark_read` is called on opening a recovered session, flows 07).
Later WIRE-A batch: machines add dialog (rig uses FakeMachines; needs a fake ssh), the queue (flows/05 step 6), a picker on another machine, ⌥⌘M, the board drawer after a conflict.

Batch 2 (tests only, no wiring change):
- G9 Machines dialog: `desktop/tests/live_machines.rs`, its own target `live_machines_test` with real LiveMachines and a fake machine behind a fake `ssh` (kept off World's API). Covers the SSH add with the host key card (flows/05 step 9, macro/05 step 12), the new-session picker on box-1 (macro/03 step 6), and an offline box that doesn't tick (macro/02 step 8). SEAM: `fake_machine` visibility for `//apps/humata2/desktop`.
- G10 Queue: `live_flows/queue.rs` (flows/05 step 6). Typing an ask into the queue is Not built (D381).
- G11 Move key: `live_flows/keys.rs` (macro/05 11a). macro/03 step 2 retagged to `live_flows/desk.rs`.
- G12 Document page clicks: `live_flows/document.rs` on a channel session (flows/04 steps 3 to 8). On the Desk it is Not built: WIRE (D382, a `SessionFolder` file root, proto SEAM).
Still open for WIRE-A: the provider and login paths of the machines dialog, starting a session on box-1 (the fake machine has no agent), the board drawer after a conflict, the fork kinds other than Code + context, the Desk file root once main takes D382.

Batch 2 scripts: wire_a_6_live_machines, wire_a_7_live_queue_keys_doc (after 3), wire_a_8_retag (after 5, once the gate is green), wire_a_9_docs (after 4).
Claims added: desktop/tests/live_machines.rs, desktop/BUILD.bazel (new target), live_flows/{queue,keys,document}.rs and their mod lines.

Batch 3:
- G13 Desk document root (D382, main took the SEAM): `SessionFolder` in files.proto (field 9), served in `files/roots.rs` as the session's clone root; the live client reads an empty-repo file through it. Tests: `daemon/humatad/tests/desk_folder_files.rs`, `clients/live/tests/desk.rs::a_desk_sessions_document_is_read_with_no_repository`, `live_flows/desk_document.rs` (flows/04 steps 3 to 8 on the Desk).
- G14 Fork kinds: `live_flows/fork_kinds.rs` (Code only, Context only, before a turn, a refused fork and its retry).
- G15 Board drawer after a refusal: `live_flows/board_conflict.rs`. A stale-revision save from the app is Not built (D383).
- G9 continued: the machines dialog's provider and login paths and a session on box-1, in `live_machines.rs`.
Follow-ups left: SessionFolder in the MCP files tools (`daemon/mcp/src/tools.rs` root schema, `daemon/humatad/src/mcp.rs`) and the CLI's locations; a Desk-folder pane in the file manager (`FileRoot::SessionFolder` in `clients/src/file_manager.rs`).
Batch 3 scripts: wire_a_10_session_folder (after TERM j1), wire_a_11_live_forks_board_deskdoc (after 6, 7), wire_a_12_live_machines_more (after 6), wire_a_13_retag (after 8, once green), wire_a_14_docs (after 9).
Batch 4:
- G15 continued, a stale drawer move (D383 reversed): `Item.revision`, `Daemon::move_item(.., seen)`, `DaemonError::Conflict`. Tests: `clients/live/tests/boards.rs`, `features/board/tests/drawer.rs`, `live_flows/board_revision.rs` (an agent session runs the real `humata` CLI and moves the item behind the drawer).
- G13 follow-ups (D385): `session_folder` in the MCP files tools, `<machine>:folder/<session>/<path>` in the CLI, `FileRoot::SessionFolder` in the file manager and `DeskAction::OpenFolder`. Tests: `daemon/humatad/tests/mcp_desk_folder.rs`, `cli_files.rs`, `cli/tests/files.rs`, `clients/live/tests/file_manager.rs`, `features/files/tests/manager_panes.rs`, `features/app/tests/desk.rs`. No visible control yet (goldens).
- G16 real input in flows/04 steps 3 and 4: a drag finds a zone drawn only while dragging (`testing/flow/tests/drag.rs`); a wheel over the page unticks Link scrolling (D384). `live_flows/document.rs` and `desk_document.rs` drag the tab and scroll by hand.
Batch 4 scripts: wire_a_15_board_revision, wire_a_16_session_folder_rest, wire_a_17_drag_scroll (each after 1-14), wire_a_18_retag (after 13, once green), wire_a_19_docs (replaces 4, 9 and 14).

Batch 1 scripts (scratchpad wire-a/): wire_a_1_restore_marks, wire_a_2_desk_reload, wire_a_3_live_tests, wire_a_4_docs (this), wire_a_5_retag (after the gate is green).
Claims: clients/live/src/daemon.rs (restore/pin/vip fns only), features/app/src/desk.rs, features/app/src/route.rs (navigate/switch), desktop/tests/live_flows/{desk,new_channel,lifecycle,channel_page}.rs, live_flows/world.rs (fake gh), live_boards.rs and live_inbox.rs (additive), flows/01–05 and cases/macro/01–05 tags.

Batch 4 rebased (2026-11-16): wire_a_15r, 16r, 17r replace 15, 16, 17. Flows retags: wire_a_13r (after 2 and 15r–17r, once green) carries 5, 8 and 13 in one script, since the flows files never took 5 or 8; then wire_a_18r. 5, 8, 13 and 18 are retired.
Batch 5 (planned): the clicks still proven only on fakes in flows 01–05, each driven on the live rig.
- G17 Review clicks (flows/02 steps 10, 11, 13, 14): `live_flows/review.rs` clicks the base menu, a diff line, the comment box and **Send 3 comments**, a group's **Push** and **Try again**, and the archive dialog for unpushed work (frame 65), in place of app actions.
- G18 Channel page (flows/03 steps 7 and 9): the palette's Pin and VIP entries, and the Sessions tab's unread-turn marker, filter and fold, in `live_flows/channel_page.rs` (an unread turn comes from a real turn end).
- G19 Forms (macro/01 step 2, macro/02 steps 4 and 10): the Desk form's path preview and repo chip in `live_flows/desk.rs`; the new channel sheet's whole-word order and its progress sheet in `live_flows/new_channel.rs`, the clone held open by the world's fake `git` until the test lets it go (no timing).
Left to lanes: paste into a session tab (TERM), Stop all (STOP), the move dialogs (ROAM). Claims for batch 5: live_flows/{review,channel_page,desk,new_channel}.rs and world.rs (fake git hold).

Batch 5 (2026-11-16): wire_a_21r drives G17–G19 on the live rig and wire_a_22 retags them. The tests went where the batch could add them without editing other lanes' tests: the review clicks are appended to `live_flows/review.rs` (claimed from PR, additive), the channel page's marks and Sessions tab are in a new `live_flows/channel_marks.rs`, the forms in new `live_flows/desk_form.rs` and `live_flows/new_channel_sheet.rs`. The world's held clone is a FIFO the test writes, not a timer. The live preview never showed a taken title's suffix: the Desk form compared the daemon's full folder path with its own `~/Humata/desk/<name>`; fixed in `features/desk` (D386).
Batch 6 (2026-11-16): wire_a_24, retagged by wire_a_25.
- G21 VIP and pin reach the session (flows/03 step 8): done. The daemon fills `Session.pinned` and `Session.vip` from the marks and tells a change as a Session event (D387).
- G22 Per-repo Push & PR by click (flows/02 step 13): done. Each group's button has the accessible name `Push & PR: <repo>`; the drawn text, and so the goldens, are unchanged (D388). `live_flows/review.rs` pushes two repos, each by its own button.
- Push, not poll: `live_flows/channel_marks.rs` waits on turn ends from the daemon (turn hooks on). `running_session` in `live_flows.rs` still re-reads the conversation: the live event stream has no terminal output to wait on (D389).
Open:
- G20 Unread marker on a channel's Sessions rows (flows/03 step 10): Not built. The rows draw the state but no unread mark (`ui/src/channel.rs`); the sidebar shows it in bold. Needs a golden; goes to the owner's next decision wave.
- The Sessions tab's filter box has no label the driver can find (`ui/src/channel.rs`), so the live test types it by app action.

Batch 7 (2026-11-16): wire_a_26 and the docs in wire_a_27.
- Push, not poll: `restart.rs` and `fork_kinds.rs` wait on `World::wrote()` and, for a reboot, on the restart report (D435, D436). `running_session` went to WIRE-B's batch 7.
- Remote pin and VIP: kept on the host, proven by `read_marks_remote.rs` (D437). No daemon change.
- Open: no Output event on the feed; a test that needs a session's raw output still reads the conversation file.

### WIRE-B (flows 06–09), batch 1, 2026-11-16

- **Inventory, by missing live piece.** Flow → cases: 06 → macro/06, agents/15, agents/16; 07 → macro/10, macro/15, micro/17, micro/18, agents/12, agents/19; 08 → macro/09, macro/14, agents/11, agents/13, agents/20; 09 → micro/16, micro/19, micro/20.
  - G1, the app's restart surfaces on a real daemon (flow 07 steps 3–5; macro/10 steps 9–11; micro/18 steps 3, 6, B3; agents/19 wording): **done in batch 1**. Live tests in `desktop/tests/live_flows/restart.rs`; the fix tells a restart the first load brings (D390).
  - G2, the terminal tab strip, Terminal session sheet and Open a shell through the app on a real daemon (flow 09 steps 1, 3, 4): **batch 2**, live tests in `desktop/tests/live_flows/terminals.rs` (expected to pass without wiring since D312; any failure is the missing wiring). micro/19 is a stop case, left to STOP.
  - G3, the app showing the merged Inbox when a stopped machine returns (flow 08 step 11): after G2.
  - G4, the Idle after the restart group on a live agent: needs turn hooks in the live rig's scripted agent.
  - G5, `humata login` passphrase on a real daemon (macro/14, agents/11 login): to check against AWAY's tests.
  - Left to other lanes, noted in the step: flow 06 step 5 offer bar and agents/15 → VIEWS; macro/06 Mark reviewed → PR; flow 07 doctor CLI (macro/10, micro/18 B4) → SAFE; start at login, exit code 7 and Answer all (micro/18 step 2, macro/15, agents/12) → UPGRADE; flow 08 steps 1–6 and macro/09, agents/11 exit codes → STOP; flow 09 step 2 drawing, micro/16, micro/20 → TERM.
- **Batch 6 (what flows 06-09 still had Fakes only):** the host keeps a remote session's mark across its restart (D396, store step 30, `read_marks_remote.rs`, store `remote_reports.rs`); a resize repaint leaves a read session read and a repaint with new words marks it (`read_marks.rs`, agent that repaints on SIGWINCH); the desktop live rig links devbox-1 and flow 08 step 11's app side Works (`live_flows/away_box.rs`, D397), with showing a request as answered on devbox-1 Not built.
- **Batch 8 (closes batch 7's pushless waits):** the history index says when it caught a session up (D438, `HistoryIndexed`, `tests/history.rs`) and `find.rs` waits on it; a Desk scratch folder is in the Bin before its row is marked deleted (D440, `tests/desk.rs`), so `desk.rs` looks once and `App::wait_polling` is gone; an upgrade rollback that relaunches by exec keeps `--ready-fd` (D439, `tests/upgrade_cut.rs`); the clone loops in `files.rs`, `document.rs` and `review.rs` are single looks after the agent's first message (D441); `live_inbox.rs`, `live_machines.rs` and `live_boards.rs` park (D442). Still pushless: an open search isn't rerun on `HistoryIndexed`; `tests/history.rs`'s `hits_when` and `upgrade_cut.rs`'s `wait_at_release` 50 ms loops; the 20 ms connect loops; `settle`'s 10 ms yields; `restart.rs` and `fork_kinds.rs` (WIRE-A).
- **Batch 7 (closes batch 6's two push gaps):** `humatad2 serve` says READY=1 on `--ready-fd` and to `NOTIFY_SOCKET` (D398, `daemon/humatad/src/ready.rs`, `tests/ready.rs`); the live rig starts both daemons on it and `reboot` waits on kqueue process exits; the headless waits park until gpui, the host's feed or an agent's conversation write wakes them (D399). Still pushless: the history index and a folder leaving disk (`App::wait_polling`), the humatad tests' own connect loop (`tests/common/mod.rs`, additive-only), the same 100 ms `wait_until` copies in `live_inbox.rs`, `live_machines.rs` and `live_boards.rs`, a few 50 ms loops in `files.rs`, `document.rs`, `review.rs` and `restart.rs`, and `settle`'s 10 ms yields.
- **Push gaps (batch 6, marked `// GAP (push):` in the code):** nothing pushes "the daemon is listening", so the live rig retries a connect to devbox-1's socket (`World::start_with_machine`; `World::spawn_daemon` and `reboot` have the same loop and a `pgrep` loop); and the headless app's state only moves while the test runs gpui's executor, with no change notification, so `away_box.rs`'s `wait_within_link` and the rig's `wait_until` settle, sleep and look again.
- **Batch 5 (read marks on remote sessions, redraws, lag; G3, G4, G5):** remote sessions show their machine's mark on the host and viewing there clears it (D394, `daemon/humatad/tests/read_marks_remote.rs`); a resize or attach redraw never marks (D393, unit tests in `read_marks`); a lag sends every client a Resync (D395, unit tests). G4: the live rig's agent can report its turn, and flow 07 step 4 Idle Works (`live_flows/restart.rs::after_a_reboot_an_agent_that_reports_idle_is_listed_under_idle_after_the_restart`). G5: macro/14 steps 7 and B5, flow 08 step 8 and agents/11 step 12 Work (`away_login.rs`, new `away_login_pipe.rs`, `cli_login.rs`). G3: flow 08 step 11's daemon part Works (`flow_26_10.rs`); the app part stays Fakes only until the desktop live rig has a linked second machine.
- **Batch 4 (real read marks, CH-17):** the daemon marks a session unread when its agent prints while no client views it and keeps the mark across restarts; MarkService.ViewSession (SEAM) views a session and clears it; the live `mark_read` and `view` hold that stream; the fakes do the same (D392, superseding D391). Proven by `daemon/humatad/tests/read_marks.rs`, `daemon/marks/tests/unread.rs`, `clients/tests/read_marks.rs` and the parity cases `unviewed_output_marks_a_session_unread` and `reading_an_unknown_session_is_not_an_error`.
- **Batch 3 (from WIRE-A's note):** the live `mark_read` was a `later` stub, so opening a recovered session showed an error; it now succeeds and sends nothing (D391), proven by `live_flows/restart.rs::opening_a_session_that_couldnt_resume_shows_no_error_and_keeps_its_card`.
- **Batch 1 retags:** flow 07 steps 3–5; macro/10 steps 9–11; micro/18 steps 3, 6, B3; agents/19 card; macro/06 step 1 (already proven live by `live_flows/find.rs`).
