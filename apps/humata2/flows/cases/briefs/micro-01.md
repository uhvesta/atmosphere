# Brief micro-01: Type a command, fix a typo, run it
Output: `apps/humata2/flows/cases/micro/01-type-fix-a-typo-and-run.md`

Scenario: I type `git stauts` into a shell tab, notice the typo, fix it with several editing keys, and run it.

Starting state: a channel session with a shell tab focused on this Mac (zsh), and the same on Linux (bash).

Steps to cover: type letters; Backspace; ⌃H; ⌃W (delete word back); ⌃U (kill line); ⌃K (kill to end); ⌃A / ⌃E; ← / → to the middle and insert; Enter runs the fixed line; ⌃L clears the screen; what the bytes are (e.g. ⌃W = 0x17).

Branches: typing while the pane is disconnected (input dropped, pane says so); typing before the new session is focused (first keys land?); key repeat on held Backspace; typing in a split that is not focused.

Pointers:
- terminal/view/src/keys.rs (KeyIntent; control text dropped from key_char, encoder builds bytes); terminal/vt/tests/input.rs (enter, ctrl_c_sends_etx, arrows by cursor key mode); terminal/pane/tests/pane.rs `while_disconnected_input_is_dropped_and_the_pane_says_so`; desktop/tests/native/terminal.rs (`new sessions get input without an extra click`, accepts keys); terminal/lab/tests/interactive.rs `actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` (real zsh).
- Check whether any test sends ⌃W/⌃U/⌃K specifically; if not, mark "Code, no test" with the encoder path.
