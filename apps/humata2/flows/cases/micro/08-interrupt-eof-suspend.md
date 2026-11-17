# ⌃C stops a running program, ⌃D on an empty line ends a shell, and ⌃Z suspends a job that fg resumes

Layer: micro. Platforms: macOS, Linux, web. The key rules are the same everywhere, except that ⌘C copies on macOS and Ctrl+Shift+C copies on Linux.

## Starting state

- Humata 2 is open on this Mac. The session "Fix the ledger" has an agent tab (tab 0) and two shell tabs (tabs 1 and 2). Tab 1 is focused, at a zsh prompt.
- Nothing is selected, and I hold no unsent text.
- The session is live, and its holder owns each shell's pseudo-terminal.

## Steps

1. **I do:** Type `yes` and press Enter. Then press ⌃C.
   **I see:** The `y` lines stop, and a prompt returns.
   **State and events:** The app sends byte `0x03` to the shell. The pseudo-terminal's line discipline turns it into an interrupt for the foreground job.
   **Status:** Fakes only for the byte. `terminal/view/src/keys.rs:289` (`ctrl_c_goes_to_the_program_on_every_platform`) checks ⌃C is sent to the program on both platform rules. `terminal/vt/tests/input.rs::ctrl_c_sends_etx` checks the engine sends `0x03`. `terminal/pane/tests/pane.rs::typed_keys_reach_the_program_in_its_encoding` checks the same through a fake program stream. None runs `yes` in a real shell.

2. **I do:** Let `yes` run for a few seconds, then press ⌃C.
   **I see:** Output stops at once, even though `yes` writes as fast as it can.
   **State and events:** The interrupt reaches the foreground job, and the view stops drawing more output. No output is queued behind the flood that the user can see.
   **Status:** Code, no test. No test floods a real shell and interrupts it.

3. **I do:** On macOS, drag to select some output, press ⌘C, then press ⌃C while `yes` runs.
   **I see:** ⌘C copies the selection and sends nothing. ⌃C stops `yes`.
   **State and events:** ⌘C goes to the copy path only. ⌃C is not a copy shortcut on macOS.
   **Status:** Fakes only: `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` checks ⌘C copies and sends nothing. The ⌃C part is covered by step 1's keys test.

4. **I do:** At an empty prompt, press ⌃D.
   **I see:** The shell exits. The agent tab is unchanged. For tab 1, the view shows the last screen with a banner that says "Lost the connection to this program's machine. The program keeps running there … typing is off until then." No "exited with code" note appears for a shell tab.
   **State and events:** The shell exits with status 0. The holder records the exit and reports it to the daemon. The daemon marks the terminal `Exited`, and the app's replica stops accepting input.
   **Status:** Works (real daemon) for the exit: `daemon/revive/tests/kill_matrix.rs::a_live_agent_is_reattached_and_one_that_ended_is_saved_as_exited` sends byte `0x04` to a real `sh` through the holder, and checks it exits with code 0. It is a holder test, not a `humatad2` test. The wrong text the user sees is a Likely bug (see below).

5. **I do:** Type `echo hi` and press Enter in the exited tab 1.
   **I see:** Nothing happens. The typed text does not reach the shell.
   **State and events:** The daemon refuses the input because the tab's shell has exited. The app's replica also drops it, because input is off once the shell has exited.
   **Status:** Works (real daemon): `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused` checks the daemon refuses the input. The app does not tell the user why nothing happened, see the Likely bug below.

6. **I do:** Start `cat` in tab 2, then press ⌃D on an empty line.
   **I see:** `cat` exits, and the prompt returns.
   **State and events:** The same byte and the same exit path as step 4, for a program instead of a shell.
   **Status:** Code, no test. Covered only by the holder's byte test in step 4.

7. **I do:** In tab 2 (a live shell), type `echo x`, then press ⌃D.
   **I see:** zsh deletes the character under the cursor, or lists completions. The shell does not exit.
   **State and events:** The app sends `0x04` unchanged. The shell decides what it means.
   **Status:** Code, no test. Nothing tests ⌃D on a non-empty line.

8. **I do:** Run `sleep 300`, press ⌃Z, then type `fg` and press Enter.
   **I see:** `sleep` stops with a message such as `suspended`, then runs again after `fg`.
   **State and events:** The app sends `0x1A` (⌃Z). The holder makes the pseudo-terminal the controlling terminal of the program it starts (`daemon/holder/src/server.rs:511-516`), so job control works.
   **Status:** Code, no test. The holder code is in place, but no test runs job control.

9. **I do:** Press ⌃\ in a running program that waits for input (for example `cat`).
   **I see:** The program quits. The shell reports that it was killed by the quit signal.
   **State and events:** The app sends `0x1C` to the program. On macOS, ⌘\ splits the pane instead (see the next note).
   **Status:** Fakes only: `terminal/pane/tests/pane.rs::the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program` checks ⌃\ sends `0x1C` and ⌘\ (shortcut) asks for a split. It uses a fake program stream.

10. **I do:** Press ⌃S, then ⌃Q, in a running `yes`.
    **I see:** Output freezes after ⌃S and resumes after ⌃Q, if the terminal has flow control on.
    **State and events:** The app sends `0x13` and `0x11`. Flow control is done by the pseudo-terminal, not by the app.
    **Status:** Code, no test. Nothing tests these bytes.

## Branches

### ⌃C with text selected on Linux

- **When:** Text is selected in a Linux shell tab, and `yes` runs.
- **I do:** Press Ctrl+C.
- **I see:** `yes` stops. The selection is not copied.
- **Expected outcome:** Ctrl+C interrupts the program, and only Ctrl+Shift+C copies.
- **Status:** Fakes only: `terminal/view/src/keys.rs:289` checks Ctrl+C goes to the program on the Linux rule. The key mapping does not read the selection, so the copy-or-interrupt choice is fixed by the key alone. `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` checks Ctrl+Shift+C copies on Linux.

### ⌃C in the agent terminal mid-turn

- **When:** An agent is partway through a turn.
- **I do:** Press ⌃C in the agent tab.
- **I see:** The agent stops or cancels its turn, if it honours the interrupt.
- **Expected outcome:** ⌃C reaches the agent's program. Humata does not add its own stop control here.
- **Status:** Code, no test. The key goes to the agent's pseudo-terminal through the same path as step 1.

### Stop on a shell that ignores hangup and terminate

- **When:** A shell has run `trap '' HUP TERM`, so it ignores the usual signals, and I press Stop on the session.
- **I do:** Press Stop on the session.
- **I see:** The session stops. Its shells and agents are gone, and no restart report is written for a plain stop.
- **Expected outcome:** The daemon kills the shell that ignores hangup and terminate.
- **Status:** Works (real daemon): `daemon/humatad/tests/stop_sessions.rs::a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop`.

### ⌃D in the last shell tab of a session

- **When:** The only remaining shell tab is ended by ⌃D. The agent tab is still there.
- **I do:** Press ⌃D on an empty line in the last shell tab.
- **I see:** The shell tab shows the same banner as step 4. The tab stays open.
- **Expected outcome:** The shell tab shows that the shell has ended, and the session stays open. The shell tab is not closed by itself.
- **Status:** Code, no test. I found no code that closes an exited shell tab on its own. The exit state is covered by step 4's holder test.

## Expected outcomes

- [ ] ⌃C sends `0x03` to the foreground program on macOS and Linux.
- [ ] ⌃C stops a running flood, such as `yes`, promptly.
- [ ] ⌘C copies on macOS and sends nothing. Ctrl+Shift+C copies on Linux.
- [ ] ⌃D on an empty line ends the shell, and the tab says so.
- [ ] Typing into an ended shell tab is refused, and the user is told why.
- [ ] ⌃D on a non-empty line does not end the shell.
- [ ] ⌃Z suspends a job, and `fg` resumes it.
- [ ] ⌃\ sends `0x1C` to the program, and ⌘\ splits the pane instead.
- [ ] ⌃S and ⌃Q freeze and resume output when flow control is on.
- [ ] Stop ends a shell that ignores hangup and terminate.

## Status summary

Paths are relative to `apps/humata2`.

| Step or branch | Status | Evidence |
|---|---|---|
| 1. ⌃C sends `0x03` | Fakes only | `terminal/view/src/keys.rs:289`; `terminal/vt/tests/input.rs::ctrl_c_sends_etx` |
| 2. ⌃C stops a flood | Code, no test | None |
| 3. ⌘C copies, ⌃C interrupts (macOS) | Fakes only | `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` |
| 4. ⌃D on an empty line exits | Works (real daemon) for the exit; Likely bug for the text shown | `daemon/revive/tests/kill_matrix.rs::a_live_agent_is_reattached_and_one_that_ended_is_saved_as_exited` |
| 5. Typing after exit | Works (real daemon) for refusal; Likely bug for the message | `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused` |
| 6. ⌃D on `cat` | Code, no test | Holder test only (step 4) |
| 7. ⌃D on a non-empty line | Code, no test | None |
| 8. ⌃Z and `fg` | Code, no test | `daemon/holder/src/server.rs:511-516` |
| 9. ⌃\ quits a program | Fakes only | `terminal/pane/tests/pane.rs::the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program` |
| 10. ⌃S and ⌃Q | Code, no test | None |
| Branch: ⌃C with selection on Linux | Fakes only | `terminal/view/src/keys.rs:289`; `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` |
| Branch: ⌃C in the agent mid-turn | Code, no test | None |
| Branch: Stop on a hangup-ignoring shell | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop` |
| Branch: ⌃D in the last shell tab | Code, no test | `ui/src/session_tabs.rs:632` (exit note for tab 0 only) |

## Gaps and likely bugs

- Likely bug: an ended shell tab shows the wrong banner. The replica marks an exited terminal as not authorized (`terminal/client/src/replica.rs:530-536`). The session view then turns the pane's disconnected banner on for any session that is not offline (`ui/src/session_tabs.rs:705-709`). The banner text is at `terminal/pane/src/pane.rs:903`, and it says the program keeps running. Confirm: ⌃D in a shell tab, then read the tab.
- Likely bug: the exit note is only shown for the agent tab. `ui/src/session_tabs.rs:632` matches `(0, Status::Exited)`, so a shell tab gets no "exited with code" note.
- Likely bug: the daemon's exit notice text is dropped on the app side. `daemon/humatad/src/attach.rs:290` sends a `Notice` of kind `Exited` with the text "exited with code N", and `clients/terminal_link/src/lib.rs:191` drops every `Notice` frame. Confirm: ⌃D in a shell tab, then check whether any "exited" text appears. Together with the two items above, the user sees no clear message.
- Not built: no milestone is named for a visible "shell ended" note. The status and exit code reach the replica (`terminal/client/src/replica.rs:1175-1178`), so the gap is in rendering.
- Coverage gap: none of ⌃C flood stop, ⌃D on a non-empty line, ⌃Z and `fg`, ⌃\ in a real shell, or ⌃S and ⌃Q has a live test.
