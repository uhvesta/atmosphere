# A wrong archive is undone from the toast, and a typo is fixed with ⌘Z

Layer: micro. Platforms: macOS (⌘ keys) first; Linux and web for the field and toast steps, where the code is shared.

## Starting state

- Humata 2 is open in one window on this Mac, with the design sample workspace.
- A second session, "Currency codes" (id `currency`), is live in the payments-rewrite channel on this Mac. Its folder holds about 1.4 GB of clones and caches.
- Machines: this-mac (online) and devbox-1 (online).
- The New channel dialog is closed. The New session dialog is closed.
- I hold the keyboard. Nothing has been archived in this session yet.

## Steps

1. **I do:** open "Currency codes" and choose Archive. Then, in the toast, click Undo within a few seconds.
   **I see:** a confirm-free archive (no unpushed work) moves the session to Archived at once. The toast reads "Archived “Currency codes”. Frees 1.4 GB when this closes." with an Undo button. Clicking Undo puts the session back in the open list.
   **State and events:** the session's `archived` flag is set, then cleared by `RestoreSession`. The toast's `RestoreSession` action runs once. The disk is not freed until the daemon's undo window ends.
   **Status:** Fakes only. `desktop/tests/native/overlays.rs::archiving_offers_a_working_undo` runs the UI against the fake daemon. The restore half is real in `daemon/humatad/tests/archive.rs::undo_brings_the_session_back_exactly` (Works (real daemon)). Not proven live: the click on a toast in a running app.

2. **I do:** in the New channel dialog, type a channel name with a typo, then press ⌘Z in the name field and type the fix.
   **I see:** ⌘Z takes back the typing in the field, one step at a time. Redo is in the same Edit menu.
   **State and events:** the field's own undo history changes. Nothing is sent to the daemon until Create is pressed.
   **Status:** Code, no test. The Edit menu wires Undo and Redo to the input component's actions (`desktop/src/main.rs:240-241`). The key bindings live in the `gpui_kit` crate, which is not in this checkout, so I could not read them here. No test in `apps/humata2` types into a field and undoes it. The only undo tests cover the toast and the report's marks.

3. **I do:** start a new session, open the machine picker, pick devbox-1 by mistake, then pick this-mac before pressing Start.
   **I see:** the picker shows this-mac as the choice. Start uses this-mac.
   **State and events:** `features/app/src/new_session.rs:178-181` sets `sheet.machine` to the last pick. Nothing is sent until Start.
   **Status:** Code, no test. `ui/src/overlay_new_session.rs:223-265` draws the picker. The one test that sends a machine pick, `features/app/tests/new_session.rs::changing_channel_selects_an_online_host_in_that_channel`, asserts only the host that a channel change selects, not the pick that stuck.

4. **I do:** after the session has started on devbox-1, open the session menu, choose Move, pick this-mac, and confirm.
   **I see:** the move dialog lists the online machines. After confirm, the session shows this-mac and its agent resumes there.
   **State and events:** the agent pauses, the branch, uncommitted files and conversation are copied, the agent resumes on the target, and the old copy is deleted only after the new one runs (MV-2).
   **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. The dialog itself is covered by `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine` (Fakes only).

5. **I do:** click in the terminal of a session and press ⌘Z.
   **I see:** nothing happens in the terminal. The program does not get a key, and the screen does not change.
   **State and events:** on macOS, a ⌘ key that is not copy, paste or scroll is ignored by the terminal (`terminal/view/src/keys.rs:65`). It is left to the app's menu.
   **Status:** Fakes only. `terminal/view/src/keys.rs` has a pure unit test, `other_cmd_shortcuts_are_left_to_the_app`, which covers ⌘T and ⇧⌘C. `terminal/view/tests/view.rs::the_apps_own_shortcuts_are_not_sent_to_the_program` checks ⌘T on a fake host. No test presses ⌘Z itself, and none runs a real shell.

6. **I do:** in the terminal's shell prompt, type a few words, then press ⌃_ (Control with underscore).
   **I see (expected):** the shell's own line editor undoes the last edit of the line.
   **State and events:** the terminal should send the control byte for ⌃_ to the program, and the shell should undo the edit. Not checked by any test.
   **Status:** Code, no test. `terminal/view/src/keys.rs` passes Control keys to the program, and `ctrl_c_goes_to_the_program_on_every_platform` covers ⌃C only. The lab tests (`terminal/lab/tests/`) never send ⌃_ to a real zsh.

## Branches

### Undo after the toast has gone

- **When:** more than 4 seconds pass after the archive (the toast's life), or I dismiss the toast.
- **I do:** look for Undo on the session, in the Archived list, or in the palette.
- **I see (expected by the spec):** the archived session's page shows Restore and Delete (LD-3). Restore asks for a machine and clones the pushed branches back (LD-4).
- **I see (code):** the toast is gone. `features/app/src/toast.rs:108-110` removes it on expiry. No Restore or Delete control for an archived session exists in `ui/src/` (the only "Restore" control is in the file manager's trash).
- **Expected outcome:** the archived session can be restored from its page, from what was pushed. A branch that was never pushed comes back from `main`, and the dialog says that work was deleted at archive (LD-4).
- **Status:** Not built (LD-3, LD-4). The daemon RPC `RestoreSession` exists and is tested (`daemon/humatad/tests/archive.rs::the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed`, Works (real daemon)), but the UI has no way to call it after the toast.

### Undo after the app restarted

- **When:** I quit and reopen Humata 2 while the Undo window is still open, or the machine restarts inside the window.
- **I do:** look for the toast or Undo.
- **I see:** no toast (toasts are kept only in memory; `features/app/src/lib.rs:107`, and `features/app/src/saved.rs` does not save them). The session is archived. If the daemon restarted, its delete was finished.
- **Expected outcome:** Undo is gone. The session stays archived and its folder is deleted once the window closes or at the next start.
- **Status:** Works (real daemon) for the daemon side: `daemon/humatad/tests/archive.rs::a_restart_inside_the_window_finishes_the_delete` shows the folder is removed after a restart and the session stays Archived. The app side (no toast after an app restart) is Code, no test. The spec's clause that the delete happens at once when the app quits is Not built (see gaps).

### Rewind an agent turn I regret

- **When:** the agent did something in its turn that I want undone.
- **I do:** press the rewind key or type `/rewind` in the agent's terminal.
- **I see (expected, if the agent CLI offers it):** the CLI's own rewind picker, in its terminal.
- **I see (code):** no Humata rewind. SE-10 says the terminal is the agent CLI's own, with "queueing, rewind, compaction and images" left to the CLI. Section 27 lists rewind as a non-goal for Humata.
- **Expected outcome:** no Humata-level rewind. Whatever the CLI does with `/rewind` happens inside its own terminal.
- **Status:** Not built (no milestone; SPEC SE-10 and §27). Searched `grep -rni rewind` in `features/`, `ui/src/`, `daemon/humatad/src/` and `daemon/conversation/`: only comments and copy text, no Humata rewind RPC. No test runs a CLI's `/rewind`.

### Review chat's undo of a view move

- **When:** the review chat moved the view to a file and line, and I want that move undone.
- **I do:** press Undo on the change, or turn the review chat off.
- **I see:** the view returns to where it was before the move. The change is removed from the feed.
- **Expected outcome:** the daemon restores the previous view, and a repeated undo answers the same.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off`. The undo after a restart is `daemon/humatad/tests/review.rs::undo_and_a_repeated_key_act_the_same_after_a_restart` (Works (real daemon)).

### The Undo request itself fails

- **When:** the daemon refuses the restore (for example the folder is in use).
- **I do:** click Undo.
- **I see (expected):** an error toast with the reason, and the first toast stays so I can try again.
- **I see (code):** `features/app/src/toast.rs` (`toast_undone`) sets `working` back to false and shows the daemon's error as a new toast.
- **Expected outcome:** the error says what failed. The session stays archived.
- **Status:** Code, no test. No test covers a failing restore on the toast.

## Expected outcomes

- [ ] Clicking Undo on the archive toast puts the session back, with its work and conversation.
- [ ] A typo in a field is undone with ⌘Z, and redone from the Edit menu.
- [ ] A machine pick made before Start is the machine the session starts on.
- [ ] A session moved after start runs on the new machine, and its branch and conversation are intact.
- [ ] ⌘Z in the terminal sends nothing to the program.
- [ ] ⌃_ reaches the shell's line editor and undoes the last edit.
- [ ] Once the toast is gone, an archived session can be restored from its page.
- [ ] After an app restart the toast is gone, and the daemon finishes the delete if its window closed.
- [ ] A Humata rewind does not exist; a CLI's own rewind runs inside its terminal.
- [ ] The review chat's view move can be undone, before and after a restart.
- [ ] A refused Undo shows the reason and leaves the session archived.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Archive, Undo on the toast | Fakes only | `desktop/tests/native/overlays.rs::archiving_offers_a_working_undo`; daemon half `daemon/humatad/tests/archive.rs::undo_brings_the_session_back_exactly` |
| 2. ⌘Z in a channel name field | Code, no test | `desktop/src/main.rs:240-241`; bindings in `gpui_kit` (not in this checkout) |
| 3. Wrong machine pick before Start | Code, no test | `features/app/src/new_session.rs:178-181`; `features/app/tests/new_session.rs` |
| 4. Move a session to the right machine | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 5. ⌘Z in the terminal does nothing | Fakes only | `terminal/view/src/keys.rs` (`other_cmd_shortcuts_are_left_to_the_app`); `terminal/view/tests/view.rs::the_apps_own_shortcuts_are_not_sent_to_the_program` |
| 6. ⌃_ readline undo | Code, no test | `terminal/view/src/keys.rs` |
| Branch: Undo after the toast has gone | Not built | LD-3, LD-4 (SPEC §8); daemon RPC tested in `daemon/humatad/tests/archive.rs` |
| Branch: Undo after the app restarted | Works (real daemon) | `daemon/humatad/tests/archive.rs::a_restart_inside_the_window_finishes_the_delete` |
| Branch: rewind an agent turn | Not built | no milestone; SPEC SE-10 and §27 |
| Branch: review chat's view move undo | Works (real daemon) | `daemon/humatad/tests/review.rs::the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off` |
| Branch: the Undo request fails | Code, no test | `features/app/src/toast.rs` (`toast_undone`) |

## Gaps and likely bugs

- Likely bug: `features/app/src/lib.rs:65` sets the toast to 4 seconds (`TOAST`). SPEC MV-15 says the toast stays for 10 seconds, and the daemon's undo window is 10 seconds (`daemon/humatad/src/session_service/archive.rs:40`, `UNDO_WINDOW_MS`). So the Undo button goes about 6 seconds before the daemon stops accepting the undo. Confirm by archiving a session and timing the toast against the restore.
- Not built: the MV-15 clause "or at once when the app quits". Quitting (`desktop/src/main.rs:202`, `:307`) does not tell the daemon to finish the archive, so the folder waits for the window (MV-15, provisional D110).
- Not built: restore of an archived session from its page, and delete from trash (LD-3, LD-4). The daemon RPC exists, the UI has no control.
- Not built: a Humata rewind. The agent CLI's own `/rewind` is the only rewind (SE-10, §27 non-goal).
- Code, no test: ⌘Z in text fields. I could not read the `gpui_kit` input bindings from this checkout, so this step is unverified.
- Code, no test: ⌃_ reaching the shell, and a machine pick that is changed before Start.
- Fakes only: the archive toast's Undo button, which the native test drives against the fake daemon.
