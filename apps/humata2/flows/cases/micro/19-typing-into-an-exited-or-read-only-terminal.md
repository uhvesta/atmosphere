# Typing into a terminal that cannot take input is refused, and I can get back to a writable one

Micro. macOS and Linux for the daemon; the app's terminal state is shown on macOS. Web: the shipped app has no browser viewer, so nothing here.

## Starting state

- Humata is running on this Mac, with the desktop app open.
- Four terminals, one for each case:
  - **T1, exited shell.** Session "Fix the ledger", tab 1 (a shell). I typed `exit 3` into it, so the shell has exited with code 3. Its last output is still on screen.
  - **T2, moving session.** Session "Cache warmup", tab 0 (the agent). I started a move from this Mac to box-1. The move is held at its copy step, so it is still running.
  - **T3, read-only attach.** Session "Fix the ledger", tab 1, attached from a terminal on this Mac with `humata attach "Fix the ledger" --tab 1 --read-only`. The desktop app has its own writable attach to the same tab.
  - **T4, stopped sessions.** Session "Docs", tab 0 (the agent). I stopped the sessions on this Mac from the machine page menu (Stop sessions), so the agent and its shells have ended and stay stopped.
- Focus is in the terminal I am typing into. Nothing is in a paste or a find box.

## Steps

1. **I do (T1):** Click into the exited shell tab and type `hello`, then Return.
   - **I see:** The output stays as it was. No line says the shell exited, and `hello` does not show up.
   - **State and events:** The shell's holder reports Exited (code 3). The daemon refuses typed input to an exited shell. Through the app, the keys never reach the link: the replica stops input once the tab has exited (`terminal/client/src/replica.rs:537`), and `write_accepted` returns `false` with no error (`:306`). So the keys are dropped and nothing is queued.
   - **Status:** Fakes only. The keys are not sent, and the app says why once: "This tab's shell has exited, so what you typed was not sent. Open a new terminal to run more commands." The daemon refuses (`daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused`); the app's toast is `desktop/tests/native/terminal.rs::typing_into_an_exited_shell_says_why_once`, told once per stop (`terminal/pane/tests/pane.rs::dropped_input_is_told_once_each_time_input_stops`).

   - **Getting back (T1):** Open a new shell from the session's tab bar. For tab 0 of an exited agent, the exit line has **Resume** (`ui/src/session_tabs.rs:632-645`). Resume starts the agent on its conversation; a new shell starts a fresh shell in the folder. The exited shell is not restarted.

2. **I do (T2):** Click into the agent tab of "Cache warmup" and type `hello`, then Return, while the move is held.
   - **I see:** The output stays. The typed text does not show. The session list shows the session as Idle, not as Moving, because the app maps the daemon's Moving status to Idle (see the gaps). The app does not show "Moving to box-1" either.
   - **State and events:** The move marks the session as moving (`daemon/humatad/src/session_service/move_session.rs:128`). The tab's attach reports `Writable::Moving` (`daemon/humatad/src/attach.rs:483-487`). The app link sends `Control` with controller false (`clients/terminal_link/src/lib.rs:280`), so the view goes read-only and its keys are dropped. The daemon also ignores input on a moving tab at the attach, with no reply (`daemon/humatad/src/attach.rs:323-327`). Nothing is kept for later.
   - **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` shows the `Moving` state on the attach, types a line while moving and checks it never reaches the agent's conversation, then checks the agent takes input after the move. The rig uses fake boxes.

   - **Getting back (T2):** Wait for the move to end. The agent resumes on box-1 and the tab becomes writable there. `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` checks the move ends with the session running on the new machine. Typing on the new machine is not checked by that test.

3. **I do (T3):** In the read-only attach, type `ls`, then Return.
   - **I see:** The output of the shell shows as it is written. My typing does not go anywhere. The CLI skips sending typed input when `--read-only` is set (`cli/src/commands/streams.rs:163`).
   - **State and events:** The read-only attach watches and does not count as a writer (SE-25). If a read-only attach sends input to the daemon, the daemon refuses it with `input` / `NotAllowedHere`, and the message says to attach again without `read_only`.
   - **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type` checks the state is `ReadOnlyAttach`, the other writer count is 1, and the typed input is refused with `NotAllowedHere`.

   - **Getting back (T3):** Detach, then attach again without `--read-only`. The shipped path has no take-control (see B4).

4. **I do (T4):** Click into the agent tab of "Docs" and type `keep going`, then Return.
   - **I see:** The header says Stopped, the view keeps its last output, and the link has ended. The view says it could not open the terminal, or that the connection ended, with "Open the tab again" (the exact words depend on whether a screen had shown).
   - **State and events:** The daemon refuses to attach or type into a stopped tab with `FailedPrecondition`, and it starts nothing. An attach that was open when the stop came is ended by the daemon with `Unavailable`.
   - **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` covers the refusal of attach, type and start. `::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again` covers the end of an open attach. The app's own message is Code, no test.

   - **Getting back (T4):** Machine page menu, **Start sessions**, then confirm. Sessions come back as after a restart, with a report (`ui/src/machine_page.rs:1279-1299`). The confirm step is tested only with fakes (`desktop/tests/machines_flow.rs::start_sessions_asks_first_and_starts_only_when_confirmed`). The daemon side is `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`.

## Branches

### B1. The move finishes while I am typing

- **When:** I type `ls` on T2 as the move's last step runs.
- **I do:** Type `ls` and Return during the last step.
- **I see:** Keys typed while the move is running do not show. Keys typed after the tab is writable again show once the agent takes them.
- **State and events:** The move ends and the attach sends the state again (`daemon/humatad/src/remote_terminal.rs:553-557` for a session on another machine; `daemon/humatad/src/attach.rs:188` for one here). The view's controller flag goes back to true on the next `Control` frame.
- **Expected outcome:** Keys typed in the gap are either refused with a message or sent after the tab is writable. They are never held and sent later, out of order.
- **Status:** Code, no test. The move test checks that keys typed during the move never reach the agent, and that typing after it works. Nothing types across the change.

### B2. The move fails and the session stays on the source

- **When:** The copy to box-1 fails at its copy step.
- **I do:** Start the move from this Mac to box-1 and wait for it to fail.
- **I see:** The move reports failed, with the step that failed named. The session is still on This Mac, and its agent is the same run.
- **State and events:** The move is undone. The source keeps its holder and its conversation. Nothing is left on box-1.
- **Expected outcome:** The session stays where it was, with the same holder pid, and the failure is named.
- **Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step`.

### B3. After a failed move, the terminal is writable again

- **When:** Same as B2.
- **I do:** Type a line into the agent tab after the failure.
- **I see:** The line shows in the agent's output (expected).
- **State and events:** The move's guard clears the moving flag when the move ends (`daemon/humatad/src/session_service/move_session.rs:128-135`). The attach then sends the new state, and the view's controller flag goes back on.
- **Expected outcome:** The terminal is writable and the typed line reaches the agent.
- **Status:** Code, no test. B2's test does not type into the tab after the failure.

### B4. I attach read-only, then ask for write

- **When:** I am in T3, a read-only attach, and I want to type.
- **I do:** Try to type. Then look for a way to ask for write.
- **I see:** The typing is refused. There is no "take control" in the desktop app or the CLI. The only way is to attach again without `--read-only`.
- **State and events:** The lab viewer has a Take control button (`terminal/client/src/lib.rs:665-676`), and the lab host grants control to one viewer at a time. The shipped daemon link ignores `TakeControl` (`clients/terminal_link/src/lib.rs:103`). The daemon's refusal says to attach again without `read_only` (`daemon/humatad/src/attach.rs:315-321`).
- **Expected outcome:** Refused with a clear message, and typing works after a new attach without `--read-only`.
- **Status:** Not built for asking in place. The refusal and a new writable attach are Works (real daemon): `daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type`, and `writers_hear_about_each_other` attaches a second writable client.

### B5. The sessions stop while a terminal is open and typed into

- **When:** I type into T4 while its sessions are being stopped.
- **I do:** Type a line as the stop is taken.
- **I see:** The terminal's connection ends. The view says the connection ended, and the typed line is not sent.
- **State and events:** The daemon ends the attach itself as it leaves, within about 2.5 seconds, rather than waiting for a timeout.
- **Expected outcome:** The attach ends quickly, the line is not sent, and the view says to open the tab again.
- **Status:** Works (real daemon) for the daemon's end of the attach: `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again`. The app's message after the end is Code, no test.

## Expected outcomes

- [ ] Typing into an exited shell is refused by the daemon and is not queued.
- [ ] The app says a shell tab has exited, with its code. (Not true today; see the gaps.)
- [ ] While a session moves, typed input never reaches its agent, and is not sent later.
- [ ] A moving session's status says it is moving to its new machine. (Not true today; see the gaps.)
- [ ] A read-only attach can watch and cannot type; the refusal says to attach again without `--read-only`.
- [ ] While sessions are stopped, attach, type and start are refused and nothing starts.
- [ ] Start sessions, once confirmed, brings the terminals back and they accept typing.
- [ ] A failed move leaves the session on its source, writable, with the same agent.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Typing into an exited shell (T1) | Fakes only | `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused`, `desktop/tests/native/terminal.rs::typing_into_an_exited_shell_says_why_once` |
| 2. Typing into a moving session (T2) | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` |
| 3. Typing into a read-only attach (T3) | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type` |
| 4. Typing into a stopped session (T4) | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started` |
| Getting back: Start sessions confirm | Fakes only | `desktop/tests/machines_flow.rs::start_sessions_asks_first_and_starts_only_when_confirmed` |
| B1. Move ends while I am typing | Code, no test | `daemon/humatad/src/attach.rs:188` |
| B2. Move fails, stays on source | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` |
| B3. Writable again after a failed move | Code, no test | `daemon/humatad/src/session_service/move_session.rs:128-135` |
| B4. Ask for write from a read-only attach | Not built (in place); refusal Works (real daemon) | `clients/terminal_link/src/lib.rs:103` |
| B5. Sessions stop while typing | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again` |

Tag count, steps: Works (real daemon) 3 (steps 2, 3, 4), Likely bug 1 (step 1). The start-sessions confirm is Fakes only.
Tag count, branches: Works (real daemon) 2 (B2, B5), Code, no test 2 (B1, B3), Not built 1 (B4, the ask), with its refusal Works (real daemon).

## Gaps and likely bugs

- Likely bug: the app drops the daemon's Exited notice. `clients/terminal_link/src/lib.rs:191` returns nothing for `ServerMessage::Notice`, and `ui/src/session_tabs.rs:632` draws the exit line only for tab 0. So an exited shell tab shows only its last output. To confirm: exit a shell tab in the app and look for a line.
- Likely bug: the app keeps a moving session as Idle. `clients/live/src/map.rs:384` maps `SessionStatus::Moving` to `Status::Idle`, and a unit test asserts that mapping. A session that is moving shows no moving status in the list. To confirm: start a move from another client, then look at the session in the app.
- Likely bug (to confirm): the Type RPC has no moving check. `daemon/humatad/src/terminal.rs:877-915` (`type_in`) goes straight to `type_into` (`:659`) without the `is_moving` check that the attach path has (`daemon/humatad/src/attach.rs:323`). A typed line sent by RPC during a move may reach the holder on the source. To confirm: hold a move at its copy step and call the Type RPC.
- Fixed: keys dropped on a read-only, exited or moving tab raise one toast that says why (`desktop/tests/native/terminal.rs::typing_into_an_exited_shell_says_why_once`, `ui/src/terminals.rs::tests::typing_during_a_move_names_where_the_session_is_going`, Fakes only).
- Not built: the MV-4 line "Moving to <machine>…" for a moving session's tab (SPEC MV-4). The string is not in `ui/src`. The only move progress is the move sheet, which shows for a move this app started (`ui/src/overlays.rs:255-311`).
- Not built: take control from a read-only attach in the shipped path (`clients/terminal_link/src/lib.rs:103` ignores `TakeControl`). It exists only in the lab host and viewer (`terminal/host/src/lib.rs`, `terminal/client/src/lib.rs:665-676`).
- Not built in the shipped path: the machine-offline state (SPEC SE-16). `WRITABLE_MACHINE_OFFLINE` is produced only for a remote machine's relay (`daemon/humatad/src/remote_terminal.rs:591`). This case uses a stopped session, not an offline machine.
- Code, no test: the app's own text for these four states (`ui/src/terminals.rs`). No headless test checks the tab's state line for an exited, moving, read-only or stopped terminal.
