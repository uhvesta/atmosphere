# IME, emoji, wide and right-to-left text reach the shell once and sit in the right cells

Micro. Platforms: macOS, with Linux noted where it differs (dead keys). Paths are relative to `apps/humata2`.

## Starting state

- Humata 2 is open on this Mac, with one session open. Its tab is a shell on this Mac.
- The shell prompt shows and the terminal has keyboard focus. No overlay is open.
- A Japanese input method is selected in the system. On the Linux machine, dead keys are set up for ´.
- The clipboard holds a family emoji (a ZWJ sequence) and a flag emoji, each on its own line.
- A second session runs a shell on `box-1`, for the disconnect branch.

## Steps

1. **I do:** with the Japanese input method on, type `nihongo` and do not press Enter.
   **I see:** `にほんご` appears at the cursor, underlined. Nothing reaches the shell. The candidate list opens under the cursor.
   **State and events:** the view holds the composing text as its preedit. The shell gets no bytes. The preedit is drawn as a background, the text and an underline at the cells' bottom edge.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` checks `preedit() == Some("に")` and that nothing was sent before the commit. `terminal/view/src/view.rs:1594-1607` (`replace_and_mark_text_in_range`) and `view.rs:1417-1432` (the preedit paint and its underline). The test drives the handler directly with `TestInput`, not an OS input method. The underline is not asserted anywhere.

2. **I do:** press Enter to commit the candidate `日本語`.
   **I see:** `日本語` appears once at the prompt. The underline goes. The shell does not run anything, because Enter was used to commit.
   **State and events:** `replace_text_in_range` clears the preedit and sends the committed text once as UTF-8 (`terminal/view/src/view.rs:1574-1592`, `pane/src/host.rs:350-358`). The shell gets `日本語` once.
   **Status:** Fakes only
   Evidence: `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` asserts the bytes sent are exactly `日本` after one commit and that `preedit()` is `None` afterwards. A shell echo is not checked, and no test uses an OS input method.

3. **I do:** the same Enter as in step 2, from the platform's side: the input method may deliver the key press before its commit.
   **I see:** no extra newline. The shell does not run a command for the Enter that committed the text.
   **State and events:** while a preedit exists, `key_down` returns before it encodes the key (`terminal/view/src/view.rs:858-861`), so a key delivered during composition is not sent to the program.
   **Status:** Code, no test
   Evidence: `terminal/view/src/view.rs:858-861`. No test sends a key while a preedit is set. The ordering of the platform's key and commit events is not known from the repo.

4. **I do:** press ⌘V with the family emoji on the clipboard.
   **I see:** the emoji appears as one glyph in two cells. The shell receives the whole sequence.
   **State and events:** the paste takes the clipboard text as UTF-8 and sends it, bracketed when the program asked (`terminal/pane/src/host.rs:426-436`). The grid stores the sequence as one cell with width Wide and a spacer after it.
   **Status:** Code, no test
   Evidence: the paste path takes any text. `terminal/lab/tests/clipboard_ssh.rs` (Works (real shell) over the SSH VM, manual target) pastes CJK, Greek and a combining mark, and checks exact UTF-8 bytes. No test pastes an emoji. The grid rule for a ZWJ sequence is covered only as fed bytes, in step 6.

5. **I do:** paste the flag emoji with ⌘V.
   **I see:** the flag appears as one glyph in two cells.
   **State and events:** one grapheme, width Wide.
   **Status:** Fakes only
   Evidence: `terminal/vt/tests/grid.rs::a_flag_is_one_grapheme`. The bytes are fed to the engine by the test, with no paste and no shell.

6. **I do:** in the shell, print one line made of `e` followed by U+0301, then `日本語`, then the family emoji (for example `printf 'é 日本語 👨‍👩‍👧\n'`).
   **I see:** `é` is one cell with its mark on the `e`. `日本語` takes two cells per character. The family emoji takes two cells. The prompt after the line starts at the next row.
   **State and events:** cell `é` with the cursor one column on. Each Wide character is followed by a spacer tail. The cursor is at column x for the text written, here after the last glyph.
   **Status:** Fakes only
   Evidence: `terminal/vt/tests/grid.rs::combining_marks_stay_with_their_base` (cell `e\u{301}`, cursor x 3), `grid.rs::cjk_characters_take_two_cells` (cursor x 4 after two Wide characters), `grid.rs::an_emoji_zwj_sequence_is_one_grapheme`. `terminal/pane/tests/pane.rs::tricky_output_lands_in_the_right_cells` feeds a line of Wide, emoji and combining text, and checks the bar lands on column 19. All are real `libghostty-vt` engine tests with scripted bytes, no shell, in a headless window.

7. **I do:** type Persian text, for example `سلام`, with the Persian keyboard layout.
   **I see:** the letters appear in the order typed, left to right in the cells. They do not join into connected forms. Nothing is reordered.
   **State and events:** each letter is one cell. The view draws right-to-left letters on their own, so the shaper cannot join or reorder them (`terminal/view/src/frame.rs:315-330`, `frame.rs:336-342`). The commit path is the same as in step 2.
   **Status:** Fakes only
   Evidence: `terminal/view/src/frame.rs::clusters_and_right_to_left_letters_stand_alone` checks the draw plan for Hebrew letters as separate pieces. It is pure data, with no font and no window. The typed Persian text itself goes through the commit path proven in step 2 by fakes only.

8. **I do:** on the Linux machine, press ´ and then e.
   **I see:** `é` appears once. No stray ´ and no stray e.
   **State and events:** the platform sends the composed `é` once through `replace_text_in_range`. The view sends it once as text (`terminal/view/src/view.rs:1574-1592`).
   **Status:** Code, no test
   Evidence: the commit path in `view.rs:1574-1592`. No test has a dead key. `grep` for "dead key" finds only a comment in `terminal/view/src/host.rs:116-117`. What the Linux platform delivers for a dead key is not in the repo.

9. **I do:** drag across `日本` in the shell output and press ⌘C, then paste into a text field elsewhere.
   **I see:** `日本` is copied, with no gap and no duplicated character.
   **State and events:** the selection covers the Wide cells. The copy gives the cell text, not the spacer.
   **Status:** Fakes only
   Evidence: `terminal/vt/tests/selection.rs::a_viewport_selection_copies_wide_characters_across_a_wrap` copies `日本語ab` across a wrap. The SSH VM test `clipboard_ssh.rs` (Works (real shell), manual) copies CJK and a combining mark over SSH and pastes them back exactly.

10. **I do:** drag across the family emoji and press ⌘C.
    **I see:** the whole sequence is copied, not one of its parts.
    **State and events:** the selected cell's text is the whole grapheme.
    **Status:** Code, no test
    Evidence: the grapheme rule is in the engine (`vt/tests/grid.rs::an_emoji_zwj_sequence_is_one_grapheme`). No test copies an emoji selection.

## Branches

### The link to box-1 drops while a composition is on screen

- **When:** the session is on `box-1`, and the user is composing `にほ` when the link drops.
- **I do:** press Enter to commit, then wait for the link to return and type `x`.
- **I see:** the banner says the terminal is disconnected. The composition stays under the cursor until Enter. After Enter, nothing reaches `box-1`. When the link returns, `x` is typed, and the Japanese text is not typed later.
- **Expected outcome:** the composition is either cleared at the drop, or refused at the start. What is typed while offline never arrives late (SE-16, and `pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so`).
- **Status:** Fakes only. The composition is cleared when the link drops, and committing it anyway sends nothing and tells the app once, so a toast says it was not sent (`terminal/pane/tests/pane.rs::a_composition_ends_when_the_link_drops`). What is typed offline never arrives late (`pane.rs::while_disconnected_input_is_dropped_and_the_pane_says_so`).

### An IME candidate list opens under the cursor

- **When:** the user has scrolled back 20 rows and then starts composing.
- **I do:** scroll up 20 rows with the wheel, then type `nihon` and wait for the candidate list.
- **I see:** the candidate list should open at the composing position, on screen.
- **Expected outcome:** the list opens at the cursor cell, which is on screen when composing.
- **Status:** Fakes only. Starting a composition while scrolled back brings the view to the live cursor, where the candidate list opens (`terminal/pane/tests/pane.rs::composing_while_scrolled_back_returns_to_the_cursor`). No test opens a real input method.

### A right-to-left line is shown in logical order

- **When:** the user types Persian, Hebrew or Arabic text and then types more letters after it.
- **I do:** type `سلام دنیا`.
- **I see:** the letters appear in typed order from left to right, each on its own. No bidi reordering happens, and the letters do not join. This is what the emulator does.
- **Expected outcome:** the brief expects logical order and no bidi. The view draws letters one cell at a time.
- **Status:** Not built (no milestone)

### Dead keys on Linux

- **When:** the Linux machine's dead key is followed by a key that does not combine, such as space.
- **I do:** press ´ and then space.
- **I see:** a single ´ appears, and the space is typed after it.
- **Expected outcome:** the platform delivers the dead key's mark, and the view sends it once as text.
- **Status:** Code, no test

### Esc while composing

- **When:** the user presses Esc while a composition is on screen.
- **I do:** type `ni` and press Esc.
- **I see:** the composition is cancelled by the input method. Nothing is sent to the shell, and no overlay closes.
- **Expected outcome:** the input method takes the Esc (`terminal/view/src/view.rs:858-861`). If an overlay is open, the view is not focused and the overlay gets the key instead.
- **Status:** Code, no test

## Expected outcomes

- [ ] A composing string is drawn at the cursor and is not sent to the shell.
- [ ] A commit sends its UTF-8 text once, and clears the composition.
- [ ] A key delivered during a composition is not sent to the program.
- [ ] A family emoji and a flag each take one grapheme and two cells.
- [ ] A combining mark stays in the same cell as its base, and the cursor moves one cell.
- [ ] A Wide character takes two cells, and the cursor lands after both.
- [ ] A copy of a wide or combining selection gives the characters, with no spacer and no gap.
- [ ] A right-to-left letter is drawn alone, in typed order.
- [ ] A dead key followed by a letter produces the composed letter once.
- [ ] Composition while disconnected is cleared or refused, and nothing is sent late.
- [ ] The candidate list opens on screen.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Preedit shows, not sent | Fakes only | `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` |
| 2. Commit sends UTF-8 once | Fakes only | `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` |
| 3. Key during composition is not sent | Code, no test | `terminal/view/src/view.rs:858-861` |
| 4. Paste family emoji | Code, no test | `terminal/pane/src/host.rs:426-436`; `terminal/lab/tests/clipboard_ssh.rs` covers other Unicode |
| 5. Paste flag | Fakes only | `terminal/vt/tests/grid.rs::a_flag_is_one_grapheme` |
| 6. Wide, combining and emoji cells, cursor | Fakes only | `terminal/vt/tests/grid.rs::cjk_characters_take_two_cells`, `combining_marks_stay_with_their_base`; `terminal/pane/tests/pane.rs::tricky_output_lands_in_the_right_cells` |
| 7. Persian typed, no joining | Fakes only | `terminal/view/src/frame.rs::clusters_and_right_to_left_letters_stand_alone` |
| 8. Dead key ´ then e | Code, no test | `terminal/view/src/view.rs:1574-1592` |
| 9. Copy Wide characters | Fakes only | `terminal/vt/tests/selection.rs::a_viewport_selection_copies_wide_characters_across_a_wrap`; `terminal/lab/tests/clipboard_ssh.rs` (Works (real shell), manual) |
| 10. Copy an emoji selection | Code, no test | `terminal/vt/tests/grid.rs::an_emoji_zwj_sequence_is_one_grapheme` (cells only) |
| Branch: composing while disconnected | Fakes only | `terminal/pane/tests/pane.rs::a_composition_ends_when_the_link_drops` |
| Branch: candidate list position | Fakes only | `terminal/pane/tests/pane.rs::composing_while_scrolled_back_returns_to_the_cursor` |
| Branch: right-to-left logical order | Not built | no milestone (no bidi in `terminal/`) |
| Branch: dead keys on Linux | Code, no test | `terminal/view/src/view.rs:1574-1592` |
| Branch: Esc while composing | Code, no test | `terminal/view/src/view.rs:858-861` |

## Gaps and likely bugs

- **Not built:** real-window IME. OVERVIEW.md, Terminal section (I6): "Test IME in a real window" is open. No milestone number.
- **Not built:** bidi reordering and letter joining for right-to-left text. No milestone. The brief expects none.
- **Not built:** no test presses a dead key or uses an OS input method on Linux. Confirm on a Linux machine with a dead-key layout.
- **Fixed:** a composition ends when the link drops (`terminal/pane/tests/pane.rs::a_composition_ends_when_the_link_drops`, Fakes only).
- **Fixed:** composing while scrolled back returns the view to the cursor, where the candidate list opens (`terminal/pane/tests/pane.rs::composing_while_scrolled_back_returns_to_the_cursor`, Fakes only).
- **Code, no test:** `deliver_input` always sends `composing: false` to the key encoder (`terminal/view/src/view.rs:799`). It is harmless while `key_down` drops keys during composition, but no test covers it.
- **Code, no test:** emoji paste and emoji copy are not tested in any file. The engine tests cover the cells, not the clipboard.
