# Roots and placement

What NOPATH promises (PL-1 to PL-13, D460–D469): the user picks machines,
never folders, and sees paths only when asking.

### Settings shows each machine's root

- **I do:** open Settings › Machines.
- **I see:** each machine with **Humata folder** and **Change…**; This Mac's reads `~/Humata` unless changed.
- **Expected outcome:** nothing else in Settings asks for a path.
- **Status:** Built. Evidence: `//apps/humata2/desktop:root_change_test` (`change_then_plan_then_new_channels_only_shows_the_new_root_on_the_row`), `//apps/humata2/daemon/humatad:change_root_test` (`this_macs_root_is_the_folder_channels_live_in_and_it_changes_like_a_boxs`).

### Changing a root plans first

- **I do:** Change… on devbox-1, pick a new folder.
- **I see:** the plan lists the channels under the old root, with Move them or Use for new channels only (FC-10).
- **Expected outcome:** nothing moves until the user chooses.
- **Status:** Built. Evidence: `//apps/humata2/daemon/humatad:change_root_test` (`a_plan_lists_the_channels_on_the_machine_and_the_sessions_running_in_them`, `moving_channels_brings_each_channel_with_its_files_and_sessions_to_the_new_root`, `moving_a_box_brings_its_channels_with_their_files_and_sessions_to_the_new_root`, `a_desk_session_the_box_moved_resumes_in_the_new_place`, `retrying_the_failed_change_asks_the_box_again_and_ends_done_with_the_root_saved`, `moving_channels_is_refused_while_a_session_runs_and_new_channels_only_still_goes_through`), `//apps/humata2/features/hosts:root_test`, `//apps/humata2/desktop:root_change_test` (`running_sessions_still_allow_new_channels_only`).

### Copy to a machine lands by rule

- **I do:** in #ml's shared folder on This Mac, select `notes.md`, **Copy to ▸** › gpu-box.
- **I see:** the menu entry reads "gpu-box · ml shared"; the tray shows `notes.md → gpu-box · ml shared`.
- **Expected outcome:** the file is in #ml's shared folder on gpu-box, byte-equal. No path was shown or asked.
- **Status:** Built. Evidence: `//apps/humata2/features/files:manager_placement_test` (`a_channels_shared_folder_lands_in_the_same_channels_shared_folder_on_a_machine_of_the_channel`, `a_landing_label_names_the_machine_and_the_place`), `//apps/humata2/desktop:files_page_test` (`copy_to_a_machine_lands_by_rule_and_the_name_arrives_when_it_is_done`).

### A Home file goes to From <machine>

- **I do:** in This Mac › Home, **Copy to ▸** › devbox-1.
- **I see:** "devbox-1 · From This Mac".
- **Expected outcome:** the file lands in devbox-1's From This Mac place.
- **Status:** Built. Evidence: `//apps/humata2/daemon/humatad:received_root_test` (`a_copy_from_home_lands_in_the_received_place_of_the_source_machine`), `//apps/humata2/features/files:manager_placement_test` (`anything_else_lands_in_the_targets_received_place_for_the_source_machine`).

### Somewhere specific… is the explicit ask

- **I do:** **Copy to ▸** › **Somewhere specific…**, pick gpu-box, open a folder, confirm.
- **I see:** the place browser, breadcrumbs of names.
- **Expected outcome:** the file lands in that folder; a taken name asks Fail, Replace or Keep both.
- **Status:** Built. Evidence: `//apps/humata2/desktop:files_page_test` (`somewhere_specific_copies_into_the_other_panes_folder`), `//apps/humata2/features/files:manager_placement_test` (`somewhere_specific_still_copies_into_the_other_pane_and_refuses_a_taken_name`).

### A taken name keeps both

- **I do:** copy `notes.md` to gpu-box again after editing it.
- **I see:** the tray says it was saved with a suffix.
- **Expected outcome:** both files exist; a byte-equal copy is skipped and the tray says it was already there.
- **Status:** Built. Evidence: `//apps/humata2/daemon/transfers:transfers_test` (`keeping_both_twice_numbers_the_second_copy_two_then_three`), `//apps/humata2/daemon/humatad:transfers_test` (`keeping_both_keeps_the_old_one_and_numbers_the_new_one`), `//apps/humata2/features/files:manager_placement_test` (`a_name_taken_where_a_machine_only_send_lands_keeps_both`), `daemon/transfers:transfers_test` (`keeping_both_a_file_with_the_same_bytes_writes_nothing_and_says_it_was_there`), `//apps/humata2/daemon/humatad:transfers_test` (`keeping_both_a_file_with_the_same_bytes_is_not_copied_again`), `//apps/humata2/features/files:manager_panes_test` (`a_finished_row_says_the_name_it_was_saved_as`, `a_finished_row_says_a_file_was_already_there`).

### Copy path and Reveal

- **I do:** right-click a clone in the Info panel.
- **I see:** Reveal in Finder (This Mac) or Open terminal here (remote), and Copy path.
- **Expected outcome:** Copy path puts the absolute path on the clipboard; nothing showed it before.
- **Status:** Built. Evidence: `//apps/humata2/features/session:clone_menu_test` (`copy_path_puts_the_clones_absolute_path_on_the_clipboard`, `reveal_asks_finder_for_the_clones_path`, `open_terminal_here_starts_a_new_shell_tab_in_the_clones_folder`), `//apps/humata2/desktop:files_row_menu_test`, `//apps/humata2/daemon/humatad:terminal_test` (Open terminal here in a folder).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| Settings shows each machine's root | Built | `desktop:root_change_test`, `daemon/humatad:change_root_test` |
| Changing a root plans first | Built | `daemon/humatad:change_root_test` (This Mac and a box), `features/hosts:root_test`, `desktop:root_change_test` |
| Copy to a machine lands by rule | Built | `features/files:manager_placement_test`, `desktop:files_page_test` |
| A Home file goes to From <machine> | Built | `daemon/humatad:received_root_test`, `features/files:manager_placement_test` |
| Somewhere specific… is the explicit ask | Built | `desktop:files_page_test`, `features/files:manager_placement_test` |
| A taken name keeps both | Built | `daemon/transfers:transfers_test`, `daemon/humatad:transfers_test`, `features/files:manager_panes_test` (saved as / already there) |
| Copy path and Reveal | Built | `features/session:clone_menu_test`, `features/files:row_menu_test`, `desktop:files_row_menu_test`, `daemon/humatad:terminal_test` |
