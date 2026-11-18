# A session moves to another machine with its work and conversation, and a failed move leaves it where it was

Layer: agents (with the user's palette steps at the start). Platforms: macOS and Linux. The palette steps are macOS only. Web is not covered.

## Starting state

- Channel `Payments` has three machines: this Mac, `box-1` and `box-2`. Its repo is `acme/api`.
- The session `Cache warmup` runs on `box-1`. Its agent is up and its shell tab is open.
- In the session's clone there is one unpushed commit, one edited `README.md` and one untracked `PLAN.md` beside the clone.
- The session's conversation holds one line I typed earlier.
- `box-2` is online and in the channel.
- I am in the session's agent tab, and the palette is closed.

## Steps

1. **I do:** press ⌘K and type `move`.
   **I see:** "Move session" is the first entry, and its hint shows ⌥⌘M.
   **State and events:** the palette knows the session beneath it. No route changes.
   **Status:** Works (real daemon): `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (checks the first entry and its shortcut against a real `humatad2`).

2. **I do:** press Return.
   **I see:** the palette asks for a machine. Only machines of this channel that are not the session's own are listed. `box-1` is not offered.
   **State and events:** the palette stays open with its question. Nothing moves yet.
   **Status:** Works (real daemon): `find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (same test, the "asking" step). The online-only filter is Fakes only: `features/palette/tests/act.rs::running_move_asks_inline_for_another_online_machine_of_the_channel`.

3. **I do:** press Esc.
   **I see:** the palette is back on the query `move`, with "Move session" first. The session stays open.
   **State and events:** Esc steps back once. The route is still the session.
   **Status:** Works (real daemon): `find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`.

4. **I do:** open the Move dialog with ⌥⌘M, pick `box-2` and press Confirm.
   **I see:** the dialog closes. The session shows Moving on the daemon's side. The app shows no "Moving to box-2…" line (see the branch on MV-4).
   **State and events:** the app sends `MoveSession` with the session id, `box-2` and a fresh idempotency key.
   **Status:** Fakes only: `features/app/tests/move_from_palette.rs::moving_a_channel_session_from_the_palette_asks_the_daemon` and `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine`. Both run against a fake daemon. The live app test stops before the move (step 1 to 3 only), so nothing shows that the real app sends this call.

5. **I do:** watch the move from the session's page.
   **I see:** the operation ends Succeeded. The daemon records eight steps in this order: check, pause, pack, copy, verify, resume, switch, release. The app does not list them.
   **State and events:** the session's machine becomes `box-2`. The operation's steps reach Done in that order.
   **Status:** Works (real daemon): `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` (asserts Succeeded). The step order itself is not asserted. The test checks only the copy and verify steps by key in other tests.

6. **I do:** while the move holds after the copy, type `typed while moving` and Enter in the session's terminal.
   **I see:** the terminal is read-only. The terminal state reports Moving. The typed line never reaches the agent. The pane has no "read-only, moving" text (see the MV-4 branch).
   **State and events:** the stream's `writable` is `Moving`. The conversation file does not hold the typed text.
   **Status:** Works (real daemon): `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes`, for a session on this Mac and for one on `box-1`.

7. **I do:** after the move, open the session's record and its clone on `box-2`.
   **I see:** the same id, title and channel. The branch is the same. The unpushed commit, the dirty `README.md` and `PLAN.md` are there.
   **State and events:** the record keeps its id, title, channel and branch. Its work moves with it.
   **Status:** Works (real daemon): `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`.

8. **I do:** type `still here` in the agent tab on `box-2`.
   **I see:** the agent answers. Its conversation file on `box-2` holds my line. Nothing runs on `box-1`.
   **State and events:** one agent tab exists, on `box-2`. The holder answers there.
   **Status:** Works (real daemon): `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` (the `say` step and the tab checks).

9. **I do:** check `box-1`'s folder for the session.
   **I see:** the folder is gone from `box-1`'s session folder. A copy sits in `box-1`'s trash. `box-2` keeps no staged files.
   **State and events:** the old copy is in the old machine's trash. The journal and staging folder on `box-2` are empty.
   **Status:** Works (real daemon): `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`.

10. **I do:** move a session from this Mac to `box-1`.
    **I see:** after the move ends, the folder on this Mac is gone and the agent runs on `box-1`.
    **State and events:** the Mac's folder is freed only after the session runs on `box-1`. The code runs `release` after `switch`.
    **Status:** Code, no test for the ordering. The end state is tested: `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` checks the folder after the move ends. It does not check that the folder is still there while the agent runs on `box-1`, so the test name claims more than it asserts.

11. **I do:** have a shell tab open in the session on `box-1`, with output on screen, then move the session to `box-2`.
    **I see:** on `box-2` a new shell opens in the session's folder. It starts with the saved output and then "This is a new shell in…". The shell on `box-1` stops.
    **State and events:** one shell tab on `box-2`. None left on `box-1`.
    **Status:** Works (real daemon): `daemon/humatad/tests/move_session.rs::a_moved_session_s_shell_tab_comes_back_on_the_new_machine_with_its_output`.

12. **I do:** have the agent write two conversation lines, one before the move and one after.
    **I see:** the host's mirror of the conversation holds each line once.
    **State and events:** the moved conversation is the new machine's file. The host's mirror adds only lines after it.
    **Status:** Works (real daemon): `move_session.rs::a_moved_conversation_is_mirrored_once`.

13. **I do:** from inside the session, the agent runs `humata session move <session> --to box-2`.
    **I see:** the CLI does not finish at once. The user gets a request in the Inbox, and the agent's call waits for the answer.
    **State and events:** an Inbox request for `session_move` appears. The session does not move. The CLI reports a pending request (exit 12).
    **Status:** Code, no test. The CLI sends `MoveSession` (`cli/src/commands/sessions.rs:246`), and the daemon gates it on `SessionMove` (`daemon/humatad/src/session_service.rs:2244`, `session_service/move_session.rs:182`). The policy default is Ask me (`daemon/access/src/capability.rs:220`). No test calls the move as an agent. Every move test calls as the user.

14. **I do:** answer Allow on the request.
    **I see:** the move runs. The Inbox request is marked Approved.
    **State and events:** the approved call runs `move_session_of` with `approved = true`. Its steps match step 5.
    **Status:** Code, no test. The approval path exists (`session_service.rs:2244` and `:2253`). No test covers an agent's move after Allow.

15. **I do:** look for an "Always allow" option on that request.
    **I see:** the card offers Allow and Deny. It offers no "Always" for a move, because the capability names no target.
    **State and events:** no grant is made for `SessionMove`.
    **Status:** Fakes only: `daemon/access/tests/grants.rs::nothing_offered_means_no_always_let` and `grants.rs::asking_ahead_for_something_with_no_always_let_is_refused`. These are policy unit tests with no daemon.

16. **I do:** the agent calls the MCP tool `session_move` with `machine` set to `box-2`.
    **I see:** an error: "session_move isn't built into this humatad2 yet". Its fix says to ask the user with `inbox_ask`.
    **State and events:** no operation starts. The MCP tool list still advertises `session_move`.
    **Status:** Not built (SPEC MV-1, agent path; no milestone). `daemon/humatad/src/mcp.rs:1064` falls through to `not_built`, and `daemon/mcp/src/tools.rs:1153` advertises the tool.

## Branches

### B1. The copy fails because `box-2` can't write the session folder

**When:** the session folder on `box-2` is read-only, so the copy step fails.
**I do:** move the session to `box-2`.
**I see:** the operation ends Failed. The error names the failed step and its fix, and says nothing was deleted.
**Expected:** the session stays on `box-1`, with the same agent, its work untouched and the clone at the same commit. `box-2` keeps nothing.
**Status:** Works (real daemon): `move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step`.

### B2. The target is offline, outside the channel, or is the session's own machine

**When:** `box-2` is offline, or `box-2` is not in the channel, or I pick `box-1` for a session already on `box-1`.
**I do:** ask for the move.
**I see:** an error at once. Offline and outside-channel return FailedPrecondition. Same-machine returns InvalidArgument.
**Expected:** nothing changes. The session runs on `box-1` with one agent.
**Status:** Works (real daemon): `move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing`.

### B3. The same move is asked again with the same key

**When:** the app retries, or the CLI is run twice with one key.
**I do:** send the same `MoveSession` twice.
**I see:** both calls return the same operation id.
**Expected:** the session moves once, and `box-2` has one agent tab.
**Status:** Works (real daemon): `move_session.rs::asking_the_same_move_again_answers_the_same_operation`.

### B4. A move is cancelled before the agent stops

**When:** I cancel while the move is at the copy step, before the old agent stops.
**I do:** cancel the operation.
**I see:** the operation ends Cancelled.
**Expected:** the session runs on `box-1` as before. `box-2` keeps no copy and no agent.
**Status:** Works (real daemon): `move_session.rs::a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused`.

### B4b. A cancel after the agent has stopped is refused

**When:** I cancel once the old agent has stopped.
**I do:** cancel the operation.
**I see:** FailedPrecondition, and the move finishes.
**Expected:** the session ends on `box-2`, running.
**Status:** Works (real daemon) for both B4 and B4b: `move_session.rs::a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused`.

### B5. The host is killed before the agent stops, and restarts

**When:** the host dies at the start, after packing, after the copy, or after the copy is taken in, and restarts.
**I do:** restart the host.
**I see:** the session is back on `box-1`, with its folder, and the same agent. `box-2` keeps nothing.
**Expected:** the move is undone and moving again works.
**Status:** Works (real daemon): `move_session.rs::a_move_cut_short_as_it_starts_is_undone_at_restart`, `a_move_cut_short_once_packed_is_undone_at_restart`, `a_move_cut_short_after_the_copy_is_undone_at_restart`, `a_move_cut_short_once_the_copy_is_taken_in_is_undone_at_restart`. A folder of the same name already on `box-2` is left alone: `a_move_cut_short_as_it_starts_leaves_a_folder_already_on_box_2_alone`.

### B6. The host is killed after the agent has moved, and restarts

**When:** the host dies once the agent runs on `box-2`, at the resume, switch or release step.
**I do:** restart the host.
**I see:** the session is on `box-2` with one agent. `box-1`'s copy is in its trash.
**Expected:** the move finishes. Never two agents.
**Status:** Works (real daemon): `move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`.

### B7. A power cut at each step of a move

**When:** the host's power is cut, or the box's, or both, at each fault point the move reaches.
**I do:** cut the power and start again.
**I see:** each item is as it was, or finished, or named in a restart report.
**Expected:** nothing the move holds is lost.
**Status:** Works (real daemon): `daemon/humatad/tests/power_cut_host.rs::a_move_with_the_hosts_power_cut_loses_nothing_part_1` and `_part_2`, `power_cut_machine.rs::a_move_with_a_boxs_power_cut_loses_nothing_part_1` and `_part_2`, `power_cut_both.rs::a_move_with_both_machines_power_cut_loses_nothing_part_1` and `_part_2`. These cover each point in chunks; the test file does not list the points.

### B8. The restart report names the cut move

**When:** B5 or B6 happens.
**I do:** open the restart report after the restart.
**I see:** one line for the move, saying "undone" for a cut before the switch and "finished" for a cut after it.
**Status:** Works (real daemon): `daemon/humatad/tests/restart_operations.rs::a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended`.

### B9. A stopped session stays stopped when its move is undone

**When:** I stopped the session before the move, and the move is cut and undone.
**I do:** restart.
**I see:** the session is still stopped. No agent starts on `box-1` or `box-2`.
**Status:** Works (real daemon): `move_session.rs::a_stopped_session_whose_move_is_undone_at_restart_stays_stopped`.

### B10. The target's link is down while the move is undone

**When:** `box-2` is offline while the move is undone, so a copy stays there.
**I do:** bring `box-2` back.
**I see:** the copy and its record are taken back. The session runs on `box-1` at once.
**Status:** Works (real daemon): `move_session.rs::a_copy_left_on_an_offline_target_is_taken_back_when_its_link_is_up`.

### B11. A second move is asked while one runs

**When:** a move is running, and I ask for another move, or an archive or restore of the same session.
**I do:** ask for a second action on the session.
**I see:** the second call is refused as busy: "being moved, archived or restored".
**Expected:** the first move is not disturbed.
**Status:** Code, no test. The refusal is at `daemon/humatad/src/session_service/move_session.rs:231`. No test asserts it.

### B12. An agent's move is asked and the user says Deny

**When:** step 13 happens, and I answer Deny on the request.
**I do:** answer Deny.
**I see:** the agent's CLI call ends denied. The session does not move.
**Status:** Code, no test. The denial path is the generic Inbox answer. No test covers a denied move.

### B13. The palette shows "Moving to box-2…" with the step in progress (MV-4)

**When:** the move runs, and I look at the session's page.
**I do:** look at the session's header and its pane.
**I see:** the session shows Idle. No "Moving to box-2…" line, and no step in progress.
**Expected (per SPEC MV-4):** the session shows "Moving to <machine>…" with the step in progress, and the terminal says it is read-only.
**Status:** Works (real daemon): `daemon/humatad/tests/move_typing.rs::typing_into_a_moving_session_is_refused_and_works_once_it_resumes_on_the_new_machine` reads the session as moving to its target with the running step, and `clients/live/src/map.rs::a_moving_session_reads_moving_to_its_target_with_the_running_step` maps that to Moving, which the tab and page draw as "Moving to …" (`ui/src/pages.rs`, `ui/src/session_tabs.rs`). The step-by-step MV-4 text is still Not built (SPEC MV-4, no milestone). (Was a likely bug: Moving mapped to Idle; fixed by BUGS.)

### B14. A Desk session is asked to move

**When:** I move a Desk session (one with no channel), or a channel's Desk session, to another machine.
**I do:** use the Desk bar's machine picker, or `humata desk move`.
**I see:** the picker answers "not in this release", and `humata desk move` does not exist. The daemon refuses a `MoveSession` on a Desk session with FailedPrecondition.
**Expected (per SPEC DK-3):** the Desk session moves to any online machine.
**Status:** Not built (SPEC DK-3 and MV-17 in `workflow/milestones/ROAM.md`; D182.2 in `DECISIONS.md`). Evidence: `clients/live/src/desk.rs:453`, `cli/src/args.rs:321` (only `desk keep`), `daemon/humatad/src/session_service.rs:2235` with `desk_service.rs:441`.

## Expected outcomes

- [ ] The palette offers "Move session" first, with ⌥⌘M, inside a session.
- [ ] A move asks for a machine of the channel, and Esc steps back.
- [ ] The move keeps the session's id, title, channel, branch, unpushed commit, dirty files and conversation.
- [ ] The agent resumes on the new machine, and nothing runs on the old one.
- [ ] The old folder goes to the old machine's trash after the session runs on the new one.
- [ ] While the move holds, the terminal refuses input, and nothing typed reaches the agent.
- [ ] A failed copy leaves the session on its old machine and names the step.
- [ ] An offline or outside-channel target is refused, and nothing changes.
- [ ] The same key gives the same operation.
- [ ] A cut at any step is undone or finished at restart, and never leaves two agents.
- [ ] A stopped session stays stopped.
- [ ] An agent's move waits for the user, and no "Always" is offered.
- [ ] MCP `session_move` says it is not built.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Palette shows Move session first with ⌥⌘M | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` |
| 2. Palette asks for a machine | Works (real daemon) | `find.rs` (same test); online filter is Fakes only: `features/palette/tests/act.rs::running_move_asks_inline_for_another_online_machine_of_the_channel` |
| 3. Esc steps back | Works (real daemon) | `find.rs` (same test) |
| 4. Confirm sends MoveSession | Fakes only | `features/app/tests/move_from_palette.rs::moving_a_channel_session_from_the_palette_asks_the_daemon`; `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine` |
| 5. Eight steps run; session on box-2 | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` (step order not asserted) |
| 6. Terminal read-only while moving | Works (real daemon) | `move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` |
| 7. Id, branch, work, conversation kept | Works (real daemon) | `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 8. Agent resumes on box-2 | Works (real daemon) | `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 9. Old copy in trash; nothing staged | Works (real daemon) | `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 10. Mac folder freed only after it runs on box-1 | Code, no test (ordering) | End state only: `move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| 11. Shell tab comes back with output | Works (real daemon) | `move_session.rs::a_moved_session_s_shell_tab_comes_back_on_the_new_machine_with_its_output` |
| 12. Conversation mirrored once | Works (real daemon) | `move_session.rs::a_moved_conversation_is_mirrored_once` |
| 13. Agent's CLI move asks the user | Code, no test | `cli/src/commands/sessions.rs:246`; `session_service.rs:2244` |
| 14. Agent's approved move runs | Code, no test | `session_service.rs:2253` |
| 15. No "Always" for a move | Fakes only | `daemon/access/tests/grants.rs::nothing_offered_means_no_always_let` |
| 16. MCP session_move | Not built | SPEC MV-1 (agent path, no milestone); `daemon/humatad/src/mcp.rs:1064` |
| B1. Copy fails, stays on source | Works (real daemon) | `move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` |
| B2. Offline, outside channel, same machine | Works (real daemon) | `move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing` |
| B3. Same move asked again | Works (real daemon) | `move_session.rs::asking_the_same_move_again_answers_the_same_operation` |
| B4. Cancel before agent stops | Works (real daemon) | `move_session.rs::a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused` |
| B4b. Cancel after agent stops refused | Works (real daemon) | `move_session.rs::a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused` |
| B5. Cut before agent stops, at restart | Works (real daemon) | `move_session.rs::a_move_cut_short_as_it_starts_is_undone_at_restart`, `a_move_cut_short_once_packed_is_undone_at_restart`, `a_move_cut_short_after_the_copy_is_undone_at_restart`, `a_move_cut_short_once_the_copy_is_taken_in_is_undone_at_restart` |
| B6. Cut after agent moved, finished at restart | Works (real daemon) | `move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart` |
| B7. Power cut at each step | Works (real daemon) | `daemon/humatad/tests/power_cut_host.rs::a_move_with_the_hosts_power_cut_loses_nothing_part_1`; `power_cut_machine.rs` and `power_cut_both.rs` (same pattern) |
| B8. Restart report names the move | Works (real daemon) | `daemon/humatad/tests/restart_operations.rs::a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended` |
| B9. Stopped session stays stopped | Works (real daemon) | `move_session.rs::a_stopped_session_whose_move_is_undone_at_restart_stays_stopped` |
| B10. Copy left on offline target taken back | Works (real daemon) | `move_session.rs::a_copy_left_on_an_offline_target_is_taken_back_when_its_link_is_up` |
| B11. Second move while one runs | Code, no test | `daemon/humatad/src/session_service/move_session.rs:231` |
| B12. Agent's move denied | Code, no test | Generic Inbox answer; no move test |
| B13. "Moving to box-2…" line (MV-4) | Works (real daemon) | `daemon/humatad/tests/move_typing.rs::typing_into_a_moving_session_is_refused_and_works_once_it_resumes_on_the_new_machine`; `clients/live/src/map.rs::a_moving_session_reads_moving_to_its_target_with_the_running_step` |
| B14. Desk session move | Not built | SPEC DK-3, MV-17 (ROAM); D182.2; `clients/live/src/desk.rs:453`; `cli/src/args.rs:321` |

## Gaps and likely bugs

- Not built: MCP `session_move` (SPEC MV-1, agent path, no milestone). The tool is advertised in `daemon/mcp/src/tools.rs:1153` and answers not_built at `daemon/humatad/src/mcp.rs:1064`.
- Not built: Desk session move, with `humata desk move` and the picker (SPEC DK-3; ROAM MV-17; D182.2). `clients/live/src/desk.rs:453` answers "not in this release".
- Not built: the MV-4 line "Moving to <machine>…" and the step progress in the app (SPEC MV-4, no milestone). `ui/src/overlay_move.rs` shows no steps.
- Fixed (was a likely bug): a moving session reads Moving with its target (`clients/live/src/map.rs::a_moving_session_reads_moving_to_its_target_with_the_running_step`).
- Test gap: `desktop/tests/live_flows/find.rs` does not finish the move. It stops at the machine question and steps back, so the real app's move call is unproven.
- Test gap: no test runs a move as an agent, so the Ask, Allow and Deny paths for `humata session move` are Code, no test.
- Test gap: the ordering "old folder freed only after the new agent runs" is not asserted while the move is held.
- Test gap: the busy refusal (`move_session.rs:231`) has no test.
- All daemon tests run `humatad2` with fake boxes (folders in the rig) and a fake agent. A real SSH box and a real Claude run are not covered by these tests.

## Corrections to the brief

- `daemon/humatad/tests/move_session.rs` has 17 tests, not 18.
- The brief's "plan" step does not exist. A move has no preview RPC. Its first step is `check` (`move_session.rs:50`).
- `desktop/tests/live_flows/find.rs` does not move a session. It asserts that the palette asks for a machine and steps back.
- MCP `session_move` is advertised in `tools/list` (`daemon/mcp/src/tools.rs:1153`), so an agent sees it and then gets not_built.
