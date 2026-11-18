# VERIFY lane `report`: bug reports end to end

Agent: copilot 1.0.113, claude-haiku-5.5. Machine: This Mac only (`ma_7cf0ba8f84c441d38debfedb3d4989e7`); no remote.
Evidence root: <scratchpad>/verify/evidence/report/

Where `humata report` lives: it is in the `humata` CLI (cli/src/args.rs:1005, dispatched in cli/src/commands/hosts.rs:824), but the CLI does not write reports itself. `record`, `stop` and `snapshot` are sent to the running Humata 2 desktop app over a local socket (`$HOME/.humata2/app.sock` or `HUMATA_APP_SOCKET`, report/src/remote.rs:84-92). The reports are built and saved by the app (report crate, apps/humata2/report). The app-side entry points are the Help > Report a Bug menu (desktop/src/main.rs:296-299), ⇧⌘R and ⇧⌘S (ui/src/keys.rs:218-219), the recording dot (ui/src/report_view.rs:246-262) and the form.

## Why steps 1 and 2 could not run for real

`humata report record|stop|snapshot` needs the Humata 2 desktop window app running. The brief says headless only, and the app opens a window, so I did not start it. Probe with no app (short socket path): `humata --json report snapshot` returns `app_not_running`, exit 7, with the fix "Open the Humata 2 app" (evidence `probe-snapshot-app-not-running.txt`). So no recording, no snapshot and no Copy-report prompt came from the product. What I did instead is marked below as stand-in.

## Flow 1: a recording replays the same steps, views and client calls, and secrets are scrubbed

Result: BLOCKED (env: the recording needs the desktop app). The Copilot turn part was a PASS.

Steps:
1. `humata report record`: not run. The app was not running (see above).
2. Copilot session on the channel `report-check` (repo verify/demo): `humata --json session start --agent copilot --model claude-haiku-5.5` gave session `se_5c98ec8042ad4ecea20e6cd0e87eb205`, status `probably_idle` after about 1 minute.
3. One tiny turn: "Create a file named hello.txt containing the word hi, then stop."
4. `humata session type` returned `typed: true`, but the text stayed in Copilot's input box and nothing ran (BUG-T1 again, see findings).
5. Pressed Enter through tmux (`say.sh`). The agent asked for a file edit under Manual Approval. I approved it with `humata inbox approve rq_f3a0...` (the product path). `hello.txt` contains `hi`.
6. Read back: `humata --json conversation` returned user turn, `create` tool call, the tool result, and the agent reply "I created hello.txt ... containing "hi"". Evidence: `s1-conversation.json`, `s1-read.json`, `s1-screen-after-turn.txt`.
7. `humata report stop`: not run (no app). `humata report stop` before a record gives `not_recording` in the app code (cli/src/error.rs:367), not checked live.
8. Replay: not run live. Replay is covered headless by desktop/tests/report.rs `a_saved_recording_replays_on_a_fresh_app_to_the_same_state` and report/tests/replay.rs (see the step 3 table).
9. Secret scan (no live recording exists, so this scanned the stand-in folders and the evidence folder): `gho_`, `ghp_`, `COPILOT_GITHUB_TOKEN` and the first 8 characters of the lane's Copilot token (read from the env, not printed) were each found in 0 files, across the stand-in report folders, `evidence/report` and the scratch folder. This does not show the scrub works on a real recording. The only scrubbed marker in the golden recording is `token ‹GITHUB_TOKEN› refreshed` (report/tests/__snapshots__/folder_snapshot/recording.txt).

## Flow 2: a snapshot saves a report that names the right elements, and the Copy-report prompt lets an agent act on it

Result: BLOCKED for the real pipeline (no app). Stand-in check PASS for both kinds the CLI offers (`record` = recording, `snapshot`).

Steps:
1. `humata report snapshot`: not run for real (app not running; probe above).
2. Stand-in folders: I split the two golden report folders that the real writer produced in the report crate's folder_snapshot test into `$LANE_ROOT/reports/`: `2026-11-14-the-draft-kept-milk-after-a/` (snapshot kind: report.md, state.txt, view.txt, layout.txt, daemon.log, events.jsonl) and `2026-11-14-the-list-showed-milk-twice/` (recording kind: same files, with a 12-line events.jsonl and daemon log). Not a report from this run.
3. Copy-report prompt: NOT produced by the app. I could not call `copy_prompt` (report/src/prompt.rs) without building. I wrote a hand-made prompt that follows the same shape the code describes (summary, then the file list), so the check below tests the agent, not the product's prompt text.
4. Snapshot-kind prompt in the Copilot session: asked for the draft text at the start and the file holding the view at the start. Reply: "milk", and `.../reports/2026-11-14-the-draft-kept-milk-after-a/view.txt`. Both correct. The agent read the files with no approval needed. Evidence `s1-snapshot-prompt-reply.txt`.
5. Recording-kind prompt: asked which gesture clicked Add and which daemon method was called. Reply: a click on Button "Add" in Window "Toy" at 200 ms, and `humata.v1.SessionService/List`. Both correct. The agent ran one shell read (`cat events.jsonl && cat daemon.log`) that needed Manual Approval; I approved it through the inbox (`rq_1b663bfc...`). Evidence `s1-recording-prompt-reply.txt`.
6. No `.new.png` renders were written by the report tests, so nothing was copied to evidence for that.

## Flow 3 (app-only parts): headless coverage

Result: PASS for the 9 headless targets I queued (run 57, all passed). Five passed from Bazel's cache (the inputs had not changed since their last pass). The bazel test log is in batch/run57.log. Evidence `batch-run57-report-targets.txt`.

| Target | What it proves (headless) | Result |
|---|---|---|
| //apps/humata2/desktop:report_test | Record via ⇧⌘R key, form, pin comment, Copy report; snapshot via ⇧⌘S; draw a loop round a row, Undo, Done, then the prompt names the row and "circle around"; drawing.json and marks.svg written; replay of a saved recording on a fresh app; a secret typed while recording is in no file; the record/stop socket without opening the form; palette Record/Snapshot; Open last report; RPC calls recorded and replay still matches | PASSED 31.8 s |
| //apps/humata2/features/app:report_test | Reducer: saved recording stops recording and opens the form; saved snapshot leaves recording alone; failure toasts; pin edits comments; match filter ignores case, capped at 50; strokes drawn, undone, cleared, removed; Escape keeps form and strokes; a report reads each machine's version once | PASSED |
| //apps/humata2/features/app:every_action_test | Every key binding runs a command. ToggleRecording runs Record, TakeSnapshot runs Snapshot (so ⇧⌘R and ⇧⌘S are wired to the commands) | PASSED (cached) |
| //apps/humata2/features/palette:report_commands_test | Palette lists "Stop recording" or "Start recording a bug report" by state, offers "Open last report" only once one exists, and hands the picked command to the app | PASSED |
| //apps/humata2/ui:ui_unit_test | record.rs: text with a secret is covered, other text is not; rpc calls outside a recording are dropped; report shortcuts are never recorded; keys.rs: the shortcut sheet is consistent with the bindings | PASSED |
| //apps/humata2/report:prompt_test | copy_prompt output (what the Copy-report prompt contains) | PASSED (cached) |
| //apps/humata2/report:scrub_test | scrub removes the seeded secrets from every file | PASSED (cached) |
| //apps/humata2/report:replay_test | a recording replays to the state, view and layout it ended in | PASSED (cached) |
| //apps/humata2/report:roundtrip_test | save then read back gives the same events | PASSED (cached) |

Not covered by any headless test (the gaps I found):
- Menu items: Help > Report a Bug > "Start or Stop Recording" and "Take Snapshot" are built in desktop/src/main.rs:296-299. No test reads the menu. The actions they call are tested through the key-based tests above.
- Title-bar dot: ui/src/report_view.rs:246-262 (button "report-stop", label "Stop recording", tooltip, red colour, top-right position). No test clicks it. The key path calls the same `toggle` function, so the logic is shared, but the dot's own wiring is not.
- "Clear all marks" (ui/src/report_view.rs:553): the reducer is tested in features/app report_test, but no desktop test clicks the button.
- The form's look (sheet layout, the element filter and pinned list): no golden image under desktop/tests/__snapshots__ covers the report sheet. The behaviour is tested, the look is not.

## Summary

| Flow | Result |
|---|---|
| 1. record, replay, scrub | BLOCKED (env: needs the desktop app). Copilot turn part: PASS |
| 2. snapshot, copy prompt, agent acts on it | BLOCKED (env: needs the desktop app). Stand-in agent check: PASS for both kinds (prompt not app-made) |
| 3. headless coverage of app-only parts | PASS, 9 of 9 targets; 4 gaps listed above |

Flows run: 3. Passed: the Copilot turn read-back, the stand-in agent checks for both kinds, and 9 of 9 headless targets. Not run for real because of the app: the recording, its replay, the snapshot and the Copy-report prompt.

## Findings

1. **BUG-T1 reproduced (known).** `humata --json session type <s> "<text>"` returns `{"typed": true}` but leaves the text unsubmitted in Copilot's input box. Enter through tmux submits it. Steps: start a Copilot session, `session type` a line, read `screen.sh`. Expected: submitted. Actual: input box only.

2. **Misleading message when the socket path is too long (low).** With the lane's `HOME`, `$HOME/.humata2/app.sock` is 157 bytes; the macOS Unix socket limit is 104. Connecting fails with a non-NotFound error, so it skips the `NotRunning` branch (report/src/remote.rs:226-234) and gets `app_unreachable` (cli/src/error.rs:357-359): "Quit and reopen the Humata 2 app" plus "couldn't talk to the app ... restart it". Repro: run with a long `HOME` and no app: `humata --json report record`. Expected: say the socket path is too long and point at `HUMATA_APP_SOCKET` with a short path, or "open the app" when none is running. Actual: "quit and reopen", which does not fit. The owner's default `HOME` path is short, so this only bites a long `HOME`. Evidence `probe-record-app-unreachable.txt`.

3. **`humata --json session queue` says the daemon is too old (unverified).** It returns `terminal.list_queued` `unimplemented`: "update Humata on this machine". The lane's daemon is from `verify/bin`, so this may be a stale build rather than a product bug. Evidence `queue-unimplemented.json`.

4. **Batch runner drops cached passes from the result file.** run.sh's awk matches lines starting with the target and a state, so `//... (cached) PASSED` lines are missing from `runN.result`. Read `runN.log` for those. Run 57 had 5 such lines.

5. **Harness note (not product).** `lane-up.sh` copies the owner's `~/.copilot/config.json` into the lane home (`cp -n`). I did not open the copy. The sync lane already flagged this.

6. **Process note.** Before I sourced the lane env, I ran `ls $HOME/.humata2/` in the owner's home by mistake. I only saw the entry names (app.sock, humatad.sock, bin, logs, state). I opened no file there and did not touch the owner's app or daemon. Reporting it so it is known.
