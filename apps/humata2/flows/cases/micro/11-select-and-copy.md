# A selection copies as plain text, and ⌘C never interrupts a program

Layer: micro. Platforms: macOS, Linux.

## Starting state

- Humata 2 is open on this Mac, with a shell tab in the session `ledger`.
- The tab shows a stack trace of about 40 lines. Some lines are longer than the window and wrap.
- The shell is idle, with its prompt showing. Nothing is selected.
- The clipboard holds the text `old text`.
- The tab keeps its default history.

## Steps

1. **I do:** I drag from the start of `at` in the stack trace to the end of a line.
   **I see:** The selected cells change to the selection colours. Nothing else changes.
   **State and events:** The view sends the drag to the emulator as a selection. The emulator reports `has_selection`. Nothing is sent to the shell.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` checks the drag and that copying sends nothing to the program. The PTY is a fake.

2. **I do:** I double-click a word, then triple-click a line.
   **I see:** Double-click selects the word, up to a blank or a boundary character such as a bracket or a colon. Triple-click selects the whole logical line, including the rows it wrapped onto.
   **State and events:** Word and line granularity come from `terminal/view/src/selection.rs` (`word` at lines 60-80, `line` after it).
   **Status:** Fakes only. `terminal/view/tests/view.rs::a_double_click_selects_a_word_and_a_triple_click_a_line` checks both gestures on a fake grid. `terminal/view/src/selection.rs::a_double_click_selects_the_word_up_to_blanks_and_boundaries` checks the boundary rule.

3. **I do:** I hold ⌥ and drag over a column of text.
   **I see:** A rectangle of cells is selected across the rows, not a run of text in reading order.
   **State and events:** The view asks the emulator for a rectangular selection.
   **Status:** Fakes only. `terminal/view/tests/view.rs::an_alt_drag_selects_a_block` checks the rectangle on a fake grid. On Linux the key is Alt, and no real run confirms it.

4. **I do:** I select the stack-trace line that wraps across two rows and press ⌘C (Ctrl+Shift+C on Linux). Then I paste into a text field.
   **I see:** The pasted text is one line. There is no new line where the wrap was. Wide characters, such as CJK text, are whole. No colour codes are pasted.
   **State and events:** The selection joins wrapped rows without a line break. The clipboard gets plain text. The terminal sends nothing to the shell.
   **Status:** Fakes only. `terminal/vt/tests/selection.rs::a_selection_across_a_wrapped_line_copies_as_one_line` and `terminal/vt/tests/selection.rs::a_viewport_selection_copies_wide_characters_across_a_wrap` run the real engine in-process, with no shell. The "sends nothing" part is in `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text`.

5. **I do:** In a real shell, I select the output of a command and press ⌘C. Then I paste.
   **I see:** The clipboard holds exactly the selected output. The paste shows it.
   **State and events:** ⌘C is `KeyIntent::Copy` (`terminal/view/src/keys.rs:53-61`). The selected text goes to the clipboard. Nothing goes to the shell.
   **Status:** Works (real shell) on macOS. `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` drags over a row of real zsh output, presses `cmd-c`, and checks that the clipboard holds `PASTE_VERIFIED` (lines 375-416). It is a manual target that needs the owner's zsh home. Linux Ctrl+Shift+C is Fakes only, in `terminal/pane/tests/pane.rs`, which uses `ctrl-shift-c` on Linux.

6. **I do:** I scroll up into the history and drag across older output. Then I press ⌘C.
   **I see:** The copied text is the older rows that I selected.
   **State and events:** The selection is in the emulator's history, and `selected_text` reads it from there.
   **Status:** Fakes only. `terminal/vt/tests/selection.rs::a_selection_can_reach_into_scrollback` checks the copied text from history. No test scrolls and drags in a real shell.

7. **I do:** I press ⌘C with nothing selected.
   **I see:** Nothing is copied. The clipboard keeps its old text. Nothing is sent to the shell.
   **State and events:** The pane returns `None` when there is no selection (`terminal/pane/src/host.rs:361-369`). The view writes to the clipboard only for `Some` (`terminal/view/src/view.rs:865-869`).
   **Status:** Code, no test. The copy test selects text first. No test copies with no selection.

8. **I do:** I select some text and press ⌃C while a program is running, such as `sleep 100`.
   **I see:** The program stops, as it does with ⌃C in any shell. Nothing is copied. On Linux, Ctrl+C sends the interrupt and Ctrl+Shift+C copies.
   **State and events:** On macOS, ⌃C is not a copy key, so it goes to the program as byte `0x03` (`terminal/view/src/keys.rs:53-61`). On Linux, Copy needs Ctrl+Shift (`terminal/view/src/keys.rs:66-69`). The selection does not change the key's meaning.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::typed_keys_reach_the_program_in_its_encoding` checks that `ctrl-c` sends `0x03`. That test has no selection on screen. The key mapping does not read the selection, so the combination is inferred, not tested.

9. **I do:** I select text and choose **Edit > Copy** from the menu bar.
   **I see:** The selected text is copied. Pasting shows it.
   **State and events:** The menu item is `input::Copy` (`desktop/src/main.rs:244`). The terminal view has no handler for that action. Its text input handler reports no selected text (`terminal/view/src/view.rs:1541-1557`), so there is nothing for the menu to copy.
   **Status:** Likely bug. `desktop/src/main.rs:244` routes Copy to an action that the terminal does not handle, and the terminal's input handler returns an empty selection. Confirm: select text in a shell tab, choose **Edit > Copy**, then paste. Expect the old clipboard text, or nothing new.

10. **I do:** A program in the shell writes to the clipboard with OSC 52, for example `printf '\033]52;c;aGVsbG8=\007'`.
    **I see:** The clipboard now holds `hello`. Nothing is asked.
    **State and events:** The pane emits a clipboard write event. The app copies its text (`ui/src/terminals.rs:197-199`). No prompt is shown. `DECISIONS.md` line 226 says remote OSC 52 writes reach the Mac clipboard.
    **Status:** Fakes only. `terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` checks the pane event. The app's copy call at `ui/src/terminals.rs:197-199` has no test.

## Branches

### A program that takes the mouse

- **When:** A full-screen program such as `htop` or `vim` has turned on mouse reporting.
- **I do:** I drag over some text, with no modifier. Then I drag again while holding ⇧.
- **I see:** The plain drag goes to the program, not to a selection. The ⇧ drag selects the text, and ⌘C copies `some`.
- **State and events:** Mouse events go to the program unless ⇧ is held. A held ⇧ routes the drag to the selection.
- **Expected outcome:** The program gets the plain drag. The ⇧ drag selects and copies.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` checks both drags with a fake PTY.

### Output arrives while I select

- **When:** The build prints while I drag a selection.
- **I do:** I drag over several lines while output keeps arriving.
- **I see:** The selection should stay on the text I chose. The selected text should not shift to the new output.
- **State and events:** Output is fed into the emulator (`terminal/pane/src/pane.rs:150`). Nothing in that path clears the selection. Only `replace_grid` clears the anchor and the selection (`terminal/view/src/view.rs:357-364`), and that runs when a snapshot is installed, not for normal output.
- **Expected outcome:** The selection keeps its cells and copies the same text.
- **Status:** Code, no test. No test feeds output while a selection is held. The replacement path, which drops the selection on a snapshot, is also untested.

### OSC 52 write from a remote shell over SSH

- **When:** The shell is on the SSH test VM, not on this Mac.
- **I do:** In the remote shell I run the OSC 52 write `printf '\033]52;c;aGVsbG8=\007'`.
- **I see:** The clipboard on this Mac holds `hello`. No prompt is shown.
- **State and events:** The write passes through the remote holder and the SSH link. The replica raises the same clipboard event as a local write. `DECISIONS.md` line 226 allows it.
- **Expected outcome:** The text reaches the Mac clipboard, the same as a local write.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` covers the local case. `terminal/lab/tests/clipboard_ssh.rs` does not send OSC 52. It copies ordinary output and checks paste-back bytes. Remote OSC 52 is not proven live.

### OSC 52 read from a program

- **When:** A program asks to read the clipboard, for example `printf '\033]52;c;?\007'`.
- **I do:** I run the read request in the shell.
- **I see:** Nothing appears on the screen. The program receives an answer that the clipboard is empty. No prompt asks me first.
- **State and events:** The host answers reads at once with an empty clipboard (`terminal/host/src/lib.rs:2111-2121`). The pane has `answer_clipboard_read` (`terminal/pane/src/pane.rs:306-310`), but only the lab calls it (`terminal/lab/src/lib.rs:396`). The app's terminal subscription has no arm for a read request (`ui/src/terminals.rs:178-211`).
- **Expected outcome:** The read is refused or allowed only after the person says so. `DECISIONS.md` line 226 says reads are never answered without the user. In the app, the refusal is automatic, and nothing asks the person.
- **Status:** Code, no test. `terminal/vt/tests/clipboard.rs::the_host_answers_a_read_with_text_or_an_empty_clipboard` checks the reply bytes only. No test checks what the app does with a read request.

### Copy in a read-only attach

- **When:** I attach to a session that another person controls. I am a read-only viewer.
- **I do:** I select some output and press ⌘C.
- **I see:** The selected text is copied.
- **State and events:** Copy is local. It reads the selection from the emulator and writes the clipboard, with no check on whether the viewer is connected or controlling (`terminal/pane/src/host.rs:361-369`, `terminal/view/src/view.rs:865-869`).
- **Expected outcome:** Copy works in a read-only attach, because it sends nothing to the host.
- **Status:** Code, no test. No test selects and copies in a read-only viewer.

## Expected outcomes

- [ ] A drag, a double-click and a triple-click select the expected text.
- [ ] A ⌥-drag selects a rectangle.
- [ ] A selection across a soft wrap copies as one line, with no line break at the wrap.
- [ ] Wide characters copy whole.
- [ ] ⌘C copies the selection to the clipboard and sends nothing to the program.
- [ ] ⌃C sends the interrupt to the program, and does not copy.
- [ ] Selection reaches into scrollback.
- [ ] A plain mouse drag goes to a program that takes the mouse. ⇧-drag selects.
- [ ] A program's OSC 52 write reaches the clipboard.
- [ ] A program's OSC 52 read is refused or asked, never answered silently.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Drag to select | Fakes only | `terminal/pane/tests/pane.rs::dragging_selects_and_the_copy_shortcut_copies_the_text` |
| 2. Double- and triple-click | Fakes only | `terminal/view/tests/view.rs::a_double_click_selects_a_word_and_a_triple_click_a_line` |
| 3. ⌥-drag block | Fakes only | `terminal/view/tests/view.rs::an_alt_drag_selects_a_block` |
| 4. Wrapped line and wide characters copy | Fakes only | `terminal/vt/tests/selection.rs::a_selection_across_a_wrapped_line_copies_as_one_line` |
| 5. ⌘C in a real shell | Works (real shell) | `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` (macOS, manual) |
| 6. Selection in scrollback | Fakes only | `terminal/vt/tests/selection.rs::a_selection_can_reach_into_scrollback` |
| 7. ⌘C with nothing selected | Code, no test | `terminal/pane/src/host.rs:361-369` |
| 8. ⌃C with a selection | Fakes only | `terminal/pane/tests/pane.rs::typed_keys_reach_the_program_in_its_encoding` (no selection) |
| 9. Edit > Copy | Likely bug | `desktop/src/main.rs:244`, `terminal/view/src/view.rs:1541-1557` |
| 10. OSC 52 write, local | Fakes only | `terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` |
| Branch: program that takes the mouse | Fakes only | `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` |
| Branch: output while selecting | Code, no test | `terminal/view/src/view.rs:357-364` |
| Branch: OSC 52 write over SSH | Fakes only | `terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` (local only) |
| Branch: OSC 52 read | Code, no test | `terminal/host/src/lib.rs:2111-2121`, `ui/src/terminals.rs:178-211` |
| Branch: copy in a read-only attach | Code, no test | `terminal/pane/src/host.rs:361-369` |

## Gaps and likely bugs

- Not built: a prompt that asks the person before a program reads the clipboard with OSC 52. `terminal/lab/ACCEPTANCE.md` line 129 lists consent for clipboard reads as a host integration still to do. No milestone covers it.
- Likely bug: Edit > Copy does nothing in a terminal. `desktop/src/main.rs:244` routes to `input::Copy`, which the terminal does not handle, and `terminal/view/src/view.rs:1541-1557` reports no selected text. Confirm as in step 9.
- Likely bug: the app drops terminal events it does not name, including clipboard read requests. `ui/src/terminals.rs:211` is a catch-all `_ => {}`. The same arm drops bells, title changes and working-directory changes. Confirm by sending an OSC 52 read from a shell and watching that nothing is asked.
- Note on double-click copy: `:` is a word boundary (`terminal/view/src/selection.rs:10`), so a double-click on a path with a line number copies only the path part. That bug is in micro-12, step 3.
