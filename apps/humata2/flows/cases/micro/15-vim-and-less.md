# Neovim gets every key, and the shell comes back as it was when vim or less quits

Micro. Platforms: macOS (the Neovim test is macOS only), with Linux noted where it differs. Paths are relative to `apps/humata2`.

## Starting state

- Humata 2 is open on this Mac, with one session open. Its tab is a zsh shell on this Mac, in a folder holding `terminal.rs`.
- The shell prompt shows, and the terminal has keyboard focus. No overlay is open.
- `nvim` is installed, and `TERMINAL_LAB_NVIM` names it for the test run.
- A log file of a few thousand lines is in the folder, for `less`.

## Steps

1. **I do:** run `nvim terminal.rs` in the shell.
   **I see:** the file opens in vim's full screen. Syntax colors show. The status line is at the bottom.
   **State and events:** the program switches to the alternate screen, and the view draws its cells. The colors come through as 24-bit RGB cell colors.
   **Status:** Works (real shell)
   Evidence: `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` opens `terminal.rs` in a real Neovim and asserts that the screen contains `struct Session`.

2. **I do:** look at the colors.
   **I see:** keywords, strings and comments in different colors.
   **State and events:** at least five different RGB foreground colors are on screen.
   **Status:** Works (real shell)
   Evidence: `neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` collects the RGB foreground colors and asserts there are at least five. The manual target also keeps a screenshot under `terminal/lab/tests/__snapshots__/neovim/`.

3. **I do:** press `i`, type `// unsaved edit `, then press Esc.
   **I see:** the text appears in the buffer. After Esc the mode line shows NORMAL.
   **State and events:** each key is sent to vim. Esc is sent as the Escape key (`terminal/view/src/keys.rs:164`), not taken by the app.
   **Status:** Works (real shell)
   Evidence: `neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` presses `i`, types the text, presses Escape, and asserts the text is on screen. The test does not check the mode line, so it does not prove that Esc left insert mode. See the gaps below.

4. **I do:** confirm Esc reaches vim and not the app. Press Esc in normal mode, then in insert mode.
   **I see:** vim gets the key both times. The app does not close anything, because nothing is open.
   **State and events:** the app binds `escape` to `Dismiss` with no context (`ui/src/keys.rs:98`). The terminal's `key_down` sends the key to the program and stops propagation (`terminal/view/src/view.rs:929-931`), so the binding does not run while the terminal has focus. `key_down` returns early during a composition (`view.rs:858-861`).
   **Status:** Code, no test
   Evidence: the routing is in `terminal/view/src/view.rs:851-931` and `ui/src/keys.rs:98`. The Neovim test presses Escape while the terminal is focused, so it runs this path, but it asserts text, not that the app consumed nothing. The rule that GPUI runs key listeners before action bindings is not in this repo.

5. **I do:** resize the window by dragging its corner, then drag it back.
   **I see:** vim redraws at the new size. Its line numbers and status line fit the new width. The text stays.
   **State and events:** the pane sends the new size to the PTY (`terminal/pane/tests/pane.rs::resizing_the_window_tells_the_program_the_new_size`, fake PTY), and vim gets SIGWINCH and redraws.
   **Status:** Works (real shell)
   Evidence: `neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` resizes the window and the font, and asserts the text is still on screen. It does not check the size vim reports. Over SSH, the rows change is covered by `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell` (manual target).

6. **I do:** press ⌃Z in vim.
   **I see:** vim stops, and the shell prints `[1]+ Stopped nvim terminal.rs`.
   **State and events:** ⌃Z is sent as byte `0x1a` to the PTY. The shell's job control stops vim. The alternate screen is left, and the shell's screen comes back.
   **Status:** Code, no test
   Evidence: `terminal/view/src/keys.rs` maps `ctrl-z` to a key with no text, and `ctrl-c` is checked by `keys.rs::ctrl_c_goes_to_the_program_on_every_platform`. No test checks ⌃Z. `terminal/vt/tests/input.rs` tests ⌃C only. No test stops a real vim with ⌃Z.

7. **I do:** type `fg` in the shell.
   **I see:** vim comes back full screen, with the same buffer and cursor.
   **State and events:** the shell sends SIGCONT to vim's process group. vim redraws its full screen from its own state.
   **Status:** Code, no test
   Evidence: the job control is the shell's. No test in the repo runs `fg` or checks the redraw after it.

8. **I do:** type `:wq` and press Enter.
   **I see:** vim closes. The shell prompt returns, and the screen is exactly as it was before step 1 (the prompt line, the command, and any earlier output).
   **State and events:** vim writes the file and leaves the alternate screen (`?1049l`). The cursor is shown again (`?25h`). The pane's screen equals its screen before the program started.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::a_full_screen_program_leaves_the_shell_as_it_was` feeds the enter and leave sequences by hand and checks the screen is equal to the one before. The real vim quit is not tested. `neovim.rs` writes with `:write`, not `:wq`, and then stops the terminal.

9. **I do:** run `less /var/log/system.log` (or the log in the folder).
   **I see:** the log fills the screen. The status line shows the file name and a percentage.
   **State and events:** `less` switches to the alternate screen. The view draws its cells.
   **Status:** Code, no test
   Evidence: no test runs `less`.

10. **I do:** type `/error` and press Enter.
    **I see:** the first match is highlighted and the status line shows it.
    **State and events:** the keys go to `less` as typed text. `less` draws its own highlight. The view does not treat `/` as a shortcut. The app's find bar is ⌘F, which the pane takes (`terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back`), not `/`.
    **Status:** Code, no test
    Evidence: no test runs `less`. The find bar's own tests use the pane's history, not `less`.

11. **I do:** press `q`.
    **I see:** `less` closes. The shell prompt returns with the screen as it was before step 9.
    **State and events:** the same alternate-screen restore as step 8.
    **Status:** Fakes only
    Evidence: the scripted version of this is `terminal/pane/tests/pane.rs::a_full_screen_program_leaves_the_shell_as_it_was`. No real `less` is run.

## Branches

### Esc when an app overlay is open

- **When:** a sheet, such as the command palette, is open while the session's vim is still running.
- **I do:** press Esc.
- **I see:** the overlay closes. vim does not get the key.
- **State and events:** an open overlay takes focus (`ui/src/overlays.rs:188-220`), so the terminal's `key_down` never runs, and the global `Dismiss` binding closes the top overlay (`ui/src/keys.rs:98`).
- **Expected outcome:** the overlay closes, focus goes back to the terminal, and vim is untouched.
- **Status:** Code, no test
  Evidence: `ui/src/overlays.rs:188-220` moves and restores focus. `desktop/tests/machines_flow.rs::escape_and_cancel_close_the_dialog` checks that Escape closes the add-machine dialog, with the terminal not focused. No test opens an overlay while vim has focus.

### ⌘S in vim on the Mac

- **When:** the user presses ⌘S in vim, out of habit.
- **I do:** press ⌘S in insert or normal mode.
- **I see:** nothing happens. The buffer is not saved and vim does not beep.
- **State and events:** the terminal ignores any ⌘ key it does not own (`terminal/view/src/keys.rs:65`). The app has no binding for ⌘S: `ui/src/keys.rs` binds `shift-cmd-s` to a bug-report snapshot, and the Edit menu in `desktop/src/main.rs` has Undo, Copy, Paste and Select All, but no Save.
- **Expected outcome:** nothing is sent to vim and no app action runs. The user saves with `:w`.
- **Status:** Fakes only
  Evidence: `terminal/view/src/keys.rs::other_cmd_shortcuts_are_left_to_the_app` checks ⌘T and ⌘⇧C are ignored on the Mac. ⌘S is the same branch, but it is not in the test.

### vim over the SSH VM

- **When:** the session is a shell on the SSH VM, and the user runs vim there.
- **I do:** run `nvim terminal.rs` on the VM and repeat steps 1 to 8.
- **I see:** the same as on the Mac, with the same colors, keys and quit.
- **State and events:** the bytes come through the SSH holder. The alternate screen and colors are the VM's.
- **Expected outcome:** the same screen and the same shell screen after quit.
- **Status:** Code, no test
  Evidence: `terminal/lab/tests/interactive.rs:168-200` starts Neovim on the VM when `TERMINAL_LAB_SSH_CONFIG` is set, and then runs zsh commands. It asserts nothing about the Neovim tab. The other SSH targets check clipboard and resize, not vim.

### The app restarts while vim is open

- **When:** the app quits and starts again while vim is running in the session.
- **I do:** quit Humata 2 with vim open, wait, then start it and open the session.
- **I see:** vim is on screen again, with the same buffer and the same cursor. Its colors are there.
- **State and events:** the holder keeps vim's PTY. On attach, the host sends a checkpoint of the screen, which keeps the alternate screen, modes and history (`terminal/vt/tests/checkpoint.rs::checkpoint_keeps_history_alt_screen_modes_links_and_live_continuation`). The live output goes on after it.
- **Expected outcome:** vim redraws at its current screen, and typing resumes once the screen has arrived (see micro-16).
- **Status:** Fakes only
  Evidence: the checkpoint test covers the alternate screen with scripted bytes. `daemon/humatad/tests/terminal.rs::attaching_from_zero_again_replays_what_was_kept` (Works (real daemon)) replays plain shell output after a restart. `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` (Works (real daemon)) does the same for a shell with a background job. No test restarts with vim running.

### less at the end of the file

- **When:** the user presses `q` or `G` past the last line.
- **I do:** press `G`, then `j`.
- **I see:** `less` stays at the end and rings nothing.
- **State and events:** the keys are sent to `less`, which decides the result.
- **Expected outcome:** nothing changes on the app's side.
- **Status:** Code, no test

## Expected outcomes

- [ ] vim's colors come through as RGB cell colors.
- [ ] Typed text reaches vim, and Esc reaches vim while the terminal has focus.
- [ ] A window resize redraws vim, and the text stays.
- [ ] Esc with an overlay open closes the overlay and does not reach vim.
- [ ] ⌘S does not reach vim and runs no app action.
- [ ] After vim or less quits, the shell's screen is exactly as before the program started.
- [ ] A restart with vim open shows vim again, with its buffer.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. vim opens full screen | Works (real shell) | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (manual, macOS) |
| 2. Truecolor colors | Works (real shell) | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` |
| 3. Insert, type, Esc | Works (real shell) | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (text only) |
| 4. Esc reaches vim, not an app binding | Code, no test | `terminal/view/src/view.rs:851-931`, `ui/src/keys.rs:98` |
| 5. Resize redraws vim | Works (real shell) | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (text only) |
| 6. ⌃Z stops vim | Code, no test | `terminal/view/src/keys.rs` (no test for ⌃Z) |
| 7. fg brings vim back | Code, no test | none |
| 8. :wq and the shell screen | Fakes only | `terminal/pane/tests/pane.rs::a_full_screen_program_leaves_the_shell_as_it_was` |
| 9. less opens | Code, no test | none |
| 10. / search in less | Code, no test | none |
| 11. q and the shell screen | Fakes only | `terminal/pane/tests/pane.rs::a_full_screen_program_leaves_the_shell_as_it_was` |
| Branch: Esc with an overlay open | Code, no test | `ui/src/overlays.rs:188-220`; `desktop/tests/machines_flow.rs::escape_and_cancel_close_the_dialog` |
| Branch: ⌘S in vim | Fakes only | `terminal/view/src/keys.rs::other_cmd_shortcuts_are_left_to_the_app` (⌘T, not ⌘S) |
| Branch: vim over the SSH VM | Code, no test | `terminal/lab/tests/interactive.rs:168-200` (starts it, asserts nothing) |
| Branch: restart with vim open | Fakes only | `terminal/vt/tests/checkpoint.rs::checkpoint_keeps_history_alt_screen_modes_links_and_live_continuation` |
| Branch: less at the end | Code, no test | none |

## Gaps and likely bugs

- **Not built:** nothing in the repo runs `less`, `fg`, or ⌃Z in a real shell. The Neovim test is the only real full-screen program test, and it is manual and macOS only.
- **Not built:** the Neovim test writes with `:write`, not `:wq`, and does not check that the shell screen comes back after vim quits. Add a step for `:wq` and a comparison of the shell screen.
- **Not built:** the Neovim test does not check the mode line after Esc. It proves the text arrived, not that vim left insert mode.
- **Code, no test:** Esc with an overlay open while a terminal is behind it. The routing is by focus, and no test checks the terminal's focus after the overlay closes in a real session.
- **Code, no test:** Neovim over the SSH VM. `terminal/lab/tests/interactive.rs:168-200` starts Neovim on the VM and never checks its screen.
- **Code, no test:** the `less` case. The code comment in `terminal/pane/src/host.rs:168` says `less` sets mode 1007, which is not checked against a real `less`.
- **Likely bug:** none found in this flow. The ⌘S and Esc paths look right as read. Confirm the Esc path on a Mac with an overlay open, since GPUI's key order is outside the repo.
