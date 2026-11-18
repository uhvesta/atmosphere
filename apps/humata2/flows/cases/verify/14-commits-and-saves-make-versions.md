# Each commit and each Save version makes a numbered version, and History says who made it

Layer: verify (whole-flow, Desk). Platforms: macOS first; Linux where the host runs; the web build shares the same UI.

This case covers flow 04 steps 9 and 10 (doc versions, DOCV). No macro case covers versions: `macro/04` predates them and says nothing about commits, Save version or History.

## Starting state

- Humata is open on Home. This Mac's daemon is running. No other machine is needed.
- A Desk session titled "Index rollout proposal" is open on the Desk. Its agent is Claude Code, on this Mac.
- Its folder is `<humata home>/desk/index-rollout-proposal/`. The folder is a git repository of its own, with no commit yet (made when the session started).
- The agent has written `rfc-index.md` in the folder. Nothing in it has been committed. The folder bar reads "Not kept · goes 7 days after close".
- `rfc-index.md` is open on a Markdown tab beside the agent's terminal, in Preview.
- I hold nothing on the clipboard. My own text editor is open on the same file, outside Humata.

## Reproduce headless

- Daemon, real `humatad2` in a temp folder, real git, fake agent. Run:
  `bazel test //apps/humata2/daemon/humatad:doc_history_test //apps/humata2/daemon/humatad:doc_comments_test //apps/humata2/daemon/humatad:doc_versions_test --test_output=all`
  Capture the output to a log file. Evidence: the pass lines for `a_file_lists_its_versions_newest_first_with_who_made_each`, `saving_with_nothing_changed_is_refused_and_a_change_shows_as_unsaved`, `an_agents_commit_tells_the_session_without_anyone_listing` and `clones_in_the_folder_are_left_out_of_its_versions`.
- Git rules, real git in a temp folder: `bazel test //apps/humata2/daemon/sessions:sessions_unit_test --test_arg=doc_repo --test_output=errors`. Evidence: the log for `save_commits_every_change_as_yours_and_never_the_clones` and `save_with_nothing_changed_is_refused`.
- Native view on fakes (the only headless render of the History strip): `bazel test //apps/humata2/desktop:native_test --test_arg=doc_versions --test_output=errors`. Evidence: a `.new.png` render in the snapshot folder if a render differs. The strip labels are not checked by this test.
- Live app on a real daemon, headless window: `bazel test //apps/humata2/desktop:live_flows_test --test_arg=desk_document`. It covers the document page and its comments, not the History strip. It is the nearest live check and is not a check of this case's version steps.
- The test stands in for the agent: it runs `git commit` in the session's folder, as an agent's plain commit would. Nothing touches `~/.claude`, `~/.codex`, `~/.copilot` or a git config. The test's folder is its own temp folder.
- What cannot be done headless: see the real window, and a real Claude Code CLI writing the commit. Nearest headless equivalents: the daemon test and the native render above.

## Steps

1. **I do:** in the agent's terminal, type "Commit the draft as a version each time you finish a rewrite." Return. The agent runs `git add rfc-index.md` and `git commit -m "Draft"` in its folder.  
   **I see:** git's one-line commit result in the terminal. Then, on the `rfc-index.md` tab, the History strip reads "v1 of 1", with its author shown as claude.  
   **State and events:** the folder's own repository gets one commit. The daemon records it as a version of `rfc-index.md`, and tells the session (Session updated). The version count for the folder is 1.  
   **Status:** Works (real daemon) for the commit being recorded and told: `daemon/humatad/tests/doc_comments.rs::an_agents_commit_tells_the_session_without_anyone_listing` and `daemon/sessions/src/doc_repo.rs::a_plain_git_commit_is_the_agents_version`. Likely bug for the strip label: after the commit it reads "Current", not "v1 of 1" (see Gaps, item 2).

2. **I do:** look at the folder bar above the tabs.  
   **I see:** the chip "Kept · 1 version". The folder is kept, so Close will not put it on a 7-day clock.  
   **State and events:** the folder's kind is kept once its repository holds a commit (DV-4). Nothing else changes.  
   **Status:** Likely bug. The chip reads "Not kept · goes 7 days after close" while the session is open. Only Close, Keep and clean up set the folder to kept (see Gaps, item 1). The chip's text is drawn in `ui/src/desk_bar.rs:256-259`, and no test checks it.

3. **I do:** in my own editor, change one sentence in `rfc-index.md` and save the file. Humata has no Markdown editor, so the edit happens outside the app.  
   **I see:** in the History strip, "Current · unsaved changes". Save version is enabled, and its message box is empty.  
   **State and events:** the file on disk differs from the newest version. Nothing is committed yet.  
   **Status:** Fakes only for the strip label and the enabled button: `features/session/tests/history.rs::save_version_is_off_without_a_change_and_on_after_one`. Works (real daemon) for the daemon seeing the change as unsaved: `daemon/humatad/tests/doc_history.rs::saving_with_nothing_changed_is_refused_and_a_change_shows_as_unsaved`. Editing Markdown inside Humata is Not built (SE-17).

4. **I do:** click **Save version**, type "Before review" in the message box, and click Save.  
   **I see:** the box clears, and a new version appears at the top of History. The strip should read "v2 of 2" (see Gaps, item 2 for what it shows).  
   **State and events:** the daemon commits every change in the folder outside its clones, as a version by you, with my message. The session is told (Session updated). The version count is 2.  
   **Status:** Works (real daemon) for the save: `daemon/sessions/src/doc_repo.rs::save_commits_every_change_as_yours_and_never_the_clones` and `daemon/humatad/tests/doc_versions.rs::clones_in_the_folder_are_left_out_of_its_versions`. Fakes only for the button and the box: `desktop/tests/native/doc_versions.rs::save_version_needs_a_change_since_the_newest_version`. Likely bug for the label "v2 of 2" (Gaps, item 2).

5. **I do:** click **History ▾**.  
   **I see:** the list, newest first: v2 by you with "Before review", then v1 by claude. Each row shows its time, its message and its lines added and removed.  
   **State and events:** the list is read from the folder's own repository. Nothing is written.  
   **Status:** Works (real daemon) for the data: `daemon/humatad/tests/doc_history.rs::a_file_lists_its_versions_newest_first_with_who_made_each`. The drawn popup (`ui/src/doc_history.rs:362-398`) has no test: Code, no test.

6. **I do:** with nothing changed since v2, look at Save version.  
   **I see:** Save version is disabled, and says "No changes since v2".  
   **State and events:** no write. The daemon would refuse a save here anyway.  
   **Status:** Fakes only for the disabled button and its hint: `desktop/tests/native/doc_versions.rs::save_version_needs_a_change_since_the_newest_version` and `features/session/tests/history.rs::save_version_is_off_without_a_change_and_on_after_one`. Works (real daemon) for the refusal: `daemon/sessions/src/doc_repo.rs::save_with_nothing_changed_is_refused`.

7. **I do:** ask the agent for a rewrite and a commit: "Rewrite the risks section and commit it."  
   **I see:** the agent commits. The History list gains v3 by claude at the top, without my reloading the tab.  
   **State and events:** a third commit in the folder. The session hears about it. The version count is 3.  
   **Status:** Works (real daemon) for the commit told to the session: `daemon/humatad/tests/doc_comments.rs::an_agents_commit_tells_the_session_without_anyone_listing`. Fakes only for the list refreshing: `features/session/tests/history.rs::an_agent_commit_moves_doc_head_and_the_versions_are_listed_again`.

8. **I do:** look at the folder bar's chip again.  
   **I see:** "Kept · 3 versions".  
   **State and events:** the folder is kept while open, and Close will not ask about it or start a 7-day clock.  
   **Status:** Likely bug. The chip shows "Not kept · goes 7 days after close" (see Gaps, item 1). The kept state is only set at close: `daemon/humatad/tests/doc_versions.rs::a_saved_version_keeps_the_folder_past_its_days` checks kept after a close, not while open.

## Branches

### Save version with nothing changed

- **When:** after step 4, with no edit since v2.
- **I do:** press Save version, or press Return in the empty message box.
- **I see:** nothing happens. The button is disabled (step 6), so there is nothing to press.
- **Expected:** no version is made. If a save reaches the daemon anyway, it is refused with a reason, and no version is added.
- **Status:** Fakes only for the disabled button (`desktop/tests/native/doc_versions.rs::save_version_needs_a_change_since_the_newest_version`). Works (real daemon) for the refusal (`daemon/sessions/src/doc_repo.rs::save_with_nothing_changed_is_refused`).

### Save with an empty message

- **When:** after a change, I click Save version and leave the box empty.
- **I do:** click Save version with the box empty.
- **I see:** a version is added, with the default note "Saved by you" in History.
- **Expected:** one version by you, with the default note.
- **Status:** Code, no test. The default note is `DEFAULT_NOTE` in `daemon/humatad/src/desk_service/versions.rs:24`. I searched the daemon's tests for that note and found no test that checks it. The save itself is Works (real daemon), per step 4.

### A commit made while the daemon is stopped

- **When:** the agent commits while this Mac's daemon is not running.
- **I do:** start the daemon again and open the session.
- **I see:** the commit appears in History as a version by claude.
- **Expected:** the commit is recorded at start, with its author and message.
- **Status:** Works (real daemon). `daemon/humatad/tests/doc_comments.rs::a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts`.

### Files in the clones are not in a version

- **When:** a repo is cloned into the session's folder, and I save a version.
- **I do:** click Save version with a change in the cloned repo and a change to `rfc-index.md`.
- **I see:** the version holds `rfc-index.md` only. The clone's files are not in it.
- **Expected:** a version holds every change outside the clones, and never a clone's files.
- **Status:** Works (real daemon). `daemon/humatad/tests/doc_versions.rs::clones_in_the_folder_are_left_out_of_its_versions`.

## Expected outcomes

- [ ] The agent's plain `git commit` in the folder becomes version N by claude, and the session hears of it without a list call.
- [ ] Save version commits every change outside the clones, as a version by you, with my message.
- [ ] With nothing changed, Save version is disabled and says why, and the daemon refuses a save.
- [ ] History lists versions newest first, each with who made it, its time, its message and its lines added and removed.
- [ ] Clones inside the folder are never part of a version.
- [ ] After a commit or Save version, the strip shows the newest version's number ("v2 of 2", "v3 of 3"). Not yet true (Likely bug 2).
- [ ] A folder that holds a version reads "Kept · N versions" while the session is open. Not yet true (Likely bug 1).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Agent commits a draft as v1 | Works (real daemon); Likely bug for the label | `daemon/humatad/tests/doc_comments.rs::an_agents_commit_tells_the_session_without_anyone_listing`; `daemon/sessions/src/doc_repo.rs::a_plain_git_commit_is_the_agents_version` |
| 2. Folder chip reads "Kept · 1 version" | Likely bug | `ui/src/desk_bar.rs:256`; `daemon/humatad/src/desk_service/view.rs:106-110` |
| 3. My edit makes the strip "Current · unsaved changes" | Fakes only; daemon Works (real daemon) | `features/session/tests/history.rs::save_version_is_off_without_a_change_and_on_after_one`; `daemon/humatad/tests/doc_history.rs::saving_with_nothing_changed_is_refused_and_a_change_shows_as_unsaved` |
| 4. Save version with a message makes v2 | Works (real daemon); Likely bug for the label | `daemon/sessions/src/doc_repo.rs::save_commits_every_change_as_yours_and_never_the_clones`; `daemon/humatad/tests/doc_versions.rs::clones_in_the_folder_are_left_out_of_its_versions` |
| 5. History ▾ lists newest first | Works (real daemon) for data; Code, no test for the popup | `daemon/humatad/tests/doc_history.rs::a_file_lists_its_versions_newest_first_with_who_made_each` |
| 6. Save version disabled with nothing changed | Fakes only (button); Works (real daemon) (refusal) | `desktop/tests/native/doc_versions.rs::save_version_needs_a_change_since_the_newest_version`; `daemon/sessions/src/doc_repo.rs::save_with_nothing_changed_is_refused` |
| 7. Agent commit adds v3 without reload | Works (real daemon) for the event; Fakes only for the list | `daemon/humatad/tests/doc_comments.rs::an_agents_commit_tells_the_session_without_anyone_listing`; `features/session/tests/history.rs::an_agent_commit_moves_doc_head_and_the_versions_are_listed_again` |
| 8. Chip reads "Kept · 3 versions" while open | Likely bug | `ui/src/desk_bar.rs:256`; `daemon/humatad/src/desk_service/close.rs:65-67` |
| Branch: Save version with nothing changed | Fakes only (button); Works (real daemon) (refusal) | as step 6 |
| Branch: empty message | Code, no test | `daemon/humatad/src/desk_service/versions.rs:24` |
| Branch: commit while the daemon is stopped | Works (real daemon) | `daemon/humatad/tests/doc_comments.rs::a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts` |
| Branch: clones left out of versions | Works (real daemon) | `daemon/humatad/tests/doc_versions.rs::clones_in_the_folder_are_left_out_of_its_versions` |

## Gaps and likely bugs

Not built:

- Editing a Markdown file inside Humata. Step 3 is done in my own editor. The Markdown editor is drawn disabled (`ui/src/panel_files.rs:306`). Milestone: SE-17.

Likely bugs:

1. **The folder reads "Not kept" while open, although it holds a version.** The kept flag is set only when a session closes (`daemon/humatad/src/desk_service/close.rs:65-67`), when it is kept by hand (`daemon/humatad/src/desk_service.rs:486`), or in clean up (`daemon/humatad/src/desk_service/clean_up.rs:257`). `desk_pb` in `daemon/humatad/src/desk_service/view.rs:106-110` builds the kind from that stored flag only. The chip in `ui/src/desk_bar.rs:256` matches `kept: false` first and ignores the version count. SPEC DV-4 (`apps/humata2/SPEC.md:1995`) says a folder is kept once its repository holds a commit. Confirm: start a Desk session, commit by hand in its folder, and read the chip on the folder bar and the Desk row. Expect "Kept · 1 version"; the code shows "Not kept · goes 7 days after close".
2. **After a commit, Save version or a restore, the strip says "Current", not "v N of N".** The label is "v N of M" only when a version is being viewed (`ui/src/doc_history.rs:228-234`). The save and restore handlers reload the list and keep `Position::Current` (`features/session/src/history.rs:451-466` and `:491-500`). The flow expects "v2 of 2" after a save, "v1 of 1" after a commit, and "v4 of 4 · You · Restored v2" after a restore. Confirm: save a version with the strip on Current and read the label. No test checks the strip labels.

Gaps in proof:

- The strip, Save version and the chip are proven only on fakes, and no live app test clicks them.
- The chip "Kept · N versions" has no test (`ui/src/desk_bar.rs:256-259`).
- The History popup's drawing (`ui/src/doc_history.rs:362-398`) has no test.
- The name `has_versions` means "is a Humata repository", not "holds a version" (`daemon/humatad/src/desk_service/view.rs:44-58`). It feeds the close question (H-3, item 3). Rename it or fix the meaning.
