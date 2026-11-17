# ↑ and ↓ recall the shell's commands, and the history survives an app restart

Layer: micro. Platforms: macOS, Linux, web. Only macOS has a live test for this flow, and it is a manual target.

## Starting state

- Humata 2 is open on this Mac, with the session "Fix the ledger" selected in the sidebar.
- The session has an agent tab (tab 0) and a shell tab (tab 1). The shell tab is focused and shows a zsh prompt.
- The shell's history file already holds `ls`, `git status` and `echo VERIFIED` from earlier use.
- The machine's holder process owns the shell's pseudo-terminal, so the shell keeps running when the app is closed.
- The shell line is empty. I hold no unsent text.

## Steps

1. **I do:** At the prompt, type `printf 'HIST_%s\n' ONE` and press Enter.
   **I see:** `HIST_ONE` on its own line, then a new prompt.
   **State and events:** The shell runs the line. The line goes into the shell's output log and into zsh's history.
   **Status:** Works (real shell): `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` types commands into a real zsh. It is a manual target that needs the owner's environment variables.

2. **I do:** Press ↑.
   **I see:** The last command, `printf 'HIST_%s\n' ONE`, appears on the prompt line with the cursor at its end.
   **State and events:** The key is sent as `ESC [ A` when the shell has not asked for application cursor keys, and as `ESC O A` when it has. zsh normally asks, so the shell gets `ESC O A`. The bytes follow the mode the shell set, not a fixed value.
   **Status:** Works (real shell): the same lab test presses ↑ and waits for `VERIFIED` on the cursor line. The choice of bytes by mode is covered by `terminal/vt/tests/input.rs::arrows_follow_the_cursor_key_mode`, which runs the engine without a shell, so it is tagged Fakes only below.

3. **I do:** Press ↓.
   **I see:** The line goes back to the draft I had before pressing ↑, and the cursor returns to its old column.
   **State and events:** The shell moves its history position back. The terminal sends `ESC [ B` or `ESC O B`, matching the mode.
   **Status:** Works (real shell): the same lab test checks that the cursor column after ↓ equals the column before ↑.

4. **I do:** Press ⌃R, then type `HIST`.
   **I see:** The prompt changes to `(reverse-i-search)` and shows the matching command, `printf 'HIST_%s\n' ONE`.
   **State and events:** The app sends byte `0x12` to the shell. The app has no binding for ⌃R (no Ctrl bindings in `ui/src` or `desktop/src`), so zsh does the search.
   **Status:** Code, no test. The key goes through the generic Ctrl path in `terminal/view/src/keys.rs:81-113`. `terminal/vt/tests/input.rs::ctrl_c_sends_etx` covers ⌃C only. No test sends ⌃R or runs zsh's search.

5. **I do:** Press ⌃P, then ⌃N.
   **I see:** The same result as ↑ then ↓.
   **State and events:** The app sends bytes `0x10` and `0x0E`.
   **Status:** Code, no test. Same generic path as step 4. No test covers these keys.

6. **I do:** Press ⌘Q to quit the app. Open the app again, select the same session and the same shell tab.
   **I see:** The shell tab reconnects and shows the screen it had before I quit, with the same prompt. Typing is accepted once the screen has arrived.
   **State and events:** The holder keeps the shell running while no client is attached. The new client reattaches to the same holder and catches up from the screen. The shell process is the same one.
   **Status:** Works (real daemon): `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` runs a real `humatad2`. A background job in the shell sets a variable before the restart and prints it after, so the test shows the same shell kept running and takes input once synced.

7. **I do:** At the prompt, press ↑.
   **I see:** The last command from before the quit appears on the prompt line.
   **State and events:** Recall works because the same zsh process still holds its in-memory history. Step 6 shows the process survives.
   **Status:** Code, no test. No test presses ↑ after a reattach.

## Branches

### ↑ inside a full-screen program

- **When:** I open Neovim in the shell tab. It turns on application cursor keys.
- **I do:** Press ↑ and ↓ in the editor.
- **I see:** The cursor moves up and down one line at a time.
- **Expected outcome:** The program gets the application-mode arrows, so it moves the way it should.
- **Status:** Works (real shell): the lab test in step 1 moves the Neovim cursor from line 1 to 2 and back, using real ↓ and ↑ presses, before it starts zsh. The mode switch itself is covered by `terminal/vt/tests/input.rs::arrows_follow_the_cursor_key_mode` (Fakes only).

### A command prints 12,000 lines

- **When:** A command prints 12,000 lines of output.
- **I do:** Scroll back with ⌘PageUp on macOS, or ⇧PageUp on Linux, or drag the scrollbar.
- **I see:** Lines `HISTORY_00001` to `HISTORY_12000` are reachable. The scrollbar total is at least 12,000.
- **Expected outcome:** The view keeps 12,000 lines of output scrollback. This is output history, not command history. Recalling 12,000 typed commands with ↑ is not tested.
- **Status:** Works (real shell): the lab test in step 1 prints the lines with `awk` and checks the scrollbar total is at least 12,000. It is a manual target.

### ↑ while the session is moving

- **When:** I start moving the session to another machine.
- **I do:** Press ↑ in the agent tab, or type a line.
- **I see:** The tab shows that the session is moving. Typed text does not reach the agent.
- **Expected outcome:** No input reaches the program during the move. The arrow keys take the same path as typed text, but the test only checks typed text.
- **Status:** Works (real daemon): `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` types a line during the move and checks it never reaches the agent's conversation. It does not press ↑.

### The host restarts and the shell is new

- **When:** The host restarts or loses power. The holder and its shell end, and the app starts again.
- **I do:** Open the app and the same shell tab.
- **I see:** The tab comes back with a new shell that takes input. Output from the old shell is kept on disk. ↑ recalls only what zsh saved to its history file, not what was in memory.
- **Expected outcome:** A new shell takes input. Output the holder saved is still there. Recall depends on zsh's own history file.
- **Status:** Three parts, each tagged on its own:
  - The tab comes back and takes input: Works (real daemon), `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back`.
  - Output is kept on disk when the holder stops: Works (real daemon), `daemon/holder/tests/terminal.rs::a_holder_stopped_by_a_signal_keeps_its_output_saved`. This is a holder test, not a full app test, and it does not check that the tab shows the saved output.
  - ↑ recalls the old commands after the new shell starts: Code, no test.

## Expected outcomes

- [ ] ↑ puts the previous command on the prompt line, and ↓ puts back the unsent draft.
- [ ] ↑ and ↓ send the bytes the shell's cursor-key mode asks for.
- [ ] ⌃R, ⌃P and ⌃N reach the shell. The app does not use them.
- [ ] After ⌘Q and a reopen, the same shell is still running and takes input once it has caught up.
- [ ] ↑ after a reopen recalls the command typed before the quit.
- [ ] Scrollback in a real shell holds at least 12,000 output lines.
- [ ] Typed input does not reach the program while the session is moving.
- [ ] After a host restart, the tab returns with a new shell that takes input.

## Status summary

Paths are relative to `apps/humata2`. Tests that run the engine without a shell (`terminal/vt`) are tagged Fakes only, because they do not meet the real-shell bar.

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Run a command | Works (real shell) | `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` (manual) |
| 2. ↑ recalls the last command | Works (real shell) | Same lab test; byte choice by mode in `terminal/vt/tests/input.rs::arrows_follow_the_cursor_key_mode` (Fakes only) |
| 3. ↓ returns the draft | Works (real shell) | Same lab test |
| 4. ⌃R reverse search | Code, no test | `terminal/view/src/keys.rs:81-113` |
| 5. ⌃P and ⌃N | Code, no test | `terminal/view/src/keys.rs:81-113` |
| 6. ⌘Q, reopen, shell catches up | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 7. ↑ after reopen | Code, no test | None; step 6 shows the process survives |
| Branch: ↑ in a full-screen program | Works (real shell) | Lab test (Neovim cursor 1 to 2 and back) |
| Branch: 12,000 lines of output | Works (real shell) | Same lab test (scrollbar total at least 12,000) |
| Branch: ↑ while moving | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_moving_session_s_terminal_is_read_only_until_it_resumes` (typed text only) |
| Branch: host restart, new shell | Works (real daemon) for the tab; Code, no test for ↑ recall | `daemon/humatad/tests/restart.rs::a_shell_whose_holder_died_comes_back`; `daemon/holder/tests/terminal.rs::a_holder_stopped_by_a_signal_keeps_its_output_saved` |

## Gaps and likely bugs

- Not built: none for this flow. The shell owns its command history, and the app keeps no command list of its own.
- Coverage gap: no test recalls commands after a reattach (step 7).
- Coverage gap: ⌃R, ⌃P and ⌃N are never sent to a live shell in a test.
- Coverage gap: the "12,000 lines" lab test measures output scrollback, not command history. Recall of many typed commands is not covered.
- Likely bug: none found in the code for this flow.
