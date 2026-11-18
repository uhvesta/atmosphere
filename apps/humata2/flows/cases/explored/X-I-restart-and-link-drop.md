# Restart and link drop: what a reboot brings back, the restart report, doctor, and a dropped link

Layer: explored · macOS · harness: desktop/tests/live_flows/restart.rs and link_drop.rs (real humatad2, World::reboot), desktop/tests/live_flows/away_box.rs (the link drop with boxes), desktop/tests/native/session.rs (away-left-out frames), desktop/tests/design/ (frames 46 and 47, PNG only), cli/tests/doctor.rs (fake daemon, report folders on disk)

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2.

## Setup

- Live restart: `World::start()` for the daemon, `running_session(...)` for "Read path" (line "kept-line"), `another_session(...)` for "Old spike" (line "lost-line"). The second conversation file is deleted (`conversation_with`), then `world.reboot()`; the app opens after the daemon has brought its sessions back (`reboot_and_settle`).
- Live link drop: `World::start_with_machine()` (host + devbox-1), session "Fix the ledger" on devbox-1, a shell tab on devbox-1. `world.devbox().cut_link()` and `restore_link()`.
- Doctor: cli/tests/doctor.rs builds report folders under a temp state dir (`state(operations)`), with restart.json and operations.jsonl, and a FakeDaemon for the repairs.
- Throwaway daemon for PENDING RUN steps. From the repo root, with the scratchpad as `S`:
  `export PATH=$PWD/.tools/bin:$PATH; bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2`
  Then `HOME=$S/home humatad2 serve --socket $S/sock --state $S/state --root $S/root --holder <holder binary from bazel-bin, see daemon/humatad/tests/common/mod.rs HUMATA2_HOLDER> --shell /bin/sh --shell-arg=-i` (the flags come from daemon/humatad/tests/common/mod.rs lines 223-245), and `HUMATA_SOCKET=$S/sock HOME=$S/home bazel-bin/apps/humata2/cli/humata <command>`. Stop the daemon when done. Never the owner's ~/.humata2.

## Steps

### 1. Reboot: the app says what came back
- **Screen:** A toast (4 s, the default toast) reads "{machine} restarted: N sessions started again, M couldn't." and, when N > 0, " Mid-turn agents are idle." Source: features/app/src/restart.rs:77-88 (`told`). Home then shows the groups in step 2.
- **Offered:** The toast has no action. Dismiss only by time.
- **Action:** Reboot the daemon host (World::reboot) and open the app.
- **Result:** The restart is known before the app's first load (`reboot_and_settle`), so the app learns of it from the load (`loaded_restarts`, D390), not from a live event.
- **Evidence:** PENDING RUN (the toast text is asserted only as non-empty in the live test)
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=after_a_reboot_the_app_says_what_came_back`

### 2. Home groups the sessions after the restart
- **Screen:** Above the usual groups: "Couldn't resume after the restart · N" with one row per session, each with the note "Couldn't resume. Open it to try again or start fresh with a summary.". Below it: "Idle after the restart · N" with the note "Came back idle after its host restarted. If it was working, tell it to continue.". The row shows the status, title, and "{host} restarted at {time}" (ui/src/restart_report.rs:137-153, ui/src/resume_failed.rs:73).
- **Offered:** Clicking a row opens that session (role Link). No other action on Home.
- **Action:** Navigate to Home after the restart.
- **Result:** The session that could not resume is listed only under its group; the one that came back is not listed there. `restart_groups` splits them (features/app).
- **Evidence:** desktop/tests/__snapshots__/design/47-home-restart-report.png (design golden, PNG only; no .view.txt exists for design frames). Frame 46 is design/46-session-couldnt-resume.png.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=after_a_reboot_the_app_says_what_came_back`

### 3. Open a session that couldn't resume: the card
- **Screen:** The session page shows the Couldn't resume card: "{host} restarted at {time}", with buttons "Try again", "Start fresh with a summary" (or "Start fresh (nothing left to summarize)" when there is nothing to summarize, ui/src/resume_failed.rs:56-57), and a "Restart report {time}" link (ui/src/resume_failed.rs:154). The link opens Hosts; there is no report view (flows/cases/README.md item 29).
- **Offered:** Try again, Start fresh with a summary, Restart report link.
- **Action:** Open the session.
- **Result:** No error line is set on the session (`state.session.error` is None), and the card stays until resumed or started fresh; the read mark is sent (D392).
- **Evidence:** design/46-session-couldnt-resume.png (PNG only). The card's text is in ui/src/resume_failed.rs.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=opening_a_session_that_couldnt_resume_shows_no_error`

### 4. Try again resumes the same conversation
- **Screen:** After the click the card goes and the session runs. The terminal shows the resumed agent (`agent up --resume`).
- **Offered:** Session actions as normal.
- **Action:** Restore the lost conversation file (the test writes it back), then click Try again.
- **Result:** `resume_failed` clears, the status leaves Exited, the same conversation file holds "lost-line".
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=after_a_reboot_the_app_says_what_came_back`

### 5. Start fresh with a summary
- **Screen:** The same card. Clicking the button starts a new conversation.
- **Offered:** Start fresh with a summary (only when the terminal kept saved output; otherwise "Start fresh (nothing left to summarize)").
- **Action:** Click Start fresh with a summary, with the old conversations deleted.
- **Result:** The session starts; the new conversation hears what the old terminal printed ("got: remember-the-blue-door").
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=start_fresh_on_the_couldnt_resume_card`

### 6. An agent that reports idle after the restart
- **Screen:** Home lists the resumed agent under "Idle after the restart · 1", and the lost session stays under "Couldn't resume after the restart".
- **Offered:** Click a row.
- **Action:** Restart with turn hooks on (World::start_with_turn_hooks), so the resumed agent reports its turn end.
- **Result:** The idle group holds only the resumed agent; the lost one stays in its own group.
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=after_a_reboot_an_agent_that_reports_idle`

### 7. humata doctor restarts (after a bad restart)
- **Screen:** Terminal output. No screen.
- **Offered:** `humata doctor restarts [REPORT] [--explain] [--allow ID]... [--diagnosis FILE|-] [--root DIR]` (args.rs:976-1000). REPORT is a folder or its name under restarts/; default is the newest.
- **Action:** `humata doctor restarts --explain` with the daemon down.
- **Result:** Reads the report with the CLI's own types, changes nothing, and lists each repair with its id, whether it is safe, and what it does (cli/src/commands/doctor.rs:400-450). Repairs: `resume:<session>` (safe: resume with the saved conversation), `start-fresh:<session>` (not safe: the old conversation stays on disk and the agent no longer continues it), `restart-tab:<tab>` (safe: shell tab again in its folder with the saved output), `store` (safe: check the store; a damaged store makes humatad restore the last good copy at its next start). Lossy repairs are listed as offered and run only with `--allow <id>`.
- **Evidence:** PENDING RUN (a throwaway report folder is not created by the daemon with the lost-conversation setup in step 4; use the test's temp state from cli/tests/doctor.rs)
- **Replay check:** `bazel test --jobs=4 //apps/humata2/cli:doctor_test`

### 8. humata doctor restarts without --allow for a lossy repair
- **Screen:** Terminal output.
- **Offered:** Rerun with `--allow start-fresh:<session>`.
- **Action:** Run without the allow.
- **Result:** The safe repairs run; the lossy one waits and is named as waiting. Allowing an id the report does not offer is a usage error and runs nothing (cli/tests/doctor.rs `allowing_a_repair_the_report_doesnt_offer_is_a_usage_error_and_runs_nothing`). diagnosis.md is written in the report folder (`explain_writes_diagnosis_md_in_the_report_folder_it_read`).
- **Evidence:** cli/tests (no screen)
- **Replay check:** `bazel test --jobs=4 //apps/humata2/cli:doctor_test`

### 9. A dropped link: the tab says so
- **Screen:** The session's tab shows "Lost the connection to devbox-1{ at time}. The agent keeps running there; this view catches up when it is back." (ui/src/session_tabs.rs:780). The pane's banner starts with the same words.
- **Offered:** Keys are not sent. The tab has no reconnect button; Reconnect now is on the machine page (ui/src/machine_page.rs:440, 558).
- **Action:** Cut the link (`cut_link`) while a job prints and a half-typed line exists.
- **Result:** The tab says the link is down; the job's output is not yet visible.
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=a_dropped_link_refuses_typing`

### 10. Typing while the link is down
- **Screen:** One toast containing "not sent" for the refused keys (one toast, however many keys).
- **Offered:** Typing is refused; there is no queue for the keys.
- **Action:** Type "xqlost" while down.
- **Result:** One toast; the keys never arrive later (the test asserts "XQLOST" never shows).
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=a_dropped_link_refuses_typing`

### 11. The link comes back by itself
- **Screen:** The Lost line goes; output printed meanwhile shows.
- **Offered:** Typing works again.
- **Action:** Restore the link (`restore_link`), no action in the app.
- **Result:** The same shell takes typing, its half-typed line is intact, and refused keys are not sent late.
- **Evidence:** PENDING RUN
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=a_dropped_link_refuses_typing`

### 12. An agent's terminal while away (host left out a write)
- **Screen:** A line in the agent terminal ("away-left-out") says what the host left out. The shell tab says nothing.
- **Offered:** Per the three frames: before (no line), refused, and ask.
- **Action:** Set AwayLeftOut on the session (session.rs:218).
- **Result:** The line appears only for the agent terminal, and not for a shell tab (session.rs:189).
- **Evidence:** desktop/tests/__snapshots__/native/away-left-out-before.view.txt, away-left-out-refused.view.txt, away-left-out-ask.view.txt (each has a PNG beside it)
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=the_away_left_out_line_frames`

## Not reachable headless

- The reboot is a daemon restart in the test harness (World::reboot), not a Mac reboot. A Mac reboot's fresh-tab token (gap) is not covered by a test.
- The toast's visible time is 4 s. The harness does not check the duration of the restart toast.
- Real SSH link drop to h2-terminal-c1 is in desktop/tests/live_flows_ssh_test (not this record).
- Restart report link (Restart report HH:MM) opens Hosts, not a report view. Recorded in flows/cases/README.md item 29; not a screen the harness reaches.

## Surprises

- The restart toast says "Mid-turn agents are idle." whenever any session came back (features/app/src/restart.rs:83-86). The daemon does not record a mid-turn state (flows/cases/README.md item 29 and the gap list); the Home row says only "Came back idle after its host restarted. If it was working, tell it to continue." The toast claims something the report does not hold.
- The restart toast uses the default 4 s toast. The Undo toast on archive uses 10 s. The Home groups hold until the session is resumed, so the toast is the only short-lived line.
- The "Couldn't resume" card names "Restart report {time}" as a link, but it opens Hosts, not the report. A person looking for the report finds the machine's page.
- `humata doctor restarts` repairs a bad restart from the CLI. Its `start-fresh:<session>` repair is lossy and needs `--allow`, while the session card's "Start fresh with a summary" button runs without a flag. The two surfaces use different words for the same choice; whether the card says the old conversation stays on disk is not checked in phase 1.
- The restart toast is shown for 4 s. After that, the record of the reboot is the Home groups (until each session is resumed or started fresh) and the "Restart report" link. Whether the Home groups still show a session after it is resumed is not checked here.
- The link drop refuses keys with a toast and the tab has no reconnect button. Reconnect now is on the machine page; a person on the session tab has to go there to force a reconnect (the link also comes back by itself, per the live test).
