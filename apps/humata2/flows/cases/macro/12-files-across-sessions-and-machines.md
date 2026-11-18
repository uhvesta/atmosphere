# Renaming, moving, deleting and restoring files in a session's Files page works across machines, and a cut copy is finished or undone

Layer: macro. Platforms: macOS (this Mac, the host) and a linked box (box-1).

Status tags in this file: a test that runs the real `humatad2` (or the real
`humata` binary against it) is tagged **Works (real daemon)**. Tests that
drive the app headless on fake clients, or a fake daemon, are **Fakes only**.
Where a step has no code path or no test, the tag says so. A step or branch takes
the tag of its main claim; the Evidence column names any part that is only faked
or not built.

## Starting state

- This Mac runs the host daemon. Channel "Payments" has a session "Ledger
  fix" with a folder of its own (its clone) and a shared folder for the
  channel.
- box-1 is linked and has a session "Cache warmup" with a log file in its
  clone.
- The app is open on the Files page of "Ledger fix". Its left pane shows the
  session's clone on this Mac. The right pane shows the channel's shared
  folder on this Mac.
- A file "notes.txt" sits in the clone, and a folder "reports" sits in the
  shared folder.

## Steps

1. **Open the Files page.**
   - **I do:** open a session and choose its **Files** page.
   - **I see:** two panes. Each has a machine picker, a breadcrumb of place
     names (no path, PL-11) and a listing, folders first, then files by name.
   - **State and events:** the app lists both panes' folders through the file
     service. Nothing changes on disk.
   - **Status:** Works (real daemon). `desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon`
     (the test opens both panes on a real daemon). The machine popup and the
     listing rules are Fakes only: `desktop/tests/files_page.rs::opening_files_lists_this_mac_in_both_panes`,
     `features/files/tests/manager_fake.rs::a_listing_has_folders_first_then_files_by_name`.

2. **Make a new folder.**
   - **I do:** in the right pane, click **New folder**, type "out", press Enter.
   - **I see:** the folder "out" appears in the pane.
   - **State and events:** the folder is created on this Mac's disk.
   - **Status:** Works (real daemon). The same live test as step 1 lists the new
     folder. Fakes only for the prompt: `desktop/tests/files_page.rs::new_folder_asks_for_a_name_and_makes_the_folder`.
     The daemon creates the folder once and refuses a taken name: `daemon/humatad/tests/file_ops.rs::a_folder_is_made_once_and_a_taken_name_is_refused`.

3. **Rename a file.**
   - **I do:** select "notes.txt", choose **Rename**, type "notes-old.txt", press Enter.
   - **I see:** the listing shows "notes-old.txt" in its place.
   - **State and events:** the file is renamed in place. Nothing is replaced or moved.
   - **Status:** Works (real daemon). The live test renames a folder on the right pane
     (`desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon`).
     The daemon rule: `daemon/humatad/tests/file_ops.rs::rename_changes_the_name_and_never_replaces_or_moves`.
     The prompt's rules (exactly one name selected; a bad name keeps the prompt open):
     Fakes only, `features/files/tests/manager_naming.rs::a_good_name_renames_and_closes_the_prompt`.

4. **Move a file from the clone into the shared folder.**
   - **I do:** drag or use **Move** to move "notes-old.txt" from the left pane to the right pane.
   - **I see:** a tray row shows the move, then the file is gone from the left and present on the right.
   - **State and events:** the source is removed when the move is done. The
     destination is whole.
   - **Status:** Works (real daemon). The live test moves a 2000-byte file from the
     clone to shared (`desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon`).
     The tray row: Fakes only (`features/files/tests/manager_panes.rs::a_copy_shows_in_the_tray_and_the_name_appears_on_the_other_side_when_done`).

5. **Move a folder.**
   - **I do:** move "reports" to the clone.
   - **I see:** "reports" appears in the clone with everything in it, and is gone from the shared folder.
   - **State and events:** a tree move. The source is removed only after the tree arrives whole.
   - **Status:** Works (real daemon, daemon level). `daemon/humatad/tests/transfers.rs::a_move_leaves_the_source_gone_and_the_destination_whole`
     moves a tree and checks that the source is gone and the bytes are the same. The
     app does not run this step in a live test.

6. **Delete a file to the Trash.**
   - **I do:** select a file, press **Delete**.
   - **I see:** the file leaves the listing. The Trash shows it with its date.
   - **State and events:** the file moves to the Trash, and it stays there 30 days.
   - **Status:** Works (real daemon). The live test deletes to the Trash
     (`desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon`).
     The daemon rule: `daemon/humatad/tests/trash.rs::a_trashed_file_leaves_its_folder_and_comes_back_whole`.
     The selection move to the Trash: Fakes only, `desktop/tests/files_page.rs::delete_moves_the_selection_to_the_trash`.

7. **Restore it from the Trash.**
   - **I do:** open the Trash, select the file, press **Restore**.
   - **I see:** the file is back in its folder, with its bytes.
   - **State and events:** the Trash entry is removed. If something else has taken
     the path, both are kept (the restored one gets a new name).
   - **Status:** Works (real daemon). The live test restores (`the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon`).
     `daemon/humatad/tests/trash.rs::restoring_onto_a_path_something_else_took_keeps_both`
     and `daemon/humatad/tests/trash.rs::the_trash_survives_a_daemon_restart`.
     The app's round trip: Fakes only, `features/files/tests/manager_panes.rs::delete_then_restore_puts_the_file_back`.

8. **Upload a file from Finder.**
   - **I do:** drag a file from Finder onto the right pane.
   - **I see:** the file appears in the pane's folder after the tray shows its transfer.
   - **State and events:** the file is uploaded in chunks and checked by its hash.
   - **Status:** Works (real daemon). `desktop/tests/live_flows/files.rs::files_dropped_from_finder_upload_into_the_pane`.
     The upload path in the daemon: `daemon/humatad/tests/transfers.rs::an_upload_in_chunks_arrives_whole_and_the_transfer_is_done`.
     The app's drop handling is `ui/src/file_manager.rs:409`.

9. **Copy a log from box-1's session to this Mac.**
   - **I do:** in the left pane choose box-1, open "Cache warmup"'s clone, select
     the log, and copy it to the right pane (this Mac's shared folder).
   - **I see:** the tray shows the copy. The log then appears on this Mac with the same bytes.
   - **State and events:** a machine-to-machine copy, relayed through the host.
   - **Status:** Works (real daemon, fake box) at the daemon level, between two boxes:
     `daemon/humatad/tests/remote_transfers.rs::a_folder_copied_from_one_machine_to_another_arrives_whole`
     (box-a to box-b). The box-1 to this Mac direction is not named in a test. The app
     copy between machines is Fakes only (`features/files/tests/manager_fake.rs::a_machine_to_machine_transfer_goes_through_the_host`).
     The live Files test says it does not try a copy between machines
     (`desktop/tests/live_flows/files.rs`, file header).

10. **Open in editor.**
    - **I do:** select a file and press **Open in editor**.
    - **I see:** the button is greyed out. Nothing opens.
    - **State and events:** none.
    - **Status:** Not built (no milestone). The button is disabled in
      `ui/src/panel_files.rs:306-311`. OVERVIEW.md:457 lists "Files Open in editor"
      as open.

11. **Use the Upload button.**
    - **I do:** press **Upload…** on the pane.
    - **I see:** the button is greyed out. Dragging from Finder still works (step 8).
    - **State and events:** none from the button.
    - **Status:** Not built (no milestone). The button is disabled in
      `ui/src/panel_files.rs:313-319`. OVERVIEW.md:457 lists the Upload button as open.
      The drop path is Works (step 8).

12. **Read the transfers list and its progress.**
    - **I do:** open the tray while a copy runs.
    - **I see:** the transfer with its progress, and a cancel control.
    - **State and events:** the transfer's state changes are pushed to the app.
    - **Status:** Fakes only for the tray: `features/files/tests/manager_panes.rs::a_transfer_can_be_cancelled_from_the_tray`
      and `features/files/tests/manager_fake.rs::every_transfer_change_reaches_a_subscriber`.
      The daemon's change watch is Works (real daemon): `daemon/humatad/tests/transfers.rs::a_watch_from_the_current_version_waits_for_the_next_change`.

13. **`humata cp` and `humata mv` from the CLI.**
    - **I do:** in the session's terminal, run `humata cp <clone location> <machine>` (placed by PL-3), then `humata mv` with an explicit `<machine>:<location>`.
    - **I see:** the copy and move finish. `humata transfers` lists the finished copy.
    - **State and events:** the same transfer as the app's.
    - **Status:** Works (real daemon). `daemon/humatad/tests/cli_files.rs::cp_from_the_session_clone_to_shared_arrives_with_identical_bytes`,
      `daemon/humatad/tests/cli_files.rs::mv_between_folders_moves_the_file`,
      `daemon/humatad/tests/cli_files.rs::transfers_lists_a_finished_copy_and_the_active_list_leaves_it_out`.

## Branches

### A name clash

- **When:** the copy, move or new folder would write a name that is taken.
- **I do:** copy "notes.txt" into a folder that already has one, or make a folder that exists.
- **I see:** an error on the prompt, for a folder. For a copy, the choice of
  replace, keep both, or fail.
- **Expected outcome:** nothing is overwritten. Replacing moves the old one to the Trash.
  Keeping both numbers the new one. Failing writes nothing.
- **Status:** Works (real daemon). `daemon/humatad/tests/file_ops.rs::a_folder_is_made_once_and_a_taken_name_is_refused`,
  `daemon/humatad/tests/transfers.rs::a_taken_name_fails_without_writing_when_the_choice_is_fail`,
  `daemon/humatad/tests/transfers.rs::replacing_puts_the_old_one_in_the_trash`,
  `daemon/humatad/tests/transfers.rs::keeping_both_keeps_the_old_one_and_numbers_the_new_one`.
  The prompt's error: Fakes only, `features/files/tests/manager_naming.rs::a_taken_name_closes_the_prompt_and_lands_in_the_error`.

### Disk full

- **When:** the destination has no room for the copy.
- **I do:** copy a large folder to a full disk.
- **I see:** the low-disk state opens, with the largest items first. After freeing
  room, retry starts the transfer.
- **Expected outcome:** the copy fails at the start with exit 10 for the CLI, with
  nothing written. The low-disk screen names what to free.
- **Status:** Fakes only. The CLI's exit code for disk full is checked against a
  fake daemon (`cli/tests/exit_codes.rs::each_error_kind_exits_with_its_code_and_name`).
  The app's refusal and low-disk screen are Fakes only: `features/files/tests/manager_fake.rs::a_transfer_that_does_not_fit_fails_at_start_with_disk_full`,
  `features/files/tests/manager_panes.rs::disk_full_opens_the_low_disk_state_with_the_largest_rows_first`,
  `features/files/tests/manager_panes.rs::freeing_room_then_retrying_starts_the_transfer`.
  No daemon test fills a real disk. **Code, no test** for the daemon's disk-full path
  (`daemon/humatad/src/transfer_service.rs`).

### A transfer cut by a restart

- **When:** the daemon is killed while a copy runs, or as it starts.
- **I do:** nothing. The daemon starts again.
- **I see:** the copy finishes by itself, with identical bytes.
- **Expected outcome:** finished. A copy killed a third of the way, or killed as it
  starts, finishes. An interrupted upload survives and can be resumed.
- **Status:** Works (real daemon). `daemon/humatad/tests/transfers.rs::a_copy_killed_a_third_of_the_way_finishes_by_itself_after_a_restart`,
  `daemon/humatad/tests/transfers.rs::a_copy_killed_as_it_starts_finishes_by_itself_after_a_restart`,
  `daemon/humatad/tests/transfers.rs::an_interrupted_upload_survives_a_restart_and_can_still_be_resumed`.
  The "undone" side (what arrived is removed, the source stays) is tested for a cancel
  (`daemon/humatad/tests/transfers.rs::cancelling_a_paused_copy_removes_what_arrived_and_leaves_the_source`),
  not for a restart. A power-cut version (the `Transfer` operation) is in the
  power-cut suite, which is not yet gated (SAFE.md T5).

### The other machine is offline

- **When:** box-1's link is down during a copy, or the pane is for an offline machine.
- **I do:** start a copy to box-1, then take its link down.
- **I see:** the transfer is paused and names box-1. When the link is back, the
  copy goes on from what box-1 confirmed. A listing of box-1 says it is offline.
- **Expected outcome:** the copy waits and finishes. Nothing is invented while offline.
- **Status:** Works (real daemon, fake box). `daemon/humatad/tests/remote_transfers.rs::a_copy_whose_link_is_lost_part_way_waits_for_the_machine_and_finishes_when_it_is_back`,
  `daemon/humatad/tests/remote_transfers.rs::a_copy_goes_on_from_what_the_machine_confirmed_when_its_link_is_back`,
  `daemon/humatad/tests/remote_files.rs::with_the_link_down_a_listing_names_the_machine_offline_and_works_again_when_it_is_back`.
  The pane's offline state: Fakes only, `features/files/tests/manager_panes.rs::a_pane_goes_stale_and_refuses_actions_then_reloads_when_the_machine_returns`
  and `features/files/tests/manager_fake.rs::a_transfer_pauses_for_an_offline_machine_and_goes_on_when_it_returns`.

### A move from a machine to another

- **When:** a move (not a copy) from box-1 to a second box.
- **I do:** move a folder from box-a to box-b.
- **I see:** the source is in box-a's trash only once the copy is done.
- **Expected outcome:** the source is trashed only after the destination is whole.
- **Status:** Works (real daemon, fake boxes). `daemon/humatad/tests/remote_transfers.rs::a_move_from_a_machine_puts_the_source_in_that_machines_trash_only_once_it_is_done`.

### Another machine's files through the agent (MCP)

- **When:** an agent lists or reads a file on box-1 through `humatad2 mcp`.
- **I do:** the agent calls `files_list` with a location that names box-1.
- **I see:** the listing of box-1's folder, or a refusal if the rule says no.
- **Expected outcome:** the listing is allowed only when the rule allows
  FilesReadOtherMachine. Otherwise it is refused.
- **Status:** Code, no test for the MCP path. The tool exists (`daemon/mcp/src/tools.rs:112-150`),
  and `daemon/humatad/src/mcp.rs:768-795` (`file_location`) reads a `machine` field.
  No test in `daemon/humatad/tests` or `daemon/mcp/tests` calls it with another machine.
  The capability's rule is Fakes only: `daemon/access/tests/channel_desk.rs::one_capability_set_to_never_blocks_a_switch_with_two`
  (FilesReadOtherMachine set to Never), and the guard maps it at `daemon/humatad/src/guard.rs:391`.

### An agent brings a file into its shared/ from another machine

- **When:** an agent session runs on vm-a, a machine of the channel, and needs a file that is in the channel's `shared/` on This Mac.
- **I do:** the agent copies `this-mac:shared/x` into its own `shared/` with `humata cp`. Then it tries the other direction: it sends a file from its machine into `shared/` on This Mac.
- **I see:** the pull lands in `shared/` on vm-a, after a copy-in card if the rule asks. The push is refused at once with the pull hint: put the file in `shared/` here, and a session on the other machine copies it in, or the user sends it with `humata cp`.
- **Expected outcome:** the file in `shared/` on vm-a has the same bytes as `this-mac:shared/x`. The push writes nothing on the other machine, and no card is raised for it (D755, TP-4).
- **Status:** Works (real daemon, fake boxes) for the pull and the push's refusal kind: `daemon/humatad/tests/agent_send_machine.rs::an_agent_names_its_channels_machine_by_name_and_sees_no_other` (the agent there runs on This Mac and pulls from box-1). The refusal and its fix are Fakes only: `daemon/access/tests/refusals.rs::an_agent_writes_shared_only_on_its_own_machine`, `daemon/access/tests/refusals.rs::a_copy_into_a_remote_shared_is_refused_before_its_source_is_judged` and `daemon/access/tests/refusals.rs::an_agent_pulls_another_machines_file_into_its_own_shared`.

### An agent's bare destination word that names no machine

- **When:** an agent session in a channel with one clone, and a machine vm-b outside the channel (or no machine of that name at all).
- **I do:** the agent runs `humata cp this-mac:shared/a.txt vm-b`, then `cp … ./vm-b`, `cp … vm-b/` and `cp … notes.md`.
- **I see:** `vm-b` is refused as usage (exit 2): it is not a machine of this channel, so nothing was copied, with the hint to pick one `humata host ls` lists, or write `./vm-b` for a file or `vm-b/` for a folder. `./vm-b`, `vm-b/` and the dotted `notes.md` land in the agent's own clone.
- **Expected outcome:** the refused word starts no transfer, and the refusal is the same whether or not a machine the agent can't see is named vm-b. A machine of the channel named by its name still gets the file by PL-3 (D756, amending D655).
- **Status:** Fakes only (CLI against a fake daemon): `cli/tests/files.rs::an_agents_bare_word_that_is_no_machine_it_sees_starts_nothing`, `an_agents_dot_slash_word_lands_in_its_clone`, `an_agents_word_with_a_slash_lands_in_that_folder_of_its_clone`, `an_agents_dotted_word_lands_in_its_clone`, `an_agents_word_naming_a_machine_it_sees_sends_there` and `an_agent_cant_tell_a_hidden_machine_from_no_machine`.

## Expected outcomes

- [ ] The Files page shows two panes with their machines' folders, folders first.
- [ ] A new folder, a rename and a move each happen on the right disk, and nothing else changes.
- [ ] A folder moved between panes arrives whole, and the source is gone.
- [ ] A deleted file is in the Trash and comes back whole when restored.
- [ ] A file dropped from Finder is uploaded and is listed in the pane.
- [ ] A copy between machines lands with identical bytes, and a move leaves the source in the source machine's trash only once the copy is done.
- [ ] A name clash never overwrites; replace keeps the old one in the Trash.
- [ ] A copy cut by a restart finishes with identical bytes.
- [ ] A copy to an offline machine waits and finishes when it is back.
- [ ] `humata cp`, `humata mv` and `humata rm` do the same as the page.
- [ ] Disk full exits 10 and writes nothing.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Open the Files page | Works (real daemon); popup Fakes only | `desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon` |
| 2. New folder | Works (real daemon); prompt Fakes only | `daemon/humatad/tests/file_ops.rs::a_folder_is_made_once_and_a_taken_name_is_refused` |
| 3. Rename | Works (real daemon); prompt Fakes only | `daemon/humatad/tests/file_ops.rs::rename_changes_the_name_and_never_replaces_or_moves` |
| 4. Move a file between panes | Works (real daemon); tray Fakes only | `desktop/tests/live_flows/files.rs::the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon` |
| 5. Move a folder | Works (real daemon, daemon level only) | `daemon/humatad/tests/transfers.rs::a_move_leaves_the_source_gone_and_the_destination_whole` |
| 6. Delete to Trash | Works (real daemon) | `daemon/humatad/tests/trash.rs::a_trashed_file_leaves_its_folder_and_comes_back_whole` |
| 7. Restore from Trash | Works (real daemon); app round trip Fakes only | `daemon/humatad/tests/trash.rs::restoring_onto_a_path_something_else_took_keeps_both` |
| 8. Upload from Finder | Works (real daemon) | `desktop/tests/live_flows/files.rs::files_dropped_from_finder_upload_into_the_pane` |
| 9. Copy box-1 log to this Mac | Works (real daemon, box to box); box-1 to this Mac not named; app Fakes only | `daemon/humatad/tests/remote_transfers.rs::a_folder_copied_from_one_machine_to_another_arrives_whole` |
| 10. Open in editor | Not built (no milestone) | `ui/src/panel_files.rs:306-311`; OVERVIEW.md:457 |
| 11. Upload button | Not built (no milestone); drop works | `ui/src/panel_files.rs:313-319`; OVERVIEW.md:457 |
| 12. Transfers list and progress | Fakes only (tray); Works (real daemon) for the watch | `features/files/tests/manager_panes.rs::a_transfer_can_be_cancelled_from_the_tray` |
| 13. `humata cp` and `mv` | Works (real daemon) | `daemon/humatad/tests/cli_files.rs::cp_from_the_session_clone_to_shared_arrives_with_identical_bytes` |
| Branch: name clash | Works (real daemon); prompt Fakes only | `daemon/humatad/tests/transfers.rs::replacing_puts_the_old_one_in_the_trash` |
| Branch: disk full | Fakes only; daemon path Code, no test | `cli/tests/exit_codes.rs::each_error_kind_exits_with_its_code_and_name` |
| Branch: transfer cut by restart | Works (real daemon) for "finished"; "undone" only for a cancel | `daemon/humatad/tests/transfers.rs::a_copy_killed_a_third_of_the_way_finishes_by_itself_after_a_restart` |
| Branch: other machine offline | Works (real daemon, fake box); pane Fakes only | `daemon/humatad/tests/remote_transfers.rs::a_copy_whose_link_is_lost_part_way_waits_for_the_machine_and_finishes_when_it_is_back` |
| Branch: move between machines | Works (real daemon, fake boxes) | `daemon/humatad/tests/remote_transfers.rs::a_move_from_a_machine_puts_the_source_in_that_machines_trash_only_once_it_is_done` |
| Branch: MCP files on another machine | Code, no test (MCP); rule Fakes only | `daemon/mcp/src/tools.rs:112-150`; `daemon/humatad/src/mcp.rs:768-795` |

## Gaps and likely bugs

- **Not built (no milestone):** Open in editor is disabled (`ui/src/panel_files.rs:306-311`).
- **Not built (no milestone):** the Upload… button is disabled (`ui/src/panel_files.rs:313-319`).
  Dragging a file from Finder does upload, so the drop works but the button does not.
- **Code, no test:** the daemon's disk-full path. Only the CLI's exit code for it is tested,
  and that test uses a fake daemon. No test fills a real disk.
- **Code, no test:** the MCP `files_list` and `file_read` with a `machine` argument.
  The access rule for FilesReadOtherMachine is tested as policy only.
- **Gap in the tests:** the live Files test runs one machine only ("The harness has one daemon,
  so one machine (This Mac)"). The cross-machine copy from the page is Fakes only.
- **Gap in the tests:** the "undone" half of a transfer cut by a restart is not named in
  a test. The restart tests only show a copy finishing.
- **Note on the brief's pointers:** `desktop/tests/files_page.rs` is headless on fakes (its
  header says so), and `daemon/humatad/tests/remote_files.rs` drives the real release
  `humatad2` over gRPC (FileService). Neither is MCP. The transfers list itself has no daemon
  test; `daemon/humatad/tests/transfers.rs` covers the transfer's state and its watch.
- **No likely bugs found** in the files code read for this case beyond the Code, no test items above.
