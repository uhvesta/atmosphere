# Scrolling holds the view still, and find steps through matches and closes with Esc

Layer: micro. Platforms: macOS, Linux.

## Starting state

- Humata 2 is open on this Mac, with a shell tab in the session `ledger`.
- A build is running in that shell. It prints about 20,000 lines in total, and it is still printing.
- The view is at the newest output, so the scrollbar is hidden.
- The find bar is closed. Nothing is selected.
- The tab keeps its default history: up to 10,000 lines (`terminal/wire/src/lib.rs:39-46`, `terminal/host/src/lib.rs:2183-2195`).

## Steps

1. **I do:** I scroll up with the wheel or trackpad by about 30 rows.
   **I see:** The view moves up into older output. A scrollbar appears at the right edge.
   **State and events:** The view asks the emulator for older rows. Nothing is sent to the build.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams` checks that scrolling sends nothing. The emulator is real, but the PTY is a fake.

2. **I do:** I press ⌘PgUp, then ⌘Home, then ⌘End. On Linux I press ⇧PgUp, ⇧Home and ⇧End.
   **I see:** ⌘PgUp moves up one screen. ⌘Home goes to the oldest kept row. ⌘End goes back to the newest output, and the scrollbar hides.
   **State and events:** The keys become scroll requests (`terminal/view/src/keys.rs:61`, `75-76`, `117-125`). The build gets no bytes.
   **Status:** Works (real shell) on macOS. `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` presses `cmd-pageup`, `cmd-home` and `cmd-end` in a real zsh and checks the scrollbar offset (lines 478-509). It is a manual target that needs the owner's zsh home. Linux keys are Fakes only: `terminal/view/tests/view.rs::history_keys_and_the_scrollbar_move_through_the_history` uses `shift-pageup` with a fake grid. ⌘PgDn is not pressed in any real-shell test.

3. **I do:** I stay scrolled up and watch for ten seconds while the build keeps printing.
   **I see:** The text under the view does not move. The scrollbar thumb stays in place, and the scrollbar track grows, because the history is longer.
   **State and events:** New output is appended to the history. The view's offset does not change. The view follows the output again only when I type or press ⌘End.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams` checks that the offset is unchanged, that the total grows, and that typing returns to the bottom. The output is fed in-process, not from a real build.

4. **I do:** I press ⌘F and type `error`.
   **I see:** A find bar appears at the top right, with the input focused. The newest match is highlighted and the bar reads "N of M", where N is the current match. Typing does not reach the build.
   **State and events:** Find opens and searches every row of history and the screen. The newest match is current. The terminal does not have focus while the find input has it.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back` checks the match count, the current index and the keyboard focus. It does not check the label text in `terminal/pane/src/pane.rs:640-649`. `desktop/tests/native/terminal.rs::the_embedded_terminal_accepts_keys_and_keeps_its_scrollback` opens the bar with ⌘F in the app, with a fake daemon.

5. **I do:** I press ⌘G, then ⇧⌘G. Then I press Enter, then ⇧Enter.
   **I see:** ⌘G moves to an older match. ⇧⌘G moves to a newer one. Enter and ⇧Enter do the same as the two keys. The "N of M" count changes with each move.
   **State and events:** ⌘G calls `find_older` and ⇧⌘G calls `find_newer` (`terminal/pane/src/pane.rs:612-625`). The current match changes.
   **Status:** Code, no test. The ⌘G and ⇧⌘G key branch in `terminal/pane/src/pane.rs:612-625` is not pressed by any test. Enter and ⇧Enter are tested in `terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back`.

6. **I do:** I turn on the regular expression button (`.*`) and type `(\d+`.
   **I see:** The bar shows an error in the danger colour instead of a count. The error says the pattern is not valid, and says to fix it or turn Regex off. Nothing is highlighted.
   **State and events:** The query is refused and no matches are set. The error comes from `FindError::Pattern` (`terminal/pane/src/error.rs`).
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::a_broken_regular_expression_says_so_and_finds_nothing` checks that an error is set and that no matches are found. It does not check the message text. The message itself follows the rule in AGENTS.md: it says what failed and how to fix it.

7. **I do:** I press Esc.
   **I see:** The find bar closes. The highlights disappear. The terminal has focus again, so the next key goes to the build.
   **State and events:** `close_find` removes the marks and gives focus back to the terminal (`terminal/pane/src/pane.rs:407-414`).
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back` checks the bar closes, the marks clear and the terminal gets the next key.

8. **I do:** I press ⌘K, expecting the screen to clear.
   **I see:** The command palette opens. The screen does not clear, and the build gets nothing.
   **State and events:** ⌘K is bound to `OpenPalette` (`ui/src/keys.rs:94`). The terminal ignores ⌘ keys it does not use (`terminal/view/src/keys.rs:53-61`), so the app receives the key. This is a deliberate choice: `SPEC.md` KP-6 says ⌘K opens the palette everywhere, including inside a terminal tab.
   **Status:** Code, no test. No test presses ⌘K inside a terminal. `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` opens the palette by app action, not by the key.

## Branches

### Find in an agent terminal

- **When:** The tab is an agent's terminal, such as an agent TUI, not a plain shell.
- **I do:** I press ⌘F and type `error`.
- **I see:** The find bar opens and counts matches in the agent's output, the same as in a shell tab.
- **State and events:** Agent terminals use the same pane as shell tabs. The find code has no agent check (`terminal/pane/src/pane.rs:596-625`).
- **Expected outcome:** Find works the same in an agent tab as in a shell tab.
- **Status:** Code, no test. The only ⌘F test in the app uses a shell tab (`desktop/tests/native/terminal.rs:17-50`). No test uses an agent tab.

### History after the app or daemon restarts

- **When:** I quit Humata 2 and reopen it while the build's tab has history.
- **I do:** I quit, reopen the app, wait for the tab to attach, and press ⌘PgUp.
- **I see:** The screen shows the same last output. ⌘PgUp reaches the older output that was kept, not only the visible screen.
- **State and events:** The daemon keeps the tab's history in a checkpoint and replays what it kept on attach (`terminal/host/src/lib.rs:584-595`).
- **Expected outcome:** The visible screen and the kept history come back after a restart.
- **Status:** Fakes only. `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` restarts a real daemon, but it checks only that "hello" is shown after the restart. It does not check history above the screen. `terminal/vt/tests/checkpoint.rs::checkpoint_keeps_history_alt_screen_modes_links_and_live_continuation` checks the history round trip inside the emulator, with no daemon.

### Output past the scrollback limit

- **When:** The build prints more lines than the tab keeps. The default is 10,000 lines.
- **I do:** I scroll to the top of the history and search for the first line the build printed.
- **I see:** The oldest lines are gone. The search for the first line finds nothing. The newest 10,000 lines or so are still there.
- **State and events:** The emulator drops the oldest history, a page at a time, so it can keep somewhat more than the limit (`terminal/vt/src/terminal.rs:177-179`). The host applies the limit with `set_scrollback_limits` (`terminal/host/src/lib.rs:1713`, `2183-2195`).
- **Expected outcome:** The newest history is kept, the oldest is dropped, and the view still scrolls and finds in what is kept.
- **Status:** Fakes only. `terminal/vt/tests/checkpoint.rs::reducing_history_limits_reclaims_existing_history` and `terminal/vt/tests/checkpoint.rs::bounded_restoration_keeps_ten_thousand_history_lines_and_accepts_live_output` test the limit inside the emulator. No test shows the limit through the daemon or the view. No UI says that history was cut.

### Scrolling with ⌘PgUp inside a full-screen program

- **When:** A full-screen program such as `htop` or `vim` is open on the alternate screen. The alternate screen has no history.
- **I do:** I press ⌘PgUp.
- **I see:** Nothing changes on the screen. The program does not react to the key.
- **State and events:** The key becomes a scroll request even on the alternate screen (`terminal/view/src/keys.rs:61`). The pane calls `scroll_viewport`, which has no history to move on the alternate screen (`terminal/pane/src/host.rs:336-349`). The program gets no `PageUp`.
- **Expected outcome:** If history keys are meant to be consumed here, the person sees why nothing happened. If they are meant to reach the program, `PageUp` arrives at it.
- **Status:** Fakes only. On the alternate screen the history keys are the program's: ⌘PgUp (⇧PgUp off a Mac) sends PageUp to it, and back at the shell they scroll the history and send nothing (`terminal/pane/tests/pane.rs::history_keys_reach_a_full_screen_program`).

## Expected outcomes

- [ ] Wheel and ⌘PgUp, ⌘PgDn, ⌘Home and ⌘End move through history and send no bytes to the program.
- [ ] While I am scrolled up, new output does not move the view.
- [ ] Typing returns the view to the newest output.
- [ ] ⌘F opens the find bar with focus in its input. The count reads "N of M".
- [ ] ⌘G and ⇧⌘G move to older and newer matches.
- [ ] A broken pattern shows an error that says how to fix it, and finds nothing.
- [ ] Esc closes the find bar, clears the highlights and returns focus to the terminal.
- [ ] ⌘K opens the command palette and sends nothing to the program.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Wheel scroll | Fakes only | `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams` |
| 2. ⌘PgUp, ⌘Home, ⌘End | Works (real shell) | `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines` (macOS, manual) |
| 3. View holds while output streams | Fakes only | `terminal/pane/tests/pane.rs::the_view_stays_where_the_user_scrolled_while_output_streams` |
| 4. ⌘F opens find, "N of M" | Fakes only | `terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back` |
| 5. ⌘G and ⇧⌘G | Code, no test | `terminal/pane/src/pane.rs:612-625` |
| 6. Broken regex shows an error | Fakes only | `terminal/pane/tests/pane.rs::a_broken_regular_expression_says_so_and_finds_nothing` |
| 7. Esc closes find | Fakes only | `terminal/pane/tests/pane.rs::the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back` |
| 8. ⌘K opens the palette | Code, no test | `ui/src/keys.rs:94`, `SPEC.md` KP-6 |
| Branch: find in an agent terminal | Code, no test | `terminal/pane/src/pane.rs:596-625` |
| Branch: history after restart | Fakes only | `daemon/humatad/tests/terminal.rs::a_tab_outlives_its_daemon` (visible text only) |
| Branch: output past the limit | Fakes only | `terminal/vt/tests/checkpoint.rs::reducing_history_limits_reclaims_existing_history` |
| Branch: ⌘PgUp in a full-screen program | Fakes only | `terminal/pane/tests/pane.rs::history_keys_reach_a_full_screen_program` |

## Gaps and likely bugs

- Not built: a clear-screen command. Humata has none. A person can run the shell's `clear` or press ⌃L, which sends Ctrl-L to the program. No milestone covers a separate clear. ⌘K stays the palette (`SPEC.md` KP-6).
- Not built: a notice when old output is dropped at the history limit. I found no UI text for it and no milestone for it.
- Likely bug: `terminal/lab/tests/interactive.rs:476` asserts at least 12,000 rows are kept, but the default live cap is 10,000 (`terminal/host/src/lib.rs:2183-2195`, `terminal/wire/src/lib.rs:42`). Either the engine keeps more by page (`terminal/vt/src/terminal.rs:177-179` says it may), or that test uses another policy. Confirm by running the manual test and printing `scrollbar.total` after the 12,000 lines.
- Fixed: on the alternate screen the history keys reach the program (`terminal/pane/tests/pane.rs::history_keys_reach_a_full_screen_program`, Fakes only).
- Test gap: `terminal/pane/src/pane.rs:640-649` shows "N+" when the match count reaches `MATCH_LIMIT` (10,000, `terminal/pane/src/find.rs:30`). No test covers that label.
