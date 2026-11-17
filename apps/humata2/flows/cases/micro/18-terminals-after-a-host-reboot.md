# After a Mac reboot, sessions come back in their conversations, and a lost one says so

Micro. macOS for the reboot and the launchd login start; the restart logic is tested on macOS and Linux. Not a web case.

## Starting state

- Humata is running on this Mac: the daemon (`humatad2`) and one holder per tab. The app is closed.
- Three sessions are running on this Mac, in channel "payments":
  - "Fix the ledger": its agent is mid-turn, writing a change.
  - "Cache notes": a plain shell tab (tab 1) in the session's folder, with some output on screen. No agent is running in this tab.
  - "Lost chat": its agent is idle, and its conversation file has been deleted from disk before the reboot.
- Nothing is typed. No request is waiting in the Inbox.

## Steps

1. **I do:** Restart the Mac from the Apple menu while the three sessions are running.
   - **I see:** The Mac goes down and comes back. Nothing in Humata is shown yet.
   - **State and events:** Every holder and the daemon die with the machine. The conversation files stay on disk. The store keeps its rows, and the saved scrollback of each holder stays in its socket folder.
   - **Status:** Works (real daemon). The reboot is simulated, not a real OS reboot. `daemon/humatad/tests/sessions.rs::a_reboot_brings_back_the_session_and_a_shell` kills the daemon and every holder at once, then starts the daemon again.

2. **I do:** Log in. The daemon should start by itself.
   - **I see:** Nothing in the app, because I have not opened it. The daemon is running in the background.
   - **State and events:** The launchd agent loads at login (`RunAtLoad` is true, `desktop/install/src/plist.rs:26-27`). It starts `humatad2 serve`, which brings back each saved session and writes a restart report.
   - **Status:** Fakes only. `desktop/install/tests/install.rs::first_install_copies_everything_and_registers` checks the install against a fake launchd. Nothing proves that a real login starts the daemon. Left to the UPGRADE lane (desktop/install).

3. **I do:** Open Humata.
   - **I see:** A toast reads "This Mac restarted: 1 session started again, 1 couldn't. Mid-turn agents are idle." Home lists "Couldn't resume after the restart · 1" with "Lost chat", and "Idle after the restart" for any idle session that came back.
   - **State and events:** The app reads `last_start` from the daemon and keeps the restart in the workspace (`features/app/src/restart.rs`, `clients/live/src/daemon.rs:65-80`). Only a restart with something to report is kept.
   - **Status:** Works (real daemon) for the toast and "Couldn't resume after the restart": `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`, with the daemon done before the app opens. "Idle after the restart" also Works (real daemon), with an agent that reports its turn through `humata turn`: `desktop/tests/live_flows/restart.rs::after_a_reboot_an_agent_that_reports_idle_is_listed_under_idle_after_the_restart`.

4. **I do:** Open "Fix the ledger".
   - **I see:** Under the old output, a line "This Mac restarted at <time>". The agent's output continues in its conversation. The agent has been told it was interrupted.
   - **State and events:** The daemon starts the agent with `--resume` on the same conversation id, in the same folder. The conversation gets one interruption note. The session's status after the start is Working in the daemon test, so it may not appear under "Idle after the restart".
   - **Status:** Works (real daemon). `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` checks the view shows `agent up --resume`, the conversation still has "before the reboot", and the session is not Exited or Offline. `daemon/revive/tests/kill_matrix.rs::a_killed_holder_is_resumed_with_its_conversation_and_its_output` checks the interruption note is written once. The desktop test is macOS only.

5. **I do:** Open "Cache notes".
   - **I see:** A new shell prompt in the same folder. Above it, the output from before the reboot, then the line "[Humata] This terminal stopped: its machine restarted or its holder was killed. Programs that were running in it are gone ... This is a new shell in <folder>."
   - **State and events:** The daemon spawns a new holder for the shell in the saved folder. The old scrollback is shown above the line. The tab keeps its id.
   - **Status:** Works (real shell). `daemon/revive/tests/kill_matrix.rs::a_shell_whose_holder_was_killed_comes_back_fresh_in_its_folder` checks the saved output shows `mark-2`, and that `pwd` in the new shell is the folder. It runs the revive code with a real holder and real `/bin/sh`, in a test binary, not through `humatad2`.

6. **I do:** Open "Lost chat".
   - **I see:** The session card reads "Couldn't resume", the reason ("claude can't find this conversation's file on This Mac. Nothing was typed into it."), with **Try again**, **Start fresh with a summary**, and a link "Restart report <time>".
   - **State and events:** The session's status is ResumeFailed. Nothing is started in its place, and no new conversation is created.
   - **Status:** Works (real daemon) for the card: `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` and `desktop/tests/live_flows/restart.rs::start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary` press its buttons. The model is tested on a real daemon: `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` (status ResumeFailed, nothing started), and `desktop/tests/live_flows.rs::a_session_whose_conversation_is_lost_cannot_resume` (model only). The card is drawn by `ui/src/session_tabs.rs:681-688` from `ui/src/resume_failed.rs:88`, and no test or golden checks its text.

7. **I do:** Choose **Start fresh with a summary** on "Lost chat".
   - **I see:** The session starts on a new conversation. Its first message is a summary of the old one. The card goes away.
   - **State and events:** `ResumeSession` with mode start-fresh. The new conversation id differs from the old one. The session id is the same.
   - **Status:** Works (real daemon). `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`.

8. **I do:** Click the "Restart report" link on the card.
   - **I see (code as written):** The app opens the Hosts list. The report is not shown there.
   - **State and events:** `features/app/src/restart.rs:108` ignores the machine and navigates to `Route::Hosts`. The report itself is a folder under the data folder's `restarts/` with `restart.json` and `report.md`.
   - **Status:** Not built. PER-29 asks for one line per failed session linking to the report. The restart report line is still unchecked in `OVERVIEW.md:537`. The link lands on the wrong page, see the gaps.

## Branches

### B1. One shell cannot come back

- **When:** The saved scrollback of "Cache notes" can't be read at start (its path is a folder, not a file).
- **I do:** Reboot as in step 1, then open the app.
- **I see:** The toast still reports the restart. "Fix the ledger" comes back. "Cache notes" does not, and the report has one line naming that tab and the problem.
- **Expected outcome:** One line for the lost shell; the other sessions come back and accept typing.
- **Status:** Works (real daemon). `daemon/humatad/tests/restart.rs::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`. The one line is not shown in the app (see step 8).

### B2. The state file is damaged and comes back from its last good copy

- **When:** `state.sqlite` is overwritten with junk while the daemon is down, and a last good copy exists.
- **I do:** Reboot as in step 1, then open the app.
- **I see (code as written):** The sessions come back from the copy. The app shows the usual restart toast. Nothing in the app says that a copy was restored or how far back it goes.
- **Expected outcome:** The start says the state came from a copy taken at a given time, and what was lost (rows changed).
- **Status:** Works (real daemon) for the daemon: `daemon/humatad/tests/restart.rs::a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`. The message in the app is Not built (PER-29; `OVERVIEW.md:537`). The fields `restored_copy_taken_unix_ms` and `restored_rows_changed` (`api/proto/humata/v1/daemon.proto:110-114`) reach the app only as a reason to show a restart toast (`clients/live/src/daemon.rs:74-75`).

### B3. I press Try again on "Lost chat"

- **When:** I restore the conversation file from a backup, then press **Try again** on the card. Or I press it while the file is still missing.
- **I do:** Press **Try again**.
- **I see (restored file):** The card goes away and the agent runs on its old conversation. **I see (file still missing):** The card stays with the same reason; nothing is started.
- **State and events:** The app sends `ResumeSession` with mode resume (`features/app/src/restart.rs`, `clients/live/src/daemon.rs:452-466`). The daemon's handler accepts a session in ResumeFailed (`daemon/humatad/src/session_service.rs:2099-2125`).
- **Expected outcome:** Resumes on the same conversation if the file is back. If not, fails again with the same reason. Nothing is started in its place.
- **Status:** Works (real daemon) for a restored file: `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`. The fake-daemon test is `features/app/tests/restart.rs::try_again_resumes_the_session`. No real-daemon test resumes a session from ResumeFailed, so the daemon side is Code, no test.

### B4. I open `humata doctor restarts`

- **When:** After the restart, in a terminal on this Mac.
- **I do:** Run `humata doctor restarts`, then `humata doctor restarts --explain`.
- **I see:** The newest report: what came back, what did not and why, and the repairs it offers. `--explain` writes a diagnosis file in the report folder and lists the repair ids.
- **State and events:** The CLI reads the report folder on disk. A safe repair runs only when allowed; a lossy one waits for `--allow`.
- **Status:** Fakes only for the CLI. `cli/tests/doctor.rs::explain_reads_the_newest_report_and_says_what_wasnt_brought_back` reads real report files but uses a fake daemon. The repair itself is Works (real daemon): `daemon/humatad/tests/doctor.rs::a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back`.

### B5. The Mac reboots again while "Fix the ledger" is resuming

- **When:** The daemon is killed at any of seven points during the resume: after the record is written, after the agent is spawned, while it is told about the interruption, and after it is verified.
- **I do:** Reboot a second time before the resume finishes.
- **I see:** After the next start, "Fix the ledger" is running again in its conversation, or it is shown as not resumed with a reason. No second agent is started.
- **State and events:** Revive finishes the resume on the next start, or records it as failed.
- **Expected outcome:** One agent, on the same conversation; no duplicate interruption note.
- **Status:** Works (real daemon). `daemon/revive/tests/kill_matrix.rs::a_daemon_killed_while_resuming_finishes_the_resume_when_it_starts_again`. The test kills real child processes and uses real holders, with the reconcile code in the test binary.

## Expected outcomes

- [ ] After the reboot, the daemon has brought back every session that had a holder, without anyone opening the app.
- [ ] An agent that was running is running again on the same conversation id, in its folder.
- [ ] A plain shell comes back as a new shell in its folder, with its earlier output above a line that says so.
- [ ] A session whose conversation file is gone shows "Couldn't resume" with a reason, and nothing runs in its place.
- [ ] Start fresh with a summary gives a new conversation id and the same session.
- [ ] A shell that can't come back gets one line in the report; the others still come back.
- [ ] A damaged state comes back from its last good copy, and the start says so.
- [ ] The restart report names what came back and what did not, and the app links to it.
- [ ] An idle agent is not told it was "mid-turn" (see the gaps).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Reboot brings sessions back (simulated) | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_reboot_brings_back_the_session_and_a_shell` |
| 2. Daemon starts at login | Fakes only | `desktop/install/tests/install.rs::first_install_copies_everything_and_registers` |
| 3. Toast and Home groups | Works (real daemon) | `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`; `desktop/tests/live_flows/restart.rs::after_a_reboot_an_agent_that_reports_idle_is_listed_under_idle_after_the_restart` |
| 4. Agent running again in its conversation | Works (real daemon) | `desktop/tests/live_flows.rs::after_a_reboot_the_session_is_running_again_in_its_conversation` |
| 5. Shell comes back as a new shell in its folder | Works (real shell) | `daemon/revive/tests/kill_matrix.rs::a_shell_whose_holder_was_killed_comes_back_fresh_in_its_folder` |
| 6. "Couldn't resume" card | Works (real daemon) | `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt`; `desktop/tests/live_flows/restart.rs::start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary` |
| 7. Start fresh with a summary | Works (real daemon) | `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` |
| 8. Restart report link and report in the app | Not built | PER-29; `OVERVIEW.md:537`; `features/app/src/restart.rs:108` |
| B1. One shell can't come back | Works (real daemon) | `daemon/humatad/tests/restart.rs::a_shell_that_cant_come_back_is_one_line_and_the_others_still_do` |
| B2. Damaged state from last good copy | Works (real daemon); app message Not built | `daemon/humatad/tests/restart.rs::a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so` |
| B3. Try again | Works (real daemon) | `desktop/tests/live_flows/restart.rs::after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt` |
| B4. `humata doctor restarts` | Fakes only (CLI); repair Works (real daemon) | `cli/tests/doctor.rs::explain_reads_the_newest_report_and_says_what_wasnt_brought_back`; `daemon/humatad/tests/doctor.rs::a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back` |
| B5. Second reboot during a resume | Works (real daemon) | `daemon/revive/tests/kill_matrix.rs::a_daemon_killed_while_resuming_finishes_the_resume_when_it_starts_again` |

Tag count, steps: Works (real daemon) 5 (steps 1, 3, 4, 6, 7), Works (real shell) 1 (step 5), Fakes only 1 (step 2), Not built 1 (step 8).
Tag count, branches: Works (real daemon) 4 (B1, B2, B3, B5). B4 has two parts: the CLI is Fakes only, and the repair is Works (real daemon).

## Gaps and likely bugs

- Likely bug: the Idle group note says "Was mid-turn when its host restarted. Tell it to continue." (`ui/src/restart_report.rs:153`). The group holds every session that is Idle and was among the resumed (`features/app/src/restart.rs:42-55`). The daemon adds every agent it starts again to `resumed` (`daemon/humatad/src/restart.rs:297-298`), and `LastStart` has no mid-turn field (`api/proto/humata/v1/daemon.proto:104-124`). So an agent that was idle before the reboot is told it was mid-turn. PER-17 says mid-turn agents come back idle. To confirm: reboot with one idle and one mid-turn session and read the group.
- Likely bug: the "Restart report" link goes to the Hosts list and drops the machine (`features/app/src/restart.rs:108`). To confirm: click it on a failed card.
- Likely bug: the status a resumed agent reports may be Working (the daemon test waits for Working, `daemon/humatad/tests/sessions.rs:264`) while the group says "Idle after the restart". The two should agree. To confirm: read the session's status right after a reboot in the app.
- Not built: the report of what did not come back, and the restored-copy message, are not in the app (PER-29; `OVERVIEW.md:537`).
- Not built: a login start for this computer on Linux (PER-18 asks for a systemd user service). `desktop/install` installs a launchd agent only; the systemd check in `daemon/machines/src/check.rs:78-80` is for remote machines.
- Code, no test: the "Couldn't resume" card and its Try again and Start fresh buttons (`ui/src/resume_failed.rs`, `ui/src/session_tabs.rs:681-688`). The Home restart groups (`ui/src/restart_report.rs`, rendered by `ui/src/lists.rs:519`) have no render test; only the grouping rule in `features/app/tests/restart.rs` is tested, on fakes.
- Code, no test: resuming a session from ResumeFailed on a real daemon (`daemon/humatad/src/session_service.rs:2099`).
- Doc conflict: `OVERVIEW.md:537` says "Couldn't resume" is not drawn. The code does draw it for tab 0 (`ui/src/session_tabs.rs:681-688`). The overview is out of date; the card has no golden.
