# A power cut mid-work leaves each operation undone or finished, and the restart report says which

Layer: macro. Platforms: macOS (this Mac, the host) and a linked box (box-1).

Status tags in this file: a test that runs the real `humatad2` is tagged
**Works (real daemon)**. The power cuts in these tests are simulated: the
daemon is stopped at a fault point, and writes it made through
`humata2_durable` that were not synced are undone first. Writes that bypass
`humata2_durable` are not undone, so the tests prove less for those. SAFE.md
records that the power-cut suite (T5) was written but not yet compiled or
gated when it was last updated (2026-11-16). Treat the power-cut rows as
"test exists, not yet proven green" until a gate runs it. A step or branch takes
the tag of its main claim; the Evidence column names any part that is only faked
or not built.

## Starting state

- This Mac runs the host daemon. Channel "Payments" has a session "Cache
  warmup" running an agent on box-1, with work in its folder.
- A code-and-context fork of "Payments" on this Mac is being started.
- A move of "Cache warmup" from box-1 to box-2 is halfway (its copy has
  reached box-2, and the agent has not yet stopped).
- A board item is being saved (its status changes from To do to In progress).
- The state store is writing its last good copy.
- Power is cut. When it returns, the Mac boots and I open the app.

## Steps

1. **The fork is cut while its code is copied.**
   - **I do:** start the fork. The power is cut at `fork.copy_code:<repo>`,
     before the agent starts.
   - **I see:** after the daemon starts, the fork is not in the sidebar. The
     parent session is as it was, with its work and its conversation.
   - **State and events:** the fork's record, its folder and its conversation
     copy are removed. The parent is untouched.
   - **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart`.
     The same undo is checked when the cut is at the record write, the conversation
     copy, or the launch: `a_fork_cut_when_its_records_are_saved_is_undone_at_restart`,
     `a_fork_cut_after_its_conversation_is_copied_is_undone_at_restart` and
     `a_fork_cut_before_its_agent_launches_is_undone_at_restart`.
     These are daemon stops, and the undo is run by the daemon. See the Likely bug
     on `sessions/src/fork.rs:520` for the power-cut version.

2. **The fork is cut after its agent started.**
   - **I do:** start the fork again. The power is cut at `fork.verify`, after the
     agent has started.
   - **I see:** after the daemon starts, the fork is in the sidebar under its
     parent and is running. Its conversation is there.
   - **State and events:** the fork is kept and resumed. It is not undone.
   - **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart`.

3. **The move is cut halfway, after its copy reached box-2.**
   - **I do:** move "Cache warmup" to box-2. The power is cut at `move-copied`.
   - **I see:** after the daemon starts, the session is on box-1, running one
     agent, with the same work and the same agent process. box-2 has no copy and
     no agent. Moving it again works.
   - **State and events:** the move is undone. The copy taken back from box-2
     is removed.
   - **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart`.
     Cuts before the copy (`move-started`, `move-packed`, and the copy taken in)
     are covered by `a_move_cut_short_as_it_starts_is_undone_at_restart`,
     `a_move_cut_short_once_packed_is_undone_at_restart` and
     `a_move_cut_short_once_the_copy_is_taken_in_is_undone_at_restart`.

4. **The move is cut after the agent moved: finished.**
   - **I do:** move the session to box-2. The power is cut at `move-resumed`,
     `move-switched` or `move-released`.
   - **I see:** after the daemon starts, the session is on box-2 with one agent.
     The old copy on box-1 is kept in its trash (the test's doc comment says so; the
     trash check itself is not asserted).
   - **State and events:** the move is finished. There are never two agents.
   - **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`.

5. **The board item is being saved when the power is cut.**
   - **I do:** change an item's column on the board (To do to In progress). The
     power is cut during the save.
   - **I see:** after the daemon starts, the item shows either To do or In
     progress. It is never half changed.
   - **State and events:** the item is one whole version.
   - **Status:** Code, no test. The write goes through `daemon/humatad/src/board_service.rs:860`
     (`write`), and the power-cut suite has no board operation (`daemon/humatad/tests/common/power_cut.rs:77-94`
     lists Start, the forks, Move, the archive and restore, CommentSend and
     Transfer). `daemon/humatad/tests/boards.rs` has no cut test.

6. **The state store is writing its last good copy when the power is cut.**
   - **I do:** nothing. The daemon writes its last good copy after writes go
     quiet. The power is cut at `store.last_good.copied`, before the copy is
     renamed into place.
   - **I see:** after the daemon starts, the store opens whole. The unfinished
     copy is gone, and no partial file is left behind.
   - **State and events:** the store reads as before the copy was started.
   - **Status:** Works (real daemon, store level). `daemon/store/tests/kill_points.rs::a_write_killed_before_it_was_renamed_into_place_leaves_nothing_once_reopened`
     (mode `last_good`). The test runs a real child process that is killed on real files,
     not a `humatad2` process.

7. **Power returns. The store is damaged and comes back from its last good copy.**
   - **I do:** boot the Mac and open the app. The store's database is damaged
     (the test writes junk over `state.sqlite`).
   - **I see:** the daemon restores the last good copy and starts. The sessions
     come back. Writes made after the copy are not in it, and the store's own tests
     list the rows lost since the copy (`daemon/store/tests/last_good.rs::a_database_damaged_inside_lists_the_rows_lost_since_the_copy`).
   - **State and events:** `LastStart.restored_copy_taken_unix_ms` is set to the
     time the copy was taken.
   - **Status:** Works (real daemon). `daemon/humatad/tests/restart.rs::a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`
     (the start carries the copy's time). Restore-then-open, when the restore is cut
     part way, is `daemon/store/tests/kill_points.rs::a_restore_killed_part_way_is_finished_when_the_store_opens_again` (store level).
     The app does not show the restore. See Gaps.

8. **The machine comes back and its sessions resume.**
   - **I do:** boot the Mac. The daemon starts and brings sessions back.
   - **I see:** each running session shows its agent resumed in its
     conversation. The shell tabs come back fresh in their folders, or with what
     they printed.
   - **State and events:** each agent comes back on its own conversation. A
     holder that was killed with the power is replaced, and a shell that cannot
     come back is listed as not brought back.
   - **Status:** Works (real daemon, simulated reboot). `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation`
     (the test kills the daemon and every holder in the folder, then starts them again).
     `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back`.

9. **The restart report lists each item.**
   - **I do:** look at the restart report in the app, or run `humata doctor restarts`.
   - **I see:** each session resumed, each shell or agent not brought back with
     its reason, and each move or fork named with how it ended.
   - **State and events:** the report folder holds the restart report. The
     live client reads `last_start` once when it sees a session event (see Gaps).
   - **Status:** Works (real daemon) for the report content:
     `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped`,
     `daemon/humatad/tests/restart.rs::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`,
     `daemon/humatad/tests/restart_operations.rs::a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended`.
     The toast that tells the restart Works (real daemon): `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`. Drawing the report in the app is Not built, no milestone (flow 7 step 6).
     No golden draws the report line (OVERVIEW.md:537).

10. **A session whose conversation is gone: "Couldn't resume".**
    - **I do:** boot the Mac after the conversation files were lost. The session
      "Fix the ledger" is listed.
    - **I see:** the session shows as exited and needing recovery. The card reads
      "Couldn't resume" with the reason and a **Try again** button
      (ui/src/resume_failed.rs:111-135, ui/src/restart_report.rs:137-142). Nothing
      runs in its place.
    - **State and events:** the daemon marks the session `Recoverable` and
      `Exited`. No agent is started.
    - **Status:** Works (real daemon) for the model: `desktop/tests/live_flows.rs::a_session_whose_conversation_is_lost_cannot_resume`.
      The card and its buttons Work (real daemon): `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` and `desktop/tests/live_flows/restart.rs::start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary` click them.

11. **I press Try again.**
    - **I do:** click **Try again** on the card.
    - **I see:** the session resumes on its conversation, and the card goes away.
    - **State and events:** the session starts an agent on its conversation.
    - **Status:** Works (real daemon). `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` restores the conversation file and presses Try again.

## Branches

### The power is cut on the host only

- **When:** the host (this Mac) loses power during any of eight operations
  (Start, a local fork, a fork onto box-1, a move, an archive on box-1, a
  restore on the host, a comment send, a transfer). Box-1 keeps running.
- **I do:** nothing during the cut. The host restarts and box-1 links back.
- **I see:** each item is as it was or finished, or the restart report names it.
- **Expected outcome:** nothing that PER-25 lists is lost.
- **Status:** Works (real daemon, simulated cut). `daemon/humatad/tests/power_cut_host.rs`
  has 16 tests generated by the `cut_tests!` macro: one per operation and per
  chunk of its fault points (`a_start_with_the_hosts_power_cut_loses_nothing_part_1`
  and so on). The suite was not yet gated when SAFE.md was last written.

### The power is cut on box-1 only

- **When:** box-1 loses power during the same eight operations. The host keeps running.
- **I do:** nothing during the cut. Box-1 restarts and the host links back.
- **I see:** the same as the host case.
- **Expected outcome:** nothing is lost.
- **Status:** Works (real daemon, simulated cut). `daemon/humatad/tests/power_cut_machine.rs` (16 tests, same generator).

### The power is cut on both

- **When:** both the host and box-1 lose power during one operation.
- **I do:** nothing during the cut. Both restart.
- **I see:** the same as above, on both machines.
- **Expected outcome:** nothing is lost on either side.
- **Status:** Works (real daemon, simulated cut). `daemon/humatad/tests/power_cut_both.rs` (16 tests). Its list of operations is the same as the host case.

### A cut while the last good copy is itself restored

- **When:** the power is cut while the store is putting back its last good copy.
- **I do:** nothing. The store restores at the next start.
- **I see:** the store opens whole, from the copy.
- **Expected outcome:** the restore is finished when the store opens again.
- **Status:** Works (real daemon, store level). `daemon/store/tests/kill_points.rs::a_restore_killed_part_way_is_finished_when_the_store_opens_again`
  (points `store.restore.copied` and `store.restore.moved`).

### `humata doctor restarts` repairs what it can

- **When:** a session was not brought back after a restart.
- **I do:** run `humata doctor restarts`, then run the repair it offers.
- **I see:** the report's safe repairs run. A lossy repair waits for `--allow`.
  The session comes back once what it needed is back.
- **Expected outcome:** the safe repair runs, the lossy one needs permission, and
  nothing else changes.
- **Status:** Works (real daemon) for the repair:
  `daemon/humatad/tests/doctor.rs::a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back`
  and `a_shell_tab_that_was_not_brought_back_is_started_again`.
  The command's reading of a report is Fakes only: `cli/tests/doctor.rs::explain_reads_the_newest_report_and_says_what_wasnt_brought_back`
  (fake daemon, real report files).

### A board edit is cut during its save

- **When:** the power is cut while a board item is being written.
- **I do:** nothing. The daemon restarts.
- **I see:** the item is either its old version or its new one.
- **Expected outcome:** one whole version.
- **Status:** Code, no test. See step 5.

### The start's restart report is not pushed to the app

- **When:** the app opens while the daemon is still bringing sessions back, or
  after a restart.
- **I do:** open the app after the boot.
- **I see:** the report appears once the app reads it.
- **Expected outcome:** the report reaches the app.
- **Status:** Not built (OVERVIEW.md:534: "Push `last_start` to the app when it's
  set (EventService) instead of the app asking again", unchecked). The app asks on a
  session event (see Likely bugs).

## Expected outcomes

- [ ] A fork cut before its agent starts is gone after the restart, with its folder and its conversation copy, and the parent is as it was.
- [ ] A fork cut after its agent started is kept and resumed.
- [ ] A move cut after its copy reached the other machine is undone, with one agent, on the original machine.
- [ ] A move cut after the agent moved is finished, with one agent on the new machine.
- [ ] A board item cut during its save is either its old version or its new one.
- [ ] A store cut while its last good copy is saved opens whole.
- [ ] A damaged store comes back from its last good copy, and the start records the copy's time.
- [ ] Each cut on the host, on the box, or on both leaves nothing from PER-25 lost or unnamed.
- [ ] The restart report names each session resumed and each one not brought back, with its reason.
- [ ] A session whose conversation is lost is marked as needing recovery, and nothing runs in its place.
- [ ] Try again resumes that session on its conversation.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Fork cut before its agent starts | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` |
| 2. Fork cut after its agent started | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart` |
| 3. Move cut after its copy | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart` |
| 4. Move cut after the agent moved | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart` |
| 5. Board edit cut during save | Code, no test | `daemon/humatad/src/board_service.rs:860`; no board operation in `daemon/humatad/tests/common/power_cut.rs:77-94` |
| 6. Store cut while writing last good copy | Works (real daemon, store level) | `daemon/store/tests/kill_points.rs::a_write_killed_before_it_was_renamed_into_place_leaves_nothing_once_reopened` |
| 7. Damaged store restored from last good copy | Works (real daemon) | `daemon/humatad/tests/restart.rs::a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`; the app does not show the restore (Not built) |
| 8. Sessions resume after power returns | Works (real daemon, simulated reboot) | `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` |
| 9. Restart report lists each item | Works (real daemon) for content and the toast; Not built for drawing the report | `daemon/humatad/tests/restart.rs::the_restart_report_has_what_the_daemon_logged_before_it_stopped`; `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` |
| 10. "Couldn't resume" for a lost conversation | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_whose_conversation_is_lost_cannot_resume`; `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` |
| 11. Try again | Works (real daemon) | `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` |
| Branch: power cut on the host | Works (real daemon, simulated cut); suite not yet gated | `daemon/humatad/tests/power_cut_host.rs` (16 tests) |
| Branch: power cut on the box | Works (real daemon, simulated cut); suite not yet gated | `daemon/humatad/tests/power_cut_machine.rs` (16 tests) |
| Branch: power cut on both | Works (real daemon, simulated cut); suite not yet gated | `daemon/humatad/tests/power_cut_both.rs` (16 tests) |
| Branch: cut while last good copy is restored | Works (real daemon, store level) | `daemon/store/tests/kill_points.rs::a_restore_killed_part_way_is_finished_when_the_store_opens_again` |
| Branch: `humata doctor restarts` | Works (real daemon) for repair; Fakes only for reading (SAFE lane) | `daemon/humatad/tests/doctor.rs::a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back`; `cli/tests/doctor.rs::explain_reads_the_newest_report_and_says_what_wasnt_brought_back` |
| Branch: board edit cut | Code, no test | see step 5 |
| Branch: report not pushed | Not built | OVERVIEW.md:534 |

## Gaps and likely bugs

- **Likely bug:** `daemon/sessions/src/fork.rs:505-530` (`copy_conversation`) writes the
  conversation copy with `fs::File::create` and `io::copy`, not through `humata2_durable`.
  A simulated power cut does not undo that write, so the power-cut version of the fork
  cases (LocalFork and RemoteFork in `power_cut_host.rs`, `power_cut_machine.rs`,
  `power_cut_both.rs`) may fail. Confirm by running one `LocalFork` power-cut shard and
  checking for a conversation copy after the restart. SAFE.md T6b-2 lists this as pending
  (batch 8), so it is known work, not yet done.
- **Likely bug:** `daemon/sessions/src/own_copy.rs:360` renames the moved folder into place
  with `fs::rename` rather than `humata2_durable::rename`. The same cause as above. SAFE.md
  T6b-1 and T6b-2 cover it (pending). Confirm with a `Move` power-cut shard.
- **Likely bug (medium-low):** `clients/live/src/events.rs:115` and
  `clients/live/src/daemon.rs:81-95` read `last_start` only when a session event arrives,
  and only once until one succeeds. The daemon sets the start at `daemon/humatad/src/restart.rs:174` (`setting.set(start)`)
  after `bring_back` finishes. A session event sent during the bring-back can be read
  before the start is set, and then no later event may come. The report would not show
  until the next session event. Confirm by a restart with several sessions, and check the
  app's report after the last session comes back. `session_service::after_restart` may
  send more events after the set, which would hide this, so check that too.
- **Not built (OVERVIEW.md:534, unchecked):** the app does not get `last_start` pushed. It
  asks on a session event (clients/live/src/daemon.rs:62-71). No milestone is named.
- **Not built (OVERVIEW.md:537, unchecked):** the restart report's lines and a restore's
  losses are not drawn. The app's `Restart` model (clients/src/model.rs:214-225) has no
  restored-copy field, so a restored store is not shown at all. `clients/live/src/map.rs:236`
  drops `restored_copy_taken_unix_ms`. No golden draws "Couldn't resume" either.
- **Not built (no milestone):** a power-cut case for a board edit. See step 5.
- **Gap in the tests, not a bug:** the power-cut suite is a simulated cut at a fault point,
  and writes that bypass `humata2_durable` are not undone (`tests/common/power_cut.rs`
  header). A pass proves less for those items. The "reboot" test kills the daemon and every
  holder in the folder; it does not reboot the Mac.
- **Brief correction:** `daemon/humatad/tests/restart.rs` covers restarts in general,
  not only power cuts. `daemon/revive/tests/kill_matrix.rs` (the brief's
  `kill_matrix*`) kills the holder and the daemon at points of the revive path; it is
  not a cut of the store.
