# Desk: the Desk page, new desk session, the folder bar, close, keep, reopen, clean up, copy to a channel

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (desk::, desk_target::, channel_desk_target::, desk_events::, channel_desk_reopen::) and `//apps/humata2/desktop:live_flows_test` (desk::, desk_form::)

Phase 1 (map from code and snapshots, no Bazel run). Every step marked PENDING RUN is filled in phase 2.

## Setup

- Native: the design workspace with the Desk (`fixture::design()` then `fixture::desk()`), channels folded, via `desk_app()` in `desktop/tests/native/desk.rs`. Headless render, no window.
- Live: a real `humatad2` started by the live rig (`desktop/tests/live_flows/world.rs`), a fake `gh`, This Mac online.
- Desk sessions in the design fixture: `desk-plan-q4` (scratch, not kept), `desk-brin-rfc` (scratch, kept, on devbox-1, two repos), `desk-tidy-infra` (picked folder ~/notes), and closed rows `desk-release-checklist` (scratch, 6 days left), `desk-onboarding-doc` (scratch, kept), `desk-port-cron` (acme/api has 2 unpushed commits), `desk-notes-cleanup` (picked folder ~/notes).
- Snapshots used: `desktop/tests/__snapshots__/native/{36-desk,37-desk-board,38-desk-move,39-desk-document,40-new-desk-session,55-desk-clean-up}.view.txt` (and the PNGs beside them).

## Steps

### 1. Desk page (sidebar Desk)
- **Screen:** heading "Desk"; toolbar "Sort: last used", "Clean up…", "New desk session". Open rows with a time and a folder note: "Plan Q4 board" (now, scratch folder), "BRIN rollout RFC" (Today 12:05, scratch folder · kept), "Tidy infra items" (Nov 2, ~/notes). Documents list: `rfc-brin.md` and `q4-plan.md`, each with "Copy to a channel…". Closed rows: "Sketch the release checklist" (deleted in 6 days, Keep and Reopen), "Notes cleanup" (~/notes, Reopen), "Port the cron jobs" (not deleted · acme/api: 2 commits on port-cron never pushed, Reopen), "Draft the onboarding doc" (scratch folder · kept, Reopen).
- **Offered:** New desk session; Sort menu; Clean up…; click an open row (opens its session); Copy to a channel… on each document; Keep (only on a closed scratch folder with days left); Reopen on each closed row.
- **Action:** Click Desk in the sidebar.
- **Result:** Route becomes Desk. Keep appears only for `desk-release-checklist`, the one closed row with days left (`the_desk_page_lists_open_documents_and_closed`).
- **Evidence:** `desktop/tests/__snapshots__/native/36-desk.view.txt` (and `36-desk.png`).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=desk::` (includes `the_desk_page_lists_open_documents_and_closed`); expected: pass.

### 2. Sort menu
- **Screen:** the menu lists the sort choices; the button reads "Sort: last used" or "Sort: title".
- **Offered:** Last used, Title.
- **Action:** Click Sort, then Title.
- **Result:** Sort becomes Title and the menu closes.
- **Evidence:** none (no snapshot of the menu). The test only checks the state.
- **Replay check:** `--test_filter=desk::` (includes `sorting_by_title_closes_the_pop_up`); expected: pass.

### 3. New desk session form
- **Screen (from code, `ui/src/overlay_desk_new.rs`):** dialog "New desk session", subtitle "outside any channel". Rows: Title (placeholder "What is it for?"); Runs (agent tabs Claude Code, Codex, Copilot, OpenCode, Terminal); Machine (a menu, "Choose a machine" until one is picked); Folder (no choice, a caption: "Humata makes a folder for it · deleted 7 days after you close it unless it holds a saved version or you keep it"); Repos (chips, an "Add a repo…" button, hint "Your library first, then GitHub. Zero or more."); Agent access (two check boxes, "Read and change every channel's board" ticked by default, "Make channels and add machines (asks you each time when off)"). Buttons: Cancel, Start. The Machine row offers "Move it later with humata desk move" (see Surprises).
- **Offered:** Title field, agent tabs, Machine menu, Add a repo…, the two access check boxes, Cancel, Start.
- **Action:** New desk session on the Desk page, type "Plan Q4 board", pick This Mac, Start.
- **Result:** Start opens the new session's page. The form closes. One new Desk session exists in the workspace.
- **Evidence:** `desktop/tests/__snapshots__/native/40-new-desk-session.view.txt`. Note the snapshot's dialog differs from the code (see Surprises: it shows "A new scratch folder" and "A folder on the machine" radio buttons, which the code does not draw).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=a_new_desk_session_starts_from_the_form_and_opens`; expected: pass. The test does not assert the folder radios, so a pass does not settle the snapshot question. PENDING RUN: the snapshot compare in the same run decides it (see the Surprises entry).

### 4. Form: folder preview and a taken title (live)
- **Screen:** the live test expects the form to preview the folder the daemon will make (the field `scratch_preview`), and the view text does not carry that caption.
- **Offered:** the same form. "Add a repo…" chip, then Start.
- **Action:** Type a title, Start. A second session with a taken title.
- **Result:** The folder exists on disk under the Desk's folder. A taken title gets a suffixed folder (the form previews the suffix, the daemon makes that one).
- **Evidence:** none. PENDING RUN: the live run prints nothing extra; the test asserts on disk.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=desk_form::` (covers `the_new_desk_session_form_previews_the_folder_the_daemon_makes_and_clones_its_repo_chip` and `a_taken_title_previews_the_suffixed_folder_the_daemon_then_makes`); expected: pass.

### 5. Desk session page: folder bar and no Fork
- **Screen:** toolbar "Desk folder": Machine chip ("Machine: This Mac" or "Machine: devbox-1"), the folder path (`~/Humata/desk/plan-q4`), a chip per repo with its ✕ ("Remove acme/ledger"), "Add repo", and "Keep folder" for a scratch folder nobody kept. Session tabs: claude, zsh, file tabs, "New terminal". Side panels: Files (session folder list, with the caption "Repos you add are cloned into this folder. Removing one deletes its clone, not your branch."), Board, Info, Inbox, Diff, Journal. No Fork anywhere.
- **Offered:** Machine chip (move), Add repo, Keep folder, a repo chip's ✕, Session actions menu (per `ui/src/pages.rs`: "Move to another machine…", "Add repo to this session…", "New desk session for this session…", and "Close session" for a Desk session, not "Archive session…").
- **Action:** Open `desk-plan-q4` on the Board panel, open a file tab, select the first tab.
- **Result:** Folder bar present, `session-fork` absent.
- **Evidence:** `desktop/tests/__snapshots__/native/37-desk-board.view.txt`.
- **Replay check:** `--test_filter=desk::` (includes `a_desk_sessions_page_has_the_folder_bar_and_no_fork`); expected: pass. Live counterpart: `--test_filter=desk::` on live_flows_test (asserts "scratch" and "agent up" in the bar, no "Fork").

### 6. Move the Desk session to another machine from the bar
- **Screen:** the Machine chip ("Machine: devbox-1"). Its menu lists the other machines.
- **Offered:** each other machine (This Mac, gpu-box, ...).
- **Action:** Open the Machine chip, pick "This Mac".
- **Result:** The session's machine changes from devbox-1 to this-mac.
- **Evidence:** `desktop/tests/__snapshots__/native/38-desk-move.view.txt`. Its terminal log is not a real CLI transcript (see Surprises).
- **Replay check:** `--test_filter=desk::` (includes `a_desk_session_moves_to_another_machine_from_its_bar`); expected: pass.

### 7. Close a Desk session with nothing to save
- **Screen:** the Desk page, or the session's Session actions menu, "Close session".
- **Offered:** Close session.
- **Action:** Close `desk-plan-q4` from Info, or a session whose folder holds no files.
- **Result:** The page moves to Desk, the session is listed under Closed with days left. Its folder stays for now.
- **Evidence:** none in snapshots (the row appears in `36-desk.view.txt` as "deleted in 6 days" for a closed row).
- **Replay check:** `--test_filter=desk::` (includes `closing_a_desk_session_lists_it_under_closed`); expected: pass.

### 8. Close a scratch folder that holds files: the question
- **Screen (from code, `ui/src/overlay_desk_close.rs`):** dialog "Close session" with the title as subtitle. Secondary text "Save a version to keep the folder and its files." Buttons: Cancel, "Close", "Save version and close" (shows "Saving…" while it saves). On a save error an error banner (`desk-close-error`) shows and the dialog stays open.
- **Offered:** Cancel, Close (closes without saving), Save version and close.
- **Action:** Close the session from its menu (`desk-plan-q4` with files, and a version support folder).
- **Result, Save version and close:** a version is saved with an empty message, then the session closes. The dialog closes.
- **Result, Close:** the session closes, no version is saved; the record has no document head.
- **Evidence:** none. The dialog is not in any snapshot.
- **Replay check:** `--test_filter=desk::` (includes `closing_an_unkept_folder_with_files_asks_and_save_version_closes_it` and `close_in_the_question_closes_without_saving`); expected: pass.

### 9. Keep and Reopen a closed session
- **Screen:** a closed row: "Keep" (only when days left), "Reopen", and the note "deleted in N days".
- **Offered:** Keep, Reopen.
- **Action:** Keep on `desk-release-checklist`, then Reopen.
- **Result:** Keep clears the days left (the row loses "deleted in 6 days"). Reopen opens the session page; the agent resumes its conversation (live: the terminal shows "agent up --resume"). The session is open again on the Desk, with the same folder.
- **Evidence:** `36-desk.view.txt` (before the change).
- **Replay check:** `--test_filter=desk::` (includes `keeping_and_reopening_a_closed_session`); expected: pass. Live: `--test_filter=desk::` covers `closing_keeping_reopening_and_cleaning_up_from_the_desk_page` (close, keep, reopen, close again, clean up).

### 10. Clean up…
- **Screen (from `37`/`55` and `ui/src/overlay_desk_clean_up.rs`):** dialog "Clean up closed sessions", subtitle "Least recently used first". One row per closed session with a check box: scratch-folder sessions are ticked by default, picked folders are not. Footer: "Deleted sessions go to the trash for 30 days". Buttons: Cancel, "Delete N sessions · frees X MB".
- **Offered:** check boxes, Cancel, Delete.
- **Action:** Clean up…, tick or untick rows, Delete.
- **Result:** Ticked sessions are deleted; their scratch folders go to the trash. Unticked ones stay. The dialog closes.
- **Evidence:** `desktop/tests/__snapshots__/native/55-desk-clean-up.view.txt`. Snapshot: "Draft the onboarding doc" and "Sketch the release checklist" ticked, "Port the cron jobs" and "Notes cleanup" not ticked, button reads "Delete 2 sessions · frees 168 MB".
- **Replay check:** `--test_filter=desk::` (includes `cleaning_up_deletes_the_ticked_closed_sessions`); expected: pass. Live: the closing/keeping/cleaning test above; it asserts the folder is gone from disk after Clean up.

### 11. Copy a document to a channel…
- **Screen (from `ui/src/overlay_desk_copy.rs`):** dialog "Copy to a channel" with the file name as its header. Channel radio buttons ("# payments", "# web"); when the picked channel has several machines, a machine list too. Buttons: Cancel, Copy.
- **Offered:** each channel radio, Copy, Cancel.
- **Action:** On the Desk page, "Copy to a channel…" on `notes.md` (live) or `q4-plan.md` (native), pick "# web", Copy.
- **Result:** The file is copied into the channel's `shared/` folder. A toast says which file went where. The Desk keeps its own copy. The dialog closes.
- **Evidence:** none. No snapshot of the Copy dialog.
- **Replay check:** native `--test_filter=desk::` (includes `a_document_is_copied_into_a_channels_shared_folder`); live `--test_filter=desk::` (includes `a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`). Expected: pass.

### 12. Add a repo to a Desk session's folder (folder bar)
- **Screen (from `ui/src/overlay_desk_add_repo.rs`):** a small sheet "Add repo" with a "Repo to add" text field. Typed `owner/name`, Return clones the repo's default branch into the folder.
- **Offered:** the text field, Return, Cancel.
- **Action:** "Add repo" in the folder bar, type `owner/api`, Enter.
- **Result:** The repo is listed on the session; `api/` appears in the folder (`api/README.md` is present in the live test); the chip `owner/api` appears in the bar.
- **Evidence:** none. The code says no frame draws this field.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=desk::` (includes the Add repo step of `a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`); expected: pass.

## Not reachable headless

- Reopen of a closed channel Desk session: the fake daemon cannot reopen one (`desktop/tests/native/channel_desk.rs`, the DOCV note). Only the live path could drive it, and the live Desk test covers Reopen for a Desk (not channel) session.
- Moving a Desk session into a channel: there is no code path for it (see Surprises).
- The Clean up trash window (30 days) and the host's move of the folder to the Bin (D440): the live test checks the folder is gone, not what the Bin holds.

## Surprises

- **New desk session snapshot disagrees with the code.** `40-new-desk-session.view.txt` shows a "Folder" choice ("A new scratch folder" selected, "A folder on the machine") and "Add a repo…" as its own button. `ui/src/overlay_desk_new.rs` has no folder choice ("Humata makes a folder for it", DK-14 comment: "no folder choice and no path"). The native test does not assert the radios. PENDING RUN will show whether the snapshot compare passes or fails in that test.
- **Machine hint names a command that does not exist.** The form says "Move it later with humata desk move". `humata desk` has only `keep` (see `cli/src/args.rs`, `DeskCommand`). The move command is `humata session move <session> --to <machine>`.
- **The desk-move snapshot's terminal log uses commands that do not exist.** `38-desk-move.view.txt` shows `humata desk move devbox-1`, `humata desk add acme/ledger acme/api`, and `humata desk keep`. Only `humata desk keep` exists. The add is `humata session repo add <session> <repo>`. (The board snapshot `37-desk-board.view.txt` also shows `humata channel_create web-revamp repos=acme/web`, which is not the CLI spelling, which is `humata channel create <name> --repo <owner/name>`.)
- **No moving a Desk session into a channel.** The Desk bar moves a session only between machines (`overlay_move` / `desk_bar` machine chip). The only channel-facing action on the Desk is "Copy to a channel…", which copies a document, not the session. A channel's own Desk sessions are a separate list (`features/desk/src/channel.rs` header).
- **Two retention windows on one page.** Closed rows say "deleted in 6 days" (7 days after close). The Clean up dialog footer says "Deleted sessions go to the trash for 30 days". Both can be true (trash after delete), but the page does not say so.
- **Closing a session with no version closes at once, no question.** `close_asks` requires a scratch folder, a saved-version capability, and files in the folder. A folder made before versions (or with no files) closes with no dialog and no warning about the 7-day delete.
- **"Save version and close" saves with an empty message.** The optional "What changed?" field exists only on the history strip, not in the close question.
- **Add repo on the folder bar takes typed text.** It takes `owner/name` plus Return, while the form's repo row and the New channel sheet use the library-first search. The folder bar gives no list or search.
- **Unpushed work on a closed row is text only.** `Port the cron jobs` shows "acme/api: 2 commits on port-cron never pushed" on its row, is unticked in Clean up, and has a Reopen button. Nothing else on the page stops a delete.
