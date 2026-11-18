# Session lifecycle and archive: the states a session shows, stopping one agent, archiving and Undo

Layer: explored · macOS · harness: desktop/tests/native/session.rs (lifecycle frames 01 to 09, headless), desktop/tests/native/session_menu.rs, desktop/tests/native/overlays.rs (archive Undo, frame 32), desktop/tests/native/fork_move.rs (archive with unpushed work, frame 65), desktop/tests/live_flows/lifecycle.rs (real humatad2)

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2.

## Setup

- Native lifecycle frames: `session_lifecycle_and_full_screen_reference_images` in native/session.rs, one fixture per status (`fixture::design_session(status)`). Frames 01 to 09 cover Working, Needs you, Exited, Offline, and panels.
- Native archive: `archiving_offers_a_working_undo` (overlays.rs:61) archives "Currency codes" from fixture::design with a 1.4 GB size and clicks the toast's Undo. `archiving_unpushed_work_asks_first_as_in_frame_65` (fork_move.rs:82) opens the archive dialog for a session with unpushed work.
- Live: `archiving_a_session_from_its_menu_and_undoing_brings_it_back` (lifecycle.rs:33) on a real daemon with session "Fix the ledger"; `a_session_archived_from_another_window_closes_its_page_here` (lifecycle.rs:119) archives from a second client.
- Status enum: clients/src/model.rs:366. Labels (model.rs `Status::label`): Working, Needs you, Idle, Probably idle, Exited (code), Offline, Stopped, Moving.

## Steps

### 1. The session states on the sidebar dot and the header
- **Screen:** Sidebar dot per state (ui/src/sidebar.rs, ui/src/chrome.rs:40-50): Working is a green filled dot; Needs you is an orange dot; Idle and Probably idle and Moving are a hollow grey dot; Offline is a grey filled dot; Stopped is the `stopped()` mark; Exited has its own dot. The session header note (ui/src/pages.rs:215-240): "Needs you" (orange), "Agent exited" (red), "Moving to {machine}…" (orange), "{machine} is offline" (grey). Working, Idle, Probably idle and Stopped have no header note.
- **Offered:** Per state, the page actions as in steps 2 to 5.
- **Action:** Open each state's fixture.
- **Result:** The frames show each state. Working: 01 (light) and 02 (dark). Needs you: 03 (light, Inbox panel) and 04 (dark). Exited: 05 (dark, Info panel). Offline: 06 (Files panel). Each is in desktop/tests/__snapshots__/native/.
- **Evidence:** desktop/tests/__snapshots__/native/01-session-working-light.view.txt, 03-session-needs-you-light.view.txt, 05-session-exited.view.txt, 06-session-offline.view.txt (each has a PNG beside it)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=session_lifecycle_and_full_screen_reference_images`

### 2. Session with an agent that exited: the page
- **Screen:** Frame 05: the terminal log ends at the agent's last output; the header note is "Agent exited" in red (pages.rs:217). The buttons: `Resume agent` (bottom), `Fork`, `Expand terminal`, `Toggle side panel`, `Session actions`. The Info panel lists Agent, Machine, Started, Forks, Clones, "Survives restarts" ("humatad holds this terminal on devbox-1, not the app. After a reboot it runs claude --resume ... in the same folder.").
- **Offered:** Resume agent, Fork, Session actions, the panel toolbar, Channel rules (disabled in the frame).
- **Action:** Opened from the fixture.
- **Result:** The page offers Resume agent; no agent is running.
- **Evidence:** desktop/tests/__snapshots__/native/05-session-exited.view.txt
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=resuming_an_exited_session_keeps_its_open_panel`

### 3. Session actions, with the stop and archive items
- **Screen:** The session menu (frame session-menu-open, native/session-menu-open.view.txt) lists Rename…, Move to another machine…, Continue in another channel…, Add repo to this session…, New desk session for this session…, Stop the agent (not on a stopped session), Push and open a pull request, Archive session… (on a Desk session the last item reads "Close session"). The menu's items are asserted in native/session_menu.rs:14-24.
- **Offered:** Those items. Stop the agent is disabled for an Exited agent (pages.rs:442-444).
- **Action:** Click Session actions.
- **Result:** The menu opens with the items listed above (`session-menu-open`).
- **Evidence:** desktop/tests/__snapshots__/native/session-menu-open.view.txt
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_session_menu_renames_and_stops_the_open_session`

### 4. Stop the agent (one session)
- **Screen:** After Stop the agent the session's status is Exited (the native test asserts `Status::Exited`). The header note reads "Agent exited"; the menu's Stop item is disabled (session-menu-stopped-dark.view.txt).
- **Offered:** Resume agent, Fork, Session actions with Stop the agent now disabled.
- **Action:** Click Session actions, then Stop the agent.
- **Result:** The agent ends; the session reads as exited, not as Stopped. Resume agent brings it back (step 2). The CLI form is `humata session stop <session>` (args.rs:470), no confirm.
- **Evidence:** desktop/tests/__snapshots__/native/session-menu-stopped-dark.view.txt
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_session_menu_renames_and_stops_the_open_session`

### 5. Archive a session: the dialog when there is unpushed work
- **Screen:** Dialog "Archive session" with the text, and three buttons: Cancel, "Archive and delete the work", "Commit, push and open a PR, then archive" (desktop/tests/__snapshots__/native/archive-unpushed.view.txt). The Diff panel shows the changes the archive would lose.
- **Offered:** Cancel, Archive and delete the work, Commit, push and open a PR, then archive.
- **Action:** Session actions, Archive session…, on a session with unpushed changes.
- **Result:** Nothing archived until a choice is made. The live test chooses "Archive and delete the work" when the dialog appears (lifecycle.rs:60-62), because that choice needs no remote.
- **Evidence:** desktop/tests/__snapshots__/native/archive-unpushed.view.txt (and .png)
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=archiving_unpushed_work_asks_first_as_in_frame_65`

### 6. Archive with no unpushed work
- **Screen:** No dialog. The session leaves the sidebar at once.
- **Offered:** A toast with Undo.
- **Action:** Archive a session with nothing to lose.
- **Result:** Per the live test, the session is archived (the daemon lists it as archived) and the toast with an action appears.
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=archiving_a_session_from_its_menu_and_undoing`

### 7. The Undo toast after an archive
- **Screen:** Bottom toast: "Archived “Currency codes”. Frees 1.4 GB when this closes." with the button Undo (native/32-toast.view.txt). The toast uses the 10-second undo toast (features/app/src/lib.rs:694 and toast.rs:11).
- **Offered:** Undo. The toast also has a close.
- **Action:** Archive the fixture session.
- **Result:** The session is archived; the Undo toast shows.
- **Evidence:** desktop/tests/__snapshots__/native/32-toast.view.txt, 32-toast.png
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=archiving_offers_a_working_undo`

### 8. Undo within the window
- **Screen:** The session returns to the sidebar and the page.
- **Offered:** Normal session actions.
- **Action:** Click Undo on the toast.
- **Result:** The daemon restores the session (`RestoreSession`), it is not archived, and the sidebar lists it again. The native test asserts the session is unarchived after Undo; the live test checks the daemon lists it as not archived.
- **Evidence:** PENDING RUN for the live one; the native one is headless.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=archiving_a_session_from_its_menu_and_undoing`

### 9. Archived from another window
- **Screen:** The session's page closes here.
- **Offered:** None for the archived session.
- **Action:** A second client archives the session (`elsewhere.daemon.archive`).
- **Result:** The live test asserts the page's route is no longer the session (lifecycle.rs:119-148). The README records this as a bug to fix (item 26 in flows/cases/README.md), so this step may fail or pass depending on the gap.
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=a_session_archived_from_another_window_closes_its_page_here`

### 10. Undo after the toast is gone
- **Screen:** The toast leaves after 10 s. Nothing on the session's page offers a restore.
- **Offered:** None found in the code for an archived session (the README records that an archived session "cannot be restored or deleted from its page"; the daemon RPC exists).
- **Action:** Wait past the toast.
- **Result:** The archive stays. The only restore is in the 10 s toast.
- **Evidence:** PENDING RUN is optional; mark as code-read only in phase 1.
- **Replay check:** The README item 26 (flows/cases/README.md) is the gap record; no test.

## Not reachable headless

- Lifecycle frames 01 to 09 are headless, but the Stopped dot (chrome.rs `stopped()`) is not in the frames. Session Stopped (machine stopped) appears only in the stop record (X-I-stop-and-start.md).
- The archive "frees" size is a fixture value. The real size comes from the daemon.
- A session's archive when the app quits before the daemon finishes (gap 27) is not covered by any test here.

## Surprises

- A session stopped with the machine (Status::Stopped) has no header note and no words at all on the page; the dot is the only sign. Other states say "Needs you", "Agent exited", "{machine} is offline". The machine-level banner is the only words (see the stop record).
- Stop the agent makes the session read "Agent exited" (red), not "Stopped". The header note and the Resume agent button are the only trace; the stop is not named as the person's choice.
- The archive dialog's text says the archive "frees about {size} once the Undo window passes" (ui/src/overlay_archive.rs:31). The Undo toast says "Frees {size} when this closes." (features/app/src/lib.rs:691-692). Two phrases for the same number.
- The Undo window is 10 s in the toast and in the daemon's clone wait (daemon/humatad/src/session_service/archive.rs:40, UNDO_WINDOW_MS = 10,000). The README's item 28 says the toast goes after 4 s; the code at features/app/src/lib.rs:694 uses UNDO_TOAST (10 s). The gap may already be fixed. Check in phase 2 (see the PENDING RUN in step 7 timing).
- The live test for "archived from another window" expects the page to close (lifecycle.rs:119-148); README item 26 says it stays open (Delegate::Left is declared but never sent). The two disagree; a phase 2 run decides.
- Once the Undo toast is gone, nothing in the UI restores an archived session (README item 26), so a wrong archive is undone only within 10 seconds. The archive dialog is the only pause before it.
- The `humata session stop` and the app's Stop the agent take effect without a confirm; the machine-level stop has a confirm in the app and none in the CLI (see the stop record).
