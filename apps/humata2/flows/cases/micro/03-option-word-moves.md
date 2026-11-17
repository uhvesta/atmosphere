# Option moves and deletes by word only when Option acts as Alt, which no setting allows

Layer: micro. Platforms: macOS (Option keys), with Linux (Alt) for comparison.

## Starting state

- Humata 2 is open on this Mac, with a session that has a shell tab.
- The shell tab has focus. Three shells are in play: plain zsh, plain bash on the Linux machine, and a zsh with custom key bindings.
- The prompt line holds `git log --oneline main`, with the cursor at the end.
- No palette or find bar is open. Nothing is held.
- Settings has no Option-as-Alt switch today (see step 7).

## Steps

1. **I do:** Press ⌥← with the cursor at the end of the line.

   **I see:** Expected: the cursor moves back one word, before `main`. Not checked by any test.

   **State and events:** `terminal/view/src/keys.rs:81-113` turns the key into Left with Alt and no text. The bytes come from the encoder behind `terminal/vt`, and its source is not read here. The usual form is `ESC [ 1 ; 3 D`. Plain zsh and bash do not bind that sequence by default, so the move may not happen.

   **Status:** Code, no test. No test sends Alt+Left.

2. **I do:** Press ⌥→.

   **I see:** Expected: the cursor moves forward one word. Not checked.

   **State and events:** Same path as step 1, with Right. The usual form is `ESC [ 1 ; 3 C`.

   **Status:** Code, no test.

3. **I do:** Press ⌥B.

   **I see:** A `∫` is typed at the cursor. No word move.

   **State and events:** Option-as-Alt defaults to Off (`terminal/vt/src/terminal.rs:223`). `terminal/pane/src/host.rs:208-224` returns nothing for the alt text when the setting is Off, so the key's own text, `∫`, goes to the program as UTF-8.

   **Status:** Fakes only. `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` proves the default on a recording PTY with ⌥F and `ƒ`. The `∫` itself is not asserted. See the Likely bug under Gaps.

4. **I do:** Press ⌥F.

   **I see:** `ƒ` is typed at the cursor.

   **State and events:** Same path as step 3.

   **Status:** Fakes only. The first half of `option_types_its_character_unless_it_is_set_to_act_as_alt` asserts `ƒ`.

5. **I do:** Press ⌥D.

   **I see:** `∂` is typed at the cursor.

   **State and events:** Same path as step 3.

   **Status:** Fakes only. Same test as step 4, with the path proven for ƒ. `∂` itself is not asserted.

6. **I do:** Press ⌥⌫.

   **I see:** Expected: the word before the cursor is deleted, as zsh does for ESC DEL. Not checked.

   **State and events:** The key is Backspace with Alt and no text. With the setting Off the encoder decides the bytes. The expected bytes are ESC then DEL (0x1b 0x7f). `keys.rs` has no special case for Alt+Backspace.

   **Status:** Code, no test.

7. **I do:** Look for an Option-as-Alt switch in Settings.

   **I see:** No such switch.

   **State and events:** `set_option_as_alt` (`terminal/vt/src/terminal.rs:774`) is called only from tests: `terminal/vt/tests/input.rs:73`, `terminal/vt/tests/view_support.rs:131`, `terminal/vt/tests/checkpoint.rs:113` and `terminal/pane/tests/pane.rs:68`. D73 in `DECISIONS.md` says "the terminal's `OptionAsAlt` setting decides", but no UI sets or stores it. A search of `ui/`, `features/`, `clients/` and `desktop/` found nothing.

   **Status:** Not built (no milestone). D73 names the setting; no milestone or requirement ID owns the switch.

8. **I do:** In a plain bash shell, press ⌥← and ⌥B.

   **I see:** The same as steps 1 and 3 with bash: ⌥← does nothing visible, and ⌥B types `∫`.

   **State and events:** Same encoder path, with Option off.

   **Status:** Code, no test for bash. ⌥B typing is covered by step 3 on fakes.

## Branches

### Option acts as Alt (test-only today)

- **When:** the setting is On. Nothing in the app can set it today, so this branch is what a switch would need to produce.
- **I do:** Press ⌥B.
- **I see:** Expected: ESC then `b`. zsh and bash move one word back.
- **Expected:** The program gets ESC then the letter, and no symbol is typed.
- **Status:** Fakes only. `terminal/vt/tests/input.rs::alt_and_a_letter_sends_escape_first_when_option_is_alt` checks the bytes. The second half of `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` checks that ⌥F sends `ESC f` when On. A user cannot reach this state, so the switch itself is Not built.

### I type a symbol on purpose

- **When:** I want `∫` in a command.
- **I do:** Press ⌥B, as in step 3.
- **I see:** `∫` is typed.
- **Expected:** With the setting Off, the symbol still types.
- **Status:** Fakes only. The `ƒ` assertion in `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` shows the Off path for an Option letter.

### Option+E, then E, types é

- **When:** I type a letter with a dead key.
- **I do:** Press ⌥E, release, then press E.
- **I see:** `é`.
- **Expected:** The input method composes the letter and commits it.
- **Status:** Fakes only. `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` commits text on fakes. The dead key on a real Mac is not tested, and MAP case `micro/13` notes that IME is untested in a real window. While a preedit is open, `terminal/view/src/view.rs` `key_down` returns early so the input method keeps the key.

### Plain bash on Linux, Alt+B

- **When:** Alt+B on the Linux machine.
- **I do:** Press Alt+B at a bash prompt.
- **I see:** Expected: the cursor moves back one word.
- **Expected:** bash receives ESC then `b`.
- **Status:** Code, no test. On the Linux branch `terminal/view/src/keys.rs:103` does not add Alt to the consumed modifiers (that line is Mac-only), so the encoder sees the letter with Alt. The bytes are unproven.

### A zsh with custom key bindings

- **When:** the shell binds word moves to `ESC b`, `ESC f` or the `ESC [ 1 ; 3 D` form.
- **I do:** Press ⌥← or ⌥B.
- **I see:** Expected: the word move. Actual: the result depends on whether Option acts as Alt, which the user cannot set (step 7).
- **Status:** Not built (no milestone) for the switch. The bytes for ⌥← are Code, no test.

## Expected outcomes

- [ ] ⌥← and ⌥→ move the cursor by a word in zsh and bash, or the case records why they do not.
- [ ] ⌥B and ⌥F move by a word where the shell binds them, when Option acts as Alt.
- [ ] A user can turn Option-as-Alt on and off in Settings.
- [ ] With the setting off, ⌥B, ⌥F and ⌥D type `∫`, `ƒ` and `∂`.
- [ ] ⌥⌫ deletes the previous word where the shell binds it.
- [ ] Option+E then E types `é`.

The Settings switch and the word moves fail today.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 ⌥← | Code, no test | `terminal/view/src/keys.rs:81-113` |
| 2 ⌥→ | Code, no test | Same path |
| 3 ⌥B types `∫` | Fakes only | `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` |
| 4 ⌥F types `ƒ` | Fakes only | Same test |
| 5 ⌥D types `∂` | Fakes only | Same test, `∂` not asserted |
| 6 ⌥⌫ | Code, no test | No Alt+Backspace case in `keys.rs` |
| 7 Settings switch | Not built (no milestone) | `terminal/vt/src/terminal.rs:774` is test-only |
| 8 Plain bash | Code, no test | Same encoder path |
| Branch: Option acts as Alt | Fakes only | `terminal/vt/tests/input.rs::alt_and_a_letter_sends_escape_first_when_option_is_alt` |
| Branch: symbol on purpose | Fakes only | Same pane test |
| Branch: Option+E, E | Fakes only | `terminal/pane/tests/pane.rs::an_input_method_commits_its_text_to_the_program` |
| Branch: Linux Alt+B | Code, no test | `terminal/view/src/keys.rs:103` is Mac-only |
| Branch: custom zsh bindings | Not built (no milestone) | Switch missing; bytes Code, no test |

## Gaps and likely bugs

- Not built (no milestone): the Option-as-Alt switch in Settings. D73 says the setting decides, but nothing in production code sets it, and no requirement ID or milestone owns it. `workflow/milestones/TERM.md` does not name it.
- Likely bug: `terminal/vt/src/terminal.rs:223` sets `OptionAsAlt::Off` by default, and no production path changes it (`set_option_as_alt` at line 774 is test-only). A Mac user therefore cannot use Option for word moves or deletes. ⌥B, ⌥F and ⌥D type `∫`, `ƒ` and `∂`. Confirm on this Mac: open a shell tab, press ⌥B, and see `∫`. The pane test already proves the same result on fakes.
- Unproven: the bytes for ⌥←, ⌥→ and ⌥⌫. No test sends them, and the encoder source is not read here. A pane test on the recording PTY would show the bytes the encoder gives for each.
- Unproven: the Linux Alt+letter path. `keys.rs:103` treats Alt as consumed on the Mac only. Whether the encoder gives ESC then the letter on Linux is not tested.
