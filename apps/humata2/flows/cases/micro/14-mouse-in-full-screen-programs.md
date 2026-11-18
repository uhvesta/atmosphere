# The mouse drives htop and vim, ⇧ selects text instead, and less scrolls its file

Micro. Platforms: macOS, with Linux noted where it differs. Paths are relative to `apps/humata2`.

## Starting state

- Humata 2 is open on this Mac, with one session open. Its tab is a zsh shell on this Mac.
- The terminal has keyboard focus. No overlay is open.
- In the shell, `htop` is started in the tab. In a second tab, `vim` runs with `set mouse=a` and a file open.
- A third tab has a shell with `less /var/log/system.log` (or any long log) open on the alternate screen.
- The mouse is over the terminal, and no button is held.

## Steps

1. **I do:** in htop, click a process row, then drag over the column header.
   **I see:** htop selects the row under the pointer and then reacts to the drag. Nothing is selected as terminal text.
   **State and events:** the view sends an SGR press and release for each button event, with the cell under the pointer as 1-based column and row (`terminal/vt/tests/input.rs::an_sgr_mouse_press_reports_the_cell_under_the_pointer`). The program takes the events, and the pane does not start a text selection (`terminal/view/src/view.rs:1000-1010`).
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` sends a drag and checks the bytes start with `\x1b[<0;3;1M` and end with `\x1b[<0;3;1m`. The program is not htop, and the bytes are checked by a recording pty, not a real program. htop itself is not tested.

2. **I do:** in vim with `mouse=a`, click on line 4, column 12.
   **I see:** the cursor moves to that place. Nothing is selected.
   **State and events:** the press is sent to vim as an SGR report, and vim moves its cursor.
   **Status:** Works (real shell)
   Evidence: `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` drags at column 12.5, row 4.5 in a real Neovim with `mouse=a`, and asserts the cursor is on row 4. This is the manual target (`neovim_test`, macOS), which needs `TERMINAL_LAB_NVIM`.

3. **I do:** in htop or vim, scroll the wheel down three notches over the program.
   **I see:** the program scrolls its own list. The terminal's history does not move.
   **State and events:** each notch is a mouse button 5 report, encoded with the wheel bits and the pointer cell (`terminal/pane/src/host.rs:169-192`, `:293-311`). With mouse tracking on, the viewport does not scroll.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` checks the wheel sends `\x1b[<64;`. A real htop or vim wheel is not tested. `neovim_test` does not scroll.

4. **I do:** hold ⇧ and drag over `some` on the shell's output, while vim or htop is running with mouse tracking on.
   **I see:** the text `some` is selected, and the program gets nothing.
   **State and events:** with Shift held, the press, the drag and the release are not reported to the program (`terminal/view/src/view.rs:1000`, `:1112`, `:1150`). The selection starts as it would without tracking.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` (Shift is set with `ModifiersChanged`, drag selects `some`, the bytes sent are empty, ⌘C copies `some`).

5. **I do:** hold ⇧ and scroll the wheel over htop.
   **I see:** the terminal's history should scroll, and htop should not react.
   **State and events:** the wheel is not checked for Shift. `scroll` (`terminal/view/src/view.rs:1159-1175`) passes the modifiers to `wheel_bytes`, and with mouse tracking on it sends the report with the Shift bit set (`terminal/pane/src/host.rs:169-192`, `:293-302`). htop gets the wheel.
   **Status:** Fakes only. With ⇧ held the wheel scrolls the history and the program gets no report, as clicks and drags already did (`terminal/pane/tests/pane.rs::shift_wheel_scrolls_the_history_while_the_program_takes_the_mouse`).

6. **I do:** in `less` on the alternate screen, scroll the wheel down three notches.
   **I see:** the file scrolls down by about three lines.
   **State and events:** `less` has no mouse tracking, but it turns on alternate scroll (mode 1007). The wheel becomes arrow keys (`terminal/pane/src/host.rs:193-199`). The arrows reach `less`, and `less` scrolls the file.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::on_the_alternate_screen_the_wheel_sends_arrows_when_asked` feeds the sequence `\x1b[?1049h\x1b[?1007h` by hand and checks two wheel notches send `\x1b[A\x1b[A`. The real `less` is not run, so it is not shown that `less` sends 1007. The comment at `terminal/pane/src/host.rs:168` says it does.

7. **I do:** in a normal shell, with output longer than the screen, scroll the wheel up.
   **I see:** the history scrolls up. The shell does not get arrow keys.
   **State and events:** no mouse tracking, no alternate screen, so the wheel scrolls the viewport by whole rows (`terminal/pane/src/host.rs:314-318`).
   **Status:** Fakes only
   Evidence: `terminal/view/tests/view.rs::scrolling_sends_whole_rows_older_output_first` and `small_pixel_scrolls_add_up_to_a_row`. These use a fake grid and a fake host.

8. **I do:** switch to another app (⌘Tab), then come back to the `vim` tab.
   **I see:** vim notes focus out and focus in, for example when a focus-aware plugin is used. htop does not react.
   **State and events:** a focus report (`CSI O` on loss, `CSI I` on gain) is sent only if the program asked with mode 1004 (`terminal/pane/src/host.rs:414-421`, `terminal/vt/src/terminal/view_support.rs`).
   **Status:** Code, no test
   Evidence: the in-window case is tested: `terminal/pane/tests/pane.rs::programs_that_ask_are_told_when_the_pane_gains_and_loses_focus` moves focus with the find bar, not with another app. The engine rule is tested by `terminal/vt/tests/view_support.rs::focus_is_reported_only_when_the_program_asked`. Nothing observes the window losing or gaining activation: `grep` for `window_activation` and `is_window_active` in the terminal code finds only a debug line in `terminal/client/src/lib.rs:367`. Whether GPUI calls the blur handler when the app loses focus is not shown in the repo.

9. **I do:** in a shell that did not ask for focus reports, switch apps and back.
   **I see:** nothing is typed into the shell.
   **State and events:** no `CSI I` or `CSI O` is sent, because mode 1004 is off.
   **Status:** Fakes only
   Evidence: `terminal/vt/tests/view_support.rs::focus_is_reported_only_when_the_program_asked` checks `focus_report` returns `None` until 1004 is set.

## Branches

### The mouse in a read-only attach

- **When:** the user opens a session that another app holds, or a session shared as read-only.
- **I do:** click and drag in htop, then scroll the wheel.
- **I see:** nothing is sent to htop. Its screen still updates as output arrives. No selection starts in the read-only view.
- **State and events:** the replica is not the controller, so it sets the pane to not connected (`terminal/client/src/replica.rs:528-541`). The pane drops every mouse report and every wheel report at the gate (`terminal/pane/src/host.rs:119-123`, `:281-291`, `:293-311`).
- **Expected outcome:** nothing is sent. The daemon refuses typing in a read-only attach, too (`daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type`, Works (real daemon)).
- **Status:** Code, no test
  Evidence: the gate is `Shared::send` (`terminal/pane/src/host.rs:119-123`). The pane test `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` covers keys and paste while disconnected, not the mouse. No test covers the mouse on a read-only pane.

### The wheel over the scrollbar

- **When:** the pointer is over the 12-point strip at the right edge, where the scrollbar's thumb is.
- **I do:** scroll the wheel with the pointer on the scrollbar, in htop and in a normal shell.
- **I see:** in a normal shell, the history scrolls. In htop, the wheel goes to htop at the pointer's column, which is the rightmost column.
- **State and events:** the wheel handler does not check the scrollbar area (`terminal/view/src/view.rs:1159-1175`). Only a press on the scrollbar is taken (`view.rs:1031-1050`, called from `mouse_down` at `view.rs:972-990`).
- **Expected outcome:** the wheel acts as it does over the cells next to it. This matches the rule for the rest of the view.
- **Status:** Code, no test

### A program crashes and leaves mouse mode on

- **When:** htop or vim is killed with `kill -9` while mouse tracking is on, and the shell is back at the prompt.
- **I do:** run `printf '\e[?1000h\e[?1006h'` in a shell, then kill the process that set it, then move and scroll the wheel over the shell.
- **I see:** with nothing resetting the mode, the wheel and clicks are reported as text into the shell: a line of `^[[<64;...M` appears in the shell's input. The pane does not reset the mode, so the shell keeps getting reports until the next program or a `reset`.
- **State and events:** the terminal state holds mouse tracking after its program is gone. The pane gates on `is_mouse_tracking()` only (`terminal/pane/src/host.rs:281-283`), and no code resets modes when a program exits. `grep` for reset sequences in `terminal/pane/src`, `terminal/host/src`, `terminal/client/src` and `daemon/holder/src` finds none.
- **Expected outcome:** either the shell gets the reports (many terminals do this too), or the pane clears the modes when its program ends. The brief asks which. The code does the first.
- **Status:** Fakes only, as designed (D483). The shell gets the reports, as in most terminals: the pane cannot tell that the program that asked has gone while the shell lives on. ⇧ still selects and scrolls (`terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held`), `reset` clears the mode, and when the tab's own program ends the mouse goes back to the history (`terminal/pane/tests/pane.rs::with_nobody_listening_the_mouse_goes_back_to_the_history`).

### Mouse events during a link drop

- **When:** a session on `box-1` loses its link while htop runs.
- **I do:** click in htop while the link is down.
- **I see:** nothing reaches the machine. The pane shows the disconnected state.
- **Expected outcome:** the click is dropped and not sent late (SE-16). See micro-16.
- **Status:** Fakes only
  Evidence: the gate is the same as the read-only case (`terminal/pane/src/host.rs:119-123`). The banner is `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so`, which checks keys only.

## Expected outcomes

- [ ] With mouse tracking on, a press, drag, release and wheel go to the program as SGR reports, with the cell under the pointer.
- [ ] With ⇧ held, a press, drag and release select text and send nothing.
- [ ] A wheel with ⇧ held does the same as a wheel without it (see the Likely bug below).
- [ ] On the alternate screen with mode 1007 on, the wheel sends up and down arrows.
- [ ] Without mouse tracking or alternate scroll, the wheel scrolls the history by rows.
- [ ] Focus reports are sent only when the program set mode 1004.
- [ ] A read-only or disconnected pane sends no mouse or wheel report.
- [ ] A program that leaves mouse mode on does not leave the shell in a wrong state (see the Likely bug below).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. htop click and drag as SGR | Fakes only | `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held`; `terminal/vt/tests/input.rs::an_sgr_mouse_press_reports_the_cell_under_the_pointer` |
| 2. vim click moves the cursor | Works (real shell) | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (manual) |
| 3. Wheel reported to the program | Fakes only | `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` |
| 4. ⇧ selects and sends nothing | Fakes only | `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held` |
| 5. ⇧ wheel over htop | Fakes only | `terminal/pane/tests/pane.rs::shift_wheel_scrolls_the_history_while_the_program_takes_the_mouse` |
| 6. less wheel becomes arrows | Fakes only | `terminal/pane/tests/pane.rs::on_the_alternate_screen_the_wheel_sends_arrows_when_asked` |
| 7. Wheel scrolls history | Fakes only | `terminal/view/tests/view.rs::scrolling_sends_whole_rows_older_output_first` |
| 8. Focus out and in on app switch | Code, no test | `terminal/pane/tests/pane.rs::programs_that_ask_are_told_when_the_pane_gains_and_loses_focus` (in-window only) |
| 9. No focus report unless asked | Fakes only | `terminal/vt/tests/view_support.rs::focus_is_reported_only_when_the_program_asked` |
| Branch: read-only attach | Code, no test | `terminal/pane/src/host.rs:119-123`, `terminal/client/src/replica.rs:528-541` |
| Branch: wheel over scrollbar | Code, no test | `terminal/view/src/view.rs:1159-1175` |
| Branch: crash leaves mouse mode on | Fakes only | `terminal/pane/tests/pane.rs::programs_that_take_the_mouse_get_it_unless_shift_is_held`, `terminal/pane/tests/pane.rs::with_nobody_listening_the_mouse_goes_back_to_the_history`; D483 |
| Branch: link drop during htop | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` |

## Gaps and likely bugs

- **Not built:** nothing in the repo runs htop, `less`, or a real wheel in a real window. No test sends a real wheel to a real program. The lab's Neovim test covers a click and no wheel.
- **Fixed:** with ⇧ held the wheel scrolls the history, as clicks and drags do (`terminal/pane/tests/pane.rs::shift_wheel_scrolls_the_history_while_the_program_takes_the_mouse`, Fakes only).
- **As designed (D483):** a program that leaves mouse tracking on when it is killed leaves it on for the shell, which gets the reports; ⇧ still selects and `reset` clears it.
- **Code, no test:** the app-switch focus report. The pane only sees its own focus handle (`terminal/view/src/view.rs` `on_focus` and `on_blur`). Whether GPUI blurs the handle when the whole app loses activation is outside the repo; confirm on the Mac.
- **Code, no test:** `less` on the alternate screen. The code comment says `less` and `vim` set mode 1007 (`terminal/pane/src/host.rs:168`), but no test runs `less`.
- **Code, no test:** the wheel over the scrollbar and the mouse on a read-only pane.
