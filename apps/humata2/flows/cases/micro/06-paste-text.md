# A pasted line reaches the shell as typed, and a multi-line paste waits for my OK

Layer: micro. Platforms: macOS (⌘V), Linux (Ctrl+Shift+V or Shift+Insert), web. The live tests run on macOS and over SSH.

## Starting state

- Humata 2 is open on this Mac. The session "Fix the ledger" has an agent tab (tab 0) and a shell tab (tab 1).
- The shell tab is focused at a zsh prompt on this Mac. zsh has asked for bracketed paste, so pasted text arrives wrapped in `ESC [ 200 ~` and `ESC [ 201 ~`.
- A second shell tab on the same session runs `cat`, a program that has not asked for bracketed paste.
- The clipboard holds `ls -la`.
- No paste is waiting for confirmation, and I hold no unsent text.

## Steps

1. **I do:** Press ⌘V at the zsh prompt (Ctrl+Shift+V on Linux).
   **I see:** `ls -la` on the prompt line. Nothing runs, because there is no Enter.
   **State and events:** The text is sent as one bracketed paste. The shell receives the bytes between the markers and nothing else.
   **Status:** Works (real shell) on macOS. `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` pastes with ⌘V into a real zsh, then presses Enter and waits for the output. It is a manual target. On Linux the same rule is only covered by `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked`, which uses a fake program stream (Fakes only).

2. **I do:** Copy a five-line shell script. Press ⌘V at the zsh prompt.
   **I see:** The five lines sit in the shell's input. Nothing runs until I press Enter.
   **State and events:** The line breaks stay as `\n` inside the bracketed paste. The shell does not treat them as Enter.
   **Status:** Works (real shell) over SSH. `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` checks that a remote reader on a real OpenSSH PTY gets the exact bracketed bytes of a multi-line Unicode paste. The remote reader is a Python script, not zsh, and no test presses Enter afterwards.

3. **I do:** Open Neovim in the shell tab, press `i`, then press ⌘V with the five-line script.
   **I see:** If Neovim asked for bracketed paste, the five lines appear as text. If it did not, the paste prompt appears (see the branch below).
   **State and events:** Neovim's own mode decides the wrapping. The app sends what the mode asks for.
   **Status:** Code, no test. Neovim is in the lab, but no test pastes several lines into it. The Neovim follow-up is deferred (OVERVIEW.md, terminal section).

4. **I do:** Copy this text: `a`, then the end marker `ESC [ 201 ~`, then `rm -rf ~`, then a new line. Press ⌘V at the zsh prompt.
   **I see:** The whole text waits at the prompt. Nothing runs. The embedded end marker does not end the paste.
   **State and events:** The engine removes the embedded end marker before wrapping, so only the real closing marker is sent.
   **Status:** Fakes only: `terminal/vt/tests/paste.rs::pasted_text_cannot_end_a_bracketed_paste_early` checks that only one end marker is in the output. It runs the engine without a shell, so the real-shell bar is not met. The same engine call is used by the app.

5. **I do:** With a shell tab focused, choose Edit > Paste from the menu bar.
   **I see:** The same result as ⌘V.
   **State and events:** The menu sends `input::Paste` (`desktop/src/main.rs:245`). I found no terminal code that handles that action. The terminal only pastes from its key handler (`terminal/view/src/view.rs:870`).
   **Status:** Code, no test. The gpui_kit source that would route the action is not in this repository, so I could not confirm the menu path. See Gaps and likely bugs.

## Branches

### I cancel the multi-line confirmation

- **When:** A program that has not asked for bracketed paste (the `cat` tab) gets five lines.
- **I do:** Copy five lines, press ⌘V in the `cat` tab, then click Cancel.
- **I see:** A bar above the terminal reads: "Paste 5 lines? This program didn't ask for bracketed paste, so each line break runs the line before it as a command." It has Cancel and Paste buttons. After Cancel, the bar closes.
- **Expected outcome:** Nothing is sent, and nothing runs.
- **Status:** Fakes only: `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user` checks that the prompt shows, Cancel closes it, and nothing is sent. It uses a fake program stream.

### Paste while the link is down

- **When:** The link to the machine is down and the tab shows disconnected.
- **I do:** Press ⌘V in the tab.
- **I see:** The "Lost the connection" banner (see the Likely bug below). No text is typed.
- **Expected outcome:** Nothing is sent, and nothing is replayed when the link comes back.
- **Status:** Fakes only: `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` checks that a paste while disconnected sends nothing, and that typing after reconnect arrives once.

### Paste into an exited shell

- **When:** The shell has exited, with `exit` or with ⌃D on an empty line.
- **I do:** Press ⌘V in the shell tab.
- **I see:** The same "Lost the connection … The program keeps running there … typing is off until then" banner. Nothing is typed. No exit note appears for a shell tab.
- **Expected outcome:** Nothing is sent. The tab says the shell exited, and does not claim the connection was lost.
- **Status:** Likely bug. The daemon does refuse typing into an exited shell: `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused`. The text shown to the user is wrong. See Likely bugs.

### Paste over SSH

- **When:** The shell runs on a remote machine over SSH.
- **I do:** Copy multi-line Unicode text, press ⌘V in the remote shell tab.
- **I see:** The remote program gets the same text, wrapped in the paste markers, with the same line breaks and Unicode characters.
- **Expected outcome:** The remote program receives the exact bytes.
- **Status:** Works (real shell): `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` (manual, external, real OpenSSH VM). The same test checks that ⌘V in an unfocused text field does not paste into the terminal.

### Paste while the agent is mid-turn

- **When:** An agent is writing its reply in the agent tab.
- **I do:** Press ⌘V in the agent tab.
- **I see:** The text appears in the agent's input, in order with other typed text.
- **Expected outcome:** The text goes to the agent's terminal now. Humata does not hold it. Holding messages for the end of a turn is a separate feature (SE-23).
- **Status:** Code, no test. The path is the same as in step 1 (`terminal/pane/src/host.rs:371-383`).

## Expected outcomes

- [ ] A one-line paste reaches the program as typed, with no Enter added.
- [ ] A multi-line paste to a program that asked for bracketed paste arrives wrapped in the markers, and runs nothing until Enter.
- [ ] A multi-line paste to a program that did not ask shows a prompt, and sends nothing until Paste is clicked.
- [ ] Cancel on that prompt sends nothing.
- [ ] An end marker inside pasted text cannot end the paste early.
- [ ] Edit > Paste does what ⌘V does.
- [ ] A paste into an exited or disconnected tab sends nothing.

## Status summary

Paths are relative to `apps/humata2`.

| Step or branch | Status | Evidence |
|---|---|---|
| 1. One line with ⌘V | Works (real shell) on macOS; Fakes only on Linux | `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines`; `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked` |
| 2. Five lines into zsh | Works (real shell) over SSH | `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` |
| 3. Five lines into Neovim | Code, no test | `terminal/pane/src/pane.rs:742-782` |
| 4. End marker in pasted text | Fakes only | `terminal/vt/tests/paste.rs::pasted_text_cannot_end_a_bracketed_paste_early` |
| 5. Edit > Paste | Code, no test | `desktop/src/main.rs:245`; no handler found |
| Branch: cancel the confirmation | Fakes only | `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user` |
| Branch: link down | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` |
| Branch: exited shell | Likely bug | Refusal: `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused`; text: `terminal/pane/src/pane.rs:903` |
| Branch: over SSH | Works (real shell) | `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` |
| Branch: agent mid-turn | Code, no test | `terminal/pane/src/host.rs:371-383` |

## Gaps and likely bugs

- Likely bug: the exited-shell banner says "Lost the connection … the program keeps running there". After `exit` or ⌃D, the shell is gone, so the text is wrong. The cause: the replica marks an exited terminal as not authorized (`terminal/client/src/replica.rs:530-536`), which turns the pane's disconnected state on, and the session view turns the banner on for any session that is not offline (`ui/src/session_tabs.rs:705-709`). The banner text is at `terminal/pane/src/pane.rs:903`. Confirm: ⌃D in a shell tab, then read the tab.
- Likely bug (unconfirmed): Edit > Paste (`desktop/src/main.rs:245`) sends `input::Paste`, and I found no terminal handler for it. If that is right, the menu item does nothing with a shell focused. Confirm: focus a shell tab, choose Edit > Paste, and check whether the clipboard text appears.
- Not built: none for paste text itself.
- Coverage gap: no test pastes several lines into Neovim or presses Enter after a multi-line paste into zsh.
