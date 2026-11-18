# Closing a Desk session that holds files asks first, and Save version and close keeps the folder

Layer: verify (whole-flow, Desk). Platforms: macOS first; Linux where the host runs; the web build shares the same UI.

This case covers the DOCV parts of flow 01 step 9 (the Close question with Save version and close) and flow 04 step 1 (a new Desk folder is its own git repository with no commit). Macro 01 covers Close, Keep and Reopen, but not the question. No macro case covers the question.

## Starting state

- Humata is open on Home. This Mac's daemon is running.
- Nothing else is open on the Desk. `<humata home>/desk/` has no folder called `notes-on-checkout-errors`.
- My global git config holds my own settings. Humata must not change it.
- I hold nothing on the clipboard.

## Reproduce headless

- Daemon, real `humatad2` in a temp folder, real git, fake agent. Run:
  `bazel test //apps/humata2/daemon/humatad:desk_test //apps/humata2/daemon/humatad:doc_versions_test --test_output=all`
  Evidence: pass lines for `closing_stops_the_agent_and_keeps_the_folder_for_seven_days`, `kept_and_open_folders_stay_past_the_days`, `reopening_resumes_the_conversation_in_the_same_folder`, `reopening_is_refused_when_the_folder_is_gone` and `a_saved_version_keeps_the_folder_past_its_days`.
- Real git in a temp folder, with a sandboxed global config: `bazel test //apps/humata2/daemon/sessions:sessions_unit_test --test_arg=doc_repo --test_output=errors`. Evidence: the log for `init_makes_a_repo_with_no_version_and_leaves_the_global_config_alone`, `save_with_nothing_changed_is_refused` and `restore_adds_a_version_and_keeps_every_old_one`.
- The question and its buttons on fakes: `bazel test //apps/humata2/features/desk:close_confirm_test --test_output=errors`. Evidence: the log for `an_unkept_folder_holding_files_asks_before_closing`, `save_version_and_close_saves_then_closes`, `a_failed_save_keeps_the_question_open_and_the_session_open` and `cancelling_the_question_closes_nothing`.
- Native render on fakes: `bazel test //apps/humata2/desktop:native_test --test_arg=closing_an_unkept_folder --test_output=errors` and `--test_arg=close_in_the_question`. Evidence: a `.new.png` if the question's render differs.
- Live client on a real daemon: `bazel test //apps/humata2/clients/live:desk_live_test --test_arg=closing_and_reopening_on_real`.
- The question's buttons (Save version and close, Close, Cancel) are not clicked on a real daemon by any live test. The nearest live check is the daemon test above.
- Isolation: the git test sets HOME and the global config to a sandbox (`daemon/sessions/src/doc_repo.rs`, the sandbox near line 727). The daemon test uses its own temp folder. Nothing touches `~/.claude`, `~/.codex`, `~/.copilot` or a git config.
- Evidence to keep: test logs, and for the folder check, `git -C <test folder> log --oneline` (read only) and `git config --global --list` before and after (must be unchanged).
- What cannot be done headless: see the real window, and a real agent CLI writing `notes.md`. The test writes the file itself.

## Steps

1. **I do:** click **New desk session**, title it "Notes on checkout errors", leave Runs on Claude Code and Machine on This Mac, and click Start. The agent writes `notes.md` in the folder.  
   **I see:** the session page. The folder bar reads "Not kept · goes 7 days after close". The Desk page lists `notes.md` under Documents.  
   **State and events:** the folder `<humata home>/desk/notes-on-checkout-errors` is created as a git repository of its own, with no commit. Humata writes only that repository's local config. `notes.md` is a file outside the clones.  
   **Status:** Works (real daemon) for the folder being a repository with no version: `daemon/humatad/tests/doc_versions.rs::a_new_desk_folder_is_a_repository_with_no_version_yet`. Works (real daemon, real git) for the global config staying untouched: `daemon/sessions/src/doc_repo.rs::init_makes_a_repo_with_no_version_and_leaves_the_global_config_alone`. Works (real daemon) for the listing: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`.

2. **I do:** open the session's menu and pick **Close session**.  
   **I see:** a one-line question card titled "Close session", with three buttons: Cancel, Close, and Save version and close. Nothing has closed. The agent is still running, and the session is still open.  
   **State and events:** no close is sent. The dialog state holds the session. The folder is not kept, and it holds files outside its clones and has a repository, so the question is asked (DV-4).  
   **Status:** Fakes only: `features/desk/tests/close_confirm.rs::an_unkept_folder_holding_files_asks_before_closing` and `desktop/tests/native/desk.rs::closing_an_unkept_folder_with_files_asks_and_save_version_closes_it`. The daemon's signal that the folder holds files: Works (real daemon), `daemon/humatad/tests/doc_history.rs::the_desk_session_holds_files_once_a_file_is_outside_its_clones`.

3. **I do:** click Cancel.  
   **I see:** the question closes. The session stays open, and the agent keeps running.  
   **State and events:** nothing is written. No version is made.  
   **Status:** Fakes only: `features/desk/tests/close_confirm.rs::cancelling_the_question_closes_nothing`.

4. **I do:** open the menu again, pick Close session, then click Save version and close.  
   **I see:** the button reads "Saving…". Then the session moves to Closed. Its folder shows no days left, and the Desk row shows no delete time.  
   **State and events:** one version is committed by you, with the default note, holding `notes.md`. The agent is stopped, and the session's closed time is set. The folder is kept, because it holds a version, so its delete time is cleared.  
   **Status:** Works (real daemon) for the version, the close and the keep: `daemon/humatad/tests/doc_versions.rs::a_saved_version_keeps_the_folder_past_its_days` (it closes a session whose folder holds a version and checks it is kept with no delete time). Fakes only for the button: `features/desk/tests/close_confirm.rs::save_version_and_close_saves_then_closes`. Note: that test uses a fake that accepts the save every time (see branch 1).

5. **I do:** open the Desk page and look at the closed row.  
   **I see:** the row is under Closed, with no days left. Reopen is on the row.  
   **State and events:** nothing is written.  
   **Status:** Works (real daemon, live client): `clients/live/tests/desk.rs::a_kept_folder_has_no_days_left_on_real`.

6. **I do:** click Reopen.  
   **I see:** the agent resumes with its conversation, in the same folder, and the session is open again.  
   **State and events:** the folder path is unchanged. The closed time and the delete time are cleared.  
   **Status:** Works (real daemon): `daemon/humatad/tests/desk.rs::reopening_resumes_the_conversation_in_the_same_folder`.

## Branches

### Close a session whose files are all already versions (Likely bug)

- **When:** the agent committed `notes.md` as a version, I changed nothing since, and the session is still open. Then I close it.
- **I do:** pick Close session, then click Save version and close.
- **I see:** the question appears (it asks, because the folder is not kept while open, holds files, and has a repository). Then the save is refused with a reason: change a file in the folder, then save a version. The question stays open with the error, and the session stays open.
- **Expected (DV-4):** a folder whose repository holds a commit is kept, so Close does not ask and closes at once. If it does ask, Save version and close should close the session, since the folder's versions already hold its files.
- **Status:** Likely bug. The question is asked on `record.has_versions`, which is set for any Humata repository, not for a version (`features/desk/src/close.rs:33-37`, `daemon/humatad/src/desk_service/view.rs:44-58`). The kept flag is still not set while open (H-1, item 1), so the question asks. Save version and close then calls `save` with nothing changed, which is refused (`features/desk/src/close.rs` save_and_close; `daemon/sessions/src/doc_repo.rs:615-618`; the message at `daemon/humatad/src/desk_service/versions.rs:446-449`). The fakes accept every save, so no fake test sees the refusal. Confirm: on a real daemon, commit `notes.md` by hand in the folder, change nothing, then pick Close session and click Save version and close. Expect the closed row; the code shows the refusal.

### Commit by hand while open, then read the chip

- **When:** while the session is open, I commit `notes.md` by hand in its folder.
- **I do:** commit in the folder's repository (with the agent's terminal or outside), then read the folder bar.
- **I see:** the chip reads "Not kept · goes 7 days after close", not "Kept · 1 version".
- **Expected (DV-4):** "Kept · 1 version".
- **Status:** Likely bug. `ui/src/desk_bar.rs:256` matches `kept: false` first and ignores the count, and the open session's kind is never set to kept (H-1, item 1; `daemon/humatad/src/desk_service/view.rs:106-110`).

### Save version and close fails in the daemon

- **When:** the save reaches the daemon and fails (for example, the folder is unreadable).
- **I do:** click Save version and close.
- **I see:** the question stays open, with the error shown, and the session stays open. I can click Close to close without saving, or Cancel.
- **Expected:** nothing closes on a failed save. The error is shown and the session is still open.
- **Status:** Fakes only: `features/desk/tests/close_confirm.rs::a_failed_save_keeps_the_question_open_and_the_session_open`.

### Close without saving

- **When:** the question is open, and I click Close.
- **I do:** click Close.
- **I see:** the session moves to Closed. Its folder has no version, so it is due for the trash in 7 days.
- **Expected:** the agent stops, the folder stays for 7 days, and the notes stay in the folder.
- **Status:** Fakes only for the question's button: `features/desk/tests/close_confirm.rs::close_closes_without_saving`. Works (real daemon) for the 7 days: `daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days`.

### A kept folder closes without asking

- **When:** I have kept the folder with Keep folder before closing.
- **I do:** pick Close session.
- **I see:** no question. The session closes at once.
- **Expected:** closed at once, and the folder stays.
- **Status:** Fakes only: `features/desk/tests/close_confirm.rs::a_kept_folder_closes_without_asking`. Works (real daemon): `daemon/humatad/tests/desk.rs::kept_and_open_folders_stay_past_the_days`.

### Reopen after the folder is gone

- **When:** the session is closed with no version, and its folder was trashed after 7 days.
- **I do:** click Reopen on the closed row.
- **I see:** Reopen is refused, and the reason says the folder is gone.
- **Expected:** the session stays closed, and nothing is created.
- **Status:** Works (real daemon): `daemon/humatad/tests/desk.rs::reopening_is_refused_when_the_folder_is_gone`.

## Expected outcomes

- [ ] A new Desk folder is a git repository with no commit, and Humata writes only that repository's local config.
- [ ] Close session on an unkept folder that holds files asks first, with Cancel, Close and Save version and close.
- [ ] Cancel closes nothing and leaves the agent running.
- [ ] Save version and close makes one version, closes the session, and keeps the folder with no delete time.
- [ ] Close alone closes the session, and the folder goes to the trash 7 days later.
- [ ] A kept folder closes without a question.
- [ ] Reopen resumes the same conversation in the same folder, and is refused once the folder is gone.
- [ ] A folder whose files are all already versions closes without an error. Not yet true (Likely bug 1 in the branch).
- [ ] While open, a folder with a version reads "Kept · N versions". Not yet true (H-1, item 1).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. New folder is a repository with no version | Works (real daemon); global config untouched: Works (real daemon, real git) | `daemon/humatad/tests/doc_versions.rs::a_new_desk_folder_is_a_repository_with_no_version_yet`; `daemon/sessions/src/doc_repo.rs::init_makes_a_repo_with_no_version_and_leaves_the_global_config_alone` |
| 2. Close asks with three buttons | Fakes only; daemon signal Works (real daemon) | `features/desk/tests/close_confirm.rs::an_unkept_folder_holding_files_asks_before_closing`; `daemon/humatad/tests/doc_history.rs::the_desk_session_holds_files_once_a_file_is_outside_its_clones` |
| 3. Cancel closes nothing | Fakes only | `features/desk/tests/close_confirm.rs::cancelling_the_question_closes_nothing` |
| 4. Save version and close makes a version, closes, keeps | Works (real daemon) for the version and the keep; Fakes only for the button | `daemon/humatad/tests/doc_versions.rs::a_saved_version_keeps_the_folder_past_its_days`; `features/desk/tests/close_confirm.rs::save_version_and_close_saves_then_closes` |
| 5. Closed row shows no days left | Works (real daemon, live client) | `clients/live/tests/desk.rs::a_kept_folder_has_no_days_left_on_real` |
| 6. Reopen resumes in the same folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::reopening_resumes_the_conversation_in_the_same_folder` |
| Branch: all files already versions, Save version and close | Likely bug | `features/desk/src/close.rs:33-37`; `daemon/sessions/src/doc_repo.rs:615-618` |
| Branch: commit by hand, chip reads "Not kept" | Likely bug | `ui/src/desk_bar.rs:256` |
| Branch: save fails, question stays open | Fakes only | `features/desk/tests/close_confirm.rs::a_failed_save_keeps_the_question_open_and_the_session_open` |
| Branch: Close without saving | Fakes only (button); Works (real daemon) (7 days) | `features/desk/tests/close_confirm.rs::close_closes_without_saving`; `daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days` |
| Branch: kept folder closes without asking | Fakes only; Works (real daemon) | `features/desk/tests/close_confirm.rs::a_kept_folder_closes_without_asking`; `daemon/humatad/tests/desk.rs::kept_and_open_folders_stay_past_the_days` |
| Branch: reopen after the folder is gone | Works (real daemon) | `daemon/humatad/tests/desk.rs::reopening_is_refused_when_the_folder_is_gone` |

## Gaps and likely bugs

Not built:

- None in these steps. Moving the Desk session into a channel (flow 01 step 8, ROAM) is Not built, but it is outside this case.

Likely bugs:

1. **Save version and close is refused when every file is already a version.** The question asks whenever the folder has a repository and holds files, and the folder is not kept while open (H-1, item 1). Save version and close then saves with nothing changed, which the daemon refuses (`daemon/sessions/src/doc_repo.rs:615-618`; reason text at `daemon/humatad/src/desk_service/versions.rs:446-449`). The user is left with an error and an open session. Confirm on a real daemon as in the branch above. The fix that makes the folder kept on its first commit (H-1, item 1) also stops the question for this case, but the save path should close when nothing is unsaved.
2. **The question's test uses a fake that always saves.** `features/desk/tests/close_confirm.rs::save_version_and_close_saves_then_closes` cannot see the refusal in item 1. Add a test against the daemon for a folder with a commit and no change.
3. **`has_versions` means "is a Humata repository".** `daemon/humatad/src/desk_service/view.rs:44-58` returns true for any repository Humata made, with or without a version. The question depends on it (`features/desk/src/close.rs:33-37`). The name hides the bug in item 1.

Gaps in proof:

- The question's three buttons are never clicked against a real daemon. Only the close, keep and reopen daemon tests are real.
- The save-failure path (the question stays open with the error) is proven on fakes only.
- The question's wording, and its one-line layout, are checked only in the native render, not in the live app.
