# A typo fixed with editing keys runs the corrected line

Layer: micro. Platforms: macOS (zsh), Linux (bash).

## Starting state

- Humata 2 is open on this Mac with one channel and one session.
- The session has a shell tab, and that tab has focus. On this Mac the shell is zsh. On the Linux machine it is bash.
- The prompt line is empty. The folder is a git repository with a clean working tree.
- The terminal has keyboard focus. No palette, find bar or paste prompt is open.
- Nothing is held: no pending paste and no queued input.

## Steps

1. **I do:** Type `git stauts --shrot`.

   **I see:** The line shows at the prompt with the cursor at the end. zsh echoes each letter.

   **State and events:** Each letter reaches zsh as its own byte. The view queues nothing.

   **Status:** Works (real shell). `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` runs a real zsh. That test types with the text path (`w.input`), not with per-key presses.

2. **I do:** Press Backspace once.

   **I see:** The line reads `git stauts --shro`. The cursor moves back one place.

   **State and events:** The key goes to the program as Backspace with no text (`terminal/view/src/keys.rs:150`). zsh should receive its erase byte, DEL (0x7f). No test asserts that byte.

   **Status:** Code, no test.

3. **I do:** Press ⌃W.

   **I see:** `--shro` is deleted. The line reads `git stauts ` with the cursor after the space.

   **State and events:** zsh should receive 0x17. The key has no text, because `keys.rs` drops control characters from the text (`terminal/view/src/keys.rs:85-90`). The encoder builds the byte from the key and Ctrl. No test sends Ctrl+W.

   **Status:** Code, no test.

4. **I do:** Type `--short`.

   **I see:** The line reads `git stauts --short`.

   **State and events:** Same as step 1.

   **Status:** Works (real shell). Same test as step 1.

5. **I do:** Press ⌃A.

   **I see:** The cursor jumps to the first column, before `g`.

   **State and events:** zsh should receive 0x01.

   **Status:** Code, no test.

6. **I do:** Press → eight times.

   **I see:** The cursor moves to just before the second `t` of `stauts`, which is after `git stau`.

   **State and events:** Eight cursor-forward sequences go to zsh. The form follows the cursor-key mode that zsh set (`ESC [ C` or `ESC O C`).

   **Status:** Fakes only. `terminal/vt/tests/input.rs::arrows_follow_the_cursor_key_mode` proves both forms of the encoding. No test moves zsh's own cursor with →.

7. **I do:** Press Backspace once, press → once, then type `u`.

   **I see:** The line reads `git status --short`. The `u` goes in before the final `s`.

   **State and events:** The Backspace deletes the `u` after `stau`. The → and the `u` go to zsh as in steps 2 and 6.

   **Status:** Code, no test (the Backspace is not proven; see step 2).

8. **I do:** Press ⌃E.

   **I see:** The cursor jumps to the end of the line.

   **State and events:** zsh should receive 0x05.

   **Status:** Code, no test.

9. **I do:** Press Enter.

   **I see:** zsh runs `git status --short` in this folder. Its output appears, and a new prompt is drawn.

   **State and events:** The Enter key goes to zsh as a carriage return (0x0d). zsh runs the line that is on screen, once.

   **Status:** Works (real shell). `interactive.rs` depends on Enter in its `command()` helper and checks the output of each command. It does not check that the edited line is the one that ran.

10. **I do:** Type `junk words here`, then press ⌃U.

    **I see:** The whole line clears. The cursor sits at the start of an empty prompt.

    **State and events:** zsh should receive 0x15 (kill to line start).

    **Status:** Code, no test. The Mac's ⌘⌫ does not do this; see `micro/02`.

11. **I do:** Type `echo one two`, press ← four times, then press ⌃K.

    **I see:** The line reads `echo one`. The text ` two` is gone.

    **State and events:** Four cursor-left sequences, then 0x0b (kill to end of line).

    **Status:** Code, no test. The ← presses are proven only by `arrows_follow_the_cursor_key_mode` (see step 6).

12. **I do:** Press ⌃H once.

    **I see:** The `e` before the cursor is deleted. The line reads `echo on`.

    **State and events:** zsh should receive 0x08.

    **Status:** Code, no test.

13. **I do:** Press ⌃L.

    **I see:** The screen clears. The prompt and `echo on` are drawn at the top.

    **State and events:** zsh should receive 0x0c and redraw. The typed text stays on the line.

    **Status:** Code, no test.

## Branches

### The pane loses its link while I type

- **When:** the link to the shell's machine drops in the middle of step 1.
- **I do:** Type `git stauts`, then press Enter.
- **I see:** The pane shows its disconnected banner (`disconnected-banner`). Nothing echoes.
- **Expected:** Keys are refused and nothing is sent. Keys typed while offline are not sent late. After the pane reconnects, only the keys typed from then on reach the shell.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` checks the banner, that nothing is sent while offline, and that a key typed after reconnect is sent alone. The real link drop is not proven live. `terminal/pane/src/host.rs:257` drops keys without an error, so the banner is the only signal.

### I type straight after a new shell opens

- **When:** I open a shell from the palette (Open a shell) and type at once.
- **I do:** Type `ls` right after the tab appears.
- **I see:** The new prompt shows `ls`.
- **Expected:** The first keys land, because the new tab takes focus when it opens.
- **Status:** Fakes only. `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click`. A key that arrives while the view is not focused is counted and dropped (`terminal/view/src/view.rs:851-856`), so the focus step is what makes this work. No real window and real shell test covers it.

### I hold Backspace down

- **When:** I hold Backspace on a long line.
- **I do:** Hold Backspace for about a second.
- **I see:** Characters go one at a time at the key-repeat rate.
- **Expected:** One erase per repeat, in order, and nothing lost.
- **Status:** Code, no test. A repeat is sent with `KeyAction::Repeat` (`terminal/view/src/view.rs:907`). No test sends a repeat for any key.

### I type in a split that is not focused

- **When:** two shells are split side by side.
- **I do:** Click the left shell, then type `ls`.
- **I see:** `ls` appears in the left shell only.
- **Expected:** Keys go to the focused split only. A click moves focus.
- **Status:** Fakes only. `desktop/tests/native/terminal.rs::clicking_a_split_terminal_focuses_that_group` covers focus. `::an_unfocused_terminal_does_not_receive_palette_keystrokes` covers an unfocused terminal taking no letters. No test types into a real split shell.

### The same keys on Linux bash

- **When:** the same line on the Linux machine.
- **I do:** Repeat steps 2 to 13 with Ctrl in place of ⌃.
- **I see:** bash makes the same edits: Ctrl+W, Ctrl+U, Ctrl+K, Ctrl+A and Ctrl+E, Ctrl+H, Ctrl+L.
- **Expected:** The same bytes as the Mac. `terminal/view/src/keys.rs:66-78` leaves Ctrl letters to the program on the Linux branch.
- **Status:** Code, no test. The Linux runs wait for the LINUX milestone, which is blocked (`workflow/milestones/LINUX.md`).

## Expected outcomes

- [ ] Each typed letter appears at the prompt in the order typed.
- [ ] Backspace deletes the character before the cursor.
- [ ] ⌃W deletes the word before the cursor and sends 0x17.
- [ ] ⌃U clears the line and sends 0x15.
- [ ] ⌃K deletes from the cursor to the end and sends 0x0b.
- [ ] ⌃A and ⌃E move to the start and end of the line.
- [ ] ← and → move the cursor, and their bytes follow the shell's cursor-key mode.
- [ ] Text typed after moving the cursor goes in at the cursor.
- [ ] Enter runs the corrected line once.
- [ ] ⌃L clears the screen and keeps the text on the prompt line.
- [ ] Keys typed while the pane is disconnected are refused, and the pane says so.
- [ ] Keys typed into a new shell reach it without a click.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Type letters | Works (real shell) | `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` |
| 2 Backspace | Code, no test | `terminal/view/src/keys.rs:150` |
| 3 ⌃W | Code, no test | `terminal/view/src/keys.rs:81-90` |
| 4 Type `--short` | Works (real shell) | Same as step 1 |
| 5 ⌃A | Code, no test | `terminal/view/src/keys.rs:81-90` |
| 6 Right arrow ×8 | Fakes only | `terminal/vt/tests/input.rs::arrows_follow_the_cursor_key_mode` |
| 7 Backspace, →, `u` | Code, no test | Backspace unproven |
| 8 ⌃E | Code, no test | `terminal/view/src/keys.rs:81-90` |
| 9 Enter runs the line | Works (real shell) | `interactive.rs`, `command()` helper |
| 10 ⌃U | Code, no test | `terminal/view/src/keys.rs:81-90` |
| 11 ← ×4, ⌃K | Code, no test | No test sends ⌃K |
| 12 ⌃H | Code, no test | No test sends ⌃H |
| 13 ⌃L | Code, no test | No test sends ⌃L |
| Branch: link drops | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` |
| Branch: type into a new shell | Fakes only | `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click` |
| Branch: held Backspace | Code, no test | `terminal/view/src/view.rs:907` |
| Branch: unfocused split | Fakes only | `desktop/tests/native/terminal.rs::clicking_a_split_terminal_focuses_that_group` |
| Branch: Linux bash | Code, no test | LINUX milestone (blocked) |

## Gaps and likely bugs

- Not built: nothing in this case is missing from the code. Every key has a path to the shell.
- Likely bug: none found on the path for these keys.
- Unproven: no test sends Backspace, ⌃W, ⌃U, ⌃K, ⌃A, ⌃E, ⌃H, ⌃L or → to a real shell. A pane test on the recording PTY could assert 0x7f, 0x17, 0x15, 0x0b, 0x01, 0x05, 0x08 and 0x0c with the real encoder, and `interactive.rs` could add the editing keys to its real zsh run.
