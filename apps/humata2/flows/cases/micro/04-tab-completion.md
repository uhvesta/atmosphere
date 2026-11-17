# Tab and Shift-Tab reach the shell, and the terminal keeps focus

Layer: micro. Platforms: macOS and Linux (zsh and bash), and a remote shell over SSH.

## Starting state

- Humata 2 is open on this Mac, with a session that has a shell tab.
- The shell tab has focus. The shell is zsh, in a folder that holds `src/components/` and `scripts/`. Inside `src/` there is only `components/`.
- The prompt line is empty. The palette is closed. The view is at the newest output.
- A second shell tab is open on `box-1` over SSH, for the remote branch.
- Nothing is held.

## Steps

1. **I do:** Type `cd src/comp`, then press Tab.

   **I see:** The line reads `cd src/components/`. zsh adds the trailing slash.

   **State and events:** Tab goes to the program as a tab byte (0x09). zsh does the completion. The app does no completion of its own.

   **Status:** Fakes only. `terminal/client/tests/replica.rs::focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal` checks that the tab byte reaches the fake socket. `terminal/vt/tests/input.rs::tab_and_shift_tab_send_terminal_navigation_keys_without_text` checks the encoder. No test runs zsh's completion.

2. **I do:** Press ⌃U to clear the line, type `ls s`, then press Tab, and then Tab again.

   **I see:** zsh lists the matches, `scripts/` and `src/`, under the prompt. The line still reads `ls s`. Whether the list shows after one Tab or two depends on zsh's options.

   **State and events:** Two tab bytes go to zsh. zsh prints the list, and the view draws it as output.

   **Status:** Fakes only. The tab bytes are proven by the test in step 1. The list itself is zsh's output and is not run in a test.

3. **I do:** Type `ls zzz`, then press Tab.

   **I see:** Nothing completes. zsh rings the bell.

   **State and events:** zsh sends BEL (0x07). The VT engine raises a bell event, which `terminal/pane/src/host.rs:137` turns into `PaneEvent::Bell`. No code in `ui/src/` or `terminal/client/src/` handles `PaneEvent::Bell`, so nothing is shown or played.

   **Status:** Not built (no milestone) for a visible or audible bell. The event itself is Fakes only: `terminal/pane/tests/pane.rs::titles_bells_and_clipboard_writes_are_told_to_the_app` sends a BEL and checks that the app is told.

4. **I do:** With zsh's completion menu showing, press Shift-Tab.

   **I see:** zsh moves the highlight back one item.

   **State and events:** Shift-Tab goes to zsh as `ESC [ Z` (CSI Z). The key has no text, and the Shift is kept in the modifiers.

   **Status:** Fakes only. `terminal/vt/tests/input.rs::tab_and_shift_tab_send_terminal_navigation_keys_without_text` checks the bytes. The replica test in step 1 checks `\t\x1b[Z` on the fake socket. The menu moving is zsh's and is not run.

5. **I do:** Press Tab, and then type `x`.

   **I see:** The `x` lands in the shell line. Focus stays in the terminal, and gpui's own focus traversal does not run.

   **State and events:** `terminal/view/src/view.rs:292-293` binds `tab` and `shift-tab` to `NoAction` in the terminal's key context. The key handler returns after the key is sent, so the event does not travel on to focus traversal.

   **Status:** Fakes only. `terminal/client/tests/replica.rs::focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal` asserts that focus stays on the pane after Tab and Shift-Tab.

6. **I do:** On the `box-1` shell over SSH, on a slow link, press Tab on a prefix with several matches.

   **I see:** The list arrives after a delay, and the line updates when the remote zsh replies. No tab is sent twice.

   **State and events:** The tab byte travels the same input path as typing, over the link to `box-1`.

   **Status:** Code, no test. No SSH lab test presses Tab. `terminal/lab/tests/app.rs`, `clipboard_ssh.rs` and `vertical_resize_ssh.rs` press Enter, Cmd+V and Cmd+C only.

## Branches

### Tab while the command palette is open

- **When:** the palette is open (⌘K), and the focus is in its search field.
- **I do:** Press ⌘K, then Tab.
- **I see:** Focus stays in the palette. Nothing reaches the shell.
- **Expected:** The palette takes the Tab, and the shell line does not change.
- **Status:** Code, no test for Tab. The rule that an unfocused terminal takes no keys is tested for letters and Down (`desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes`), not for Tab.

### Tab in an unfocused terminal

- **When:** I click outside the terminal, then press Tab.
- **I do:** Click in the sidebar, then press Tab.
- **I see:** Nothing changes in the shell.
- **Expected:** An unfocused terminal sends nothing. `terminal/view/src/view.rs:851-856` counts the key and returns.
- **Status:** Fakes only. `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` checks the same rule with letters.

### Tab in the find bar

- **When:** the find bar is open (⌘F) on a shell tab, and the focus is in the find field.
- **I do:** Press Tab.
- **I see:** Nothing reaches the shell.
- **Expected:** The find field takes the Tab. No input goes to the program.
- **Status:** Fakes only. The second half of `terminal/client/tests/replica.rs::focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal` checks that Tab in the find field sends no input.

### Completion output while I have scrolled up

- **When:** I have scrolled up 20 lines with the wheel, and the shell prints a completion list.
- **I do:** Scroll up, then, with the output still arriving, press Tab twice on an ambiguous prefix.
- **I see:** While output arrives without a key, the view stays where I scrolled. The Tab key itself moves the view to the newest line.
- **Expected per the brief:** The view stays where I scrolled.
- **Actual:** `terminal/pane/src/host.rs:271` calls `follow_output` on every key sent, so a keystroke snaps to the bottom. The comment at `host.rs:155-156` says this is meant to match typing in the engine.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams` covers the output case. The keystroke snap is Code, no test. See Gaps.

### Link drops during Tab

- **When:** the link to `box-1` drops right after I press Tab.
- **I do:** Press Tab on the remote shell, then wait with the link down.
- **I see:** The pane shows its disconnected banner. The Tab is not sent again when the link returns.
- **Expected:** Input is refused while disconnected, and nothing is replayed.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` checks the refusal. The real SSH drop is not tested.

## Expected outcomes

- [ ] Tab sends one tab byte, and Shift-Tab sends `ESC [ Z`.
- [ ] A Tab with one match completes in the shell.
- [ ] A Tab with several matches lists them in the shell.
- [ ] A Tab with no match rings the shell bell, and the app is told about it.
- [ ] Tab never moves focus out of the terminal.
- [ ] Tab in the find bar does not reach the shell.
- [ ] Tab in an unfocused terminal sends nothing.
- [ ] Tab over SSH completes at the remote shell, after the link delivers the reply.
- [ ] A keystroke while scrolled up returns the view to the newest line (confirm this is wanted).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Single match completes | Fakes only | `terminal/client/tests/replica.rs::focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal` |
| 2 Double Tab lists | Fakes only | Tab bytes as step 1; list not run |
| 3 No match, bell | Not built (no milestone) | `terminal/pane/src/host.rs:137`, no UI handler |
| 4 Shift-Tab in menu | Fakes only | `terminal/vt/tests/input.rs::tab_and_shift_tab_send_terminal_navigation_keys_without_text` |
| 5 Tab keeps focus | Fakes only | `terminal/client/tests/replica.rs::focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal` |
| 6 Tab over SSH | Code, no test | `terminal/lab/tests/*ssh*.rs` press no Tab |
| Branch: palette open | Code, no test | `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` (letters only) |
| Branch: unfocused terminal | Fakes only | `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` |
| Branch: find bar | Fakes only | Second half of the replica test in step 5 |
| Branch: scrolled up | Fakes only | `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams`; keystroke snap Code, no test |
| Branch: link drops | Fakes only | `terminal/pane/tests/pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so` |

## Gaps and likely bugs

- Not built (no milestone): a visible or audible bell. `PaneEvent::Bell` is raised (`terminal/pane/src/host.rs:137`), but no code under `ui/src/` or `terminal/client/src/` matches it. A failed completion makes no sound and no visible mark.
- Likely bug: none found in the Tab path. Tab, Shift-Tab and the focus rule are handled in `terminal/view/src/keys.rs` and `terminal/view/src/view.rs`, and their bytes are proven on fakes.
- Decision to confirm: a keystroke while scrolled up moves the view to the newest line (`terminal/pane/src/host.rs:157-163` and `:271`). The brief expected the view to hold. The code and its comment say the snap is intended. Decide which is wanted, then add a pane test for whichever is chosen.
- Unproven: no test presses Tab in a real shell or over SSH. `terminal/lab/tests/` contains no Tab press, so zsh and bash completion, and the menu, are not proven live.
