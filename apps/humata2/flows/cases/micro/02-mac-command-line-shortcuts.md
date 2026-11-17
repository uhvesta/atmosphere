# ⌘⌫, ⌘← and ⌘→ edit the shell line as they do in Mac text fields

Layer: micro. Platforms: macOS. (Linux uses Ctrl keys, see `micro/01`.)

## Starting state

- Humata 2 is open on this Mac, with a session that has a shell tab.
- The shell tab has focus. The shell is zsh.
- The prompt line holds `docker compose up --build web`, with the cursor at the end.
- No palette, find bar or paste prompt is open. The view is at the newest output.
- Nothing is held.

## Steps

1. **I do:** Press ⌘←.

   **I see:** Expected: the cursor jumps to the start of the line, before `d`. Actual: nothing moves, and no bytes are sent.

   **State and events:** Expected: zsh receives ^A (0x01), which it binds to start of line. Actual: `terminal/view/src/keys.rs:57-63` returns `KeyIntent::Ignore` for Cmd with no Ctrl or Alt, unless the key is c, v or a scroll key. `terminal/view/src/view.rs:863` returns with nothing sent.

   **Status:** Not built (no milestone). The shortcut is not in `SPEC.md` or `DECISIONS.md`. The code also looks wrong; see Likely bug below.

2. **I do:** Press ⌘→.

   **I see:** Expected: the cursor jumps to the end of the line. Actual: nothing changes.

   **State and events:** Expected: 0x05 (^E). Actual: the same `Ignore` path as step 1.

   **Status:** Not built (no milestone).

3. **I do:** Press ⌘⌫ with the cursor at the end.

   **I see:** Expected: the whole line is deleted, since the text before the cursor runs back to the start. Actual: nothing.

   **State and events:** Expected: 0x15 (^U, kill to line start). Actual: `keys.rs:57-63` again. The key is Backspace with Cmd, and `scroll_key` returns nothing for it, so the result is `Ignore`. `ui/src/keys.rs` binds no `cmd-backspace`.

   **Status:** Not built (no milestone).

4. **I do:** Press ⌘A in the shell tab.

   **I see:** Nothing visible happens in the terminal.

   **State and events:** `keys.rs:61` returns `Ignore`, so the view does not handle it. Edit > Select All is an OS action (`desktop/src/main.rs:246`, `input::SelectAll`). No handler for it exists in `terminal/view/` or `terminal/client/`.

   **Status:** Likely bug at `desktop/src/main.rs:246`. See Gaps.

5. **I do:** Press ⌘Home.

   **I see:** The history scrolls to the top. The shell line does not move.

   **State and events:** `keys.rs:61` maps Cmd+Home to `ScrollTo::Top`. The program never gets the key. This is by design (`terminal/view/src/keys.rs:15-16`, and D73 in `DECISIONS.md`).

   **Status:** Fakes only. `terminal/view/src/keys.rs::history_keys_scroll_the_view_instead_of_reaching_the_program` is a unit test of the mapping. `terminal/view/tests/view.rs::history_keys_and_the_scrollbar_move_through_the_history` checks the scroll on fakes.

## Branches

### ⌘⌫ in the message composer

- **When:** the focus is in the composer, below the terminal, with half-typed text.
- **I do:** Press ⌘⌫.
- **I see:** Expected: the text before the cursor is deleted, as in a Mac text field.
- **Expected:** The composer's own text-input keys apply.
- **Status:** Code, no test. The composer is a text input from the external `gpui-kit` crate (`Cargo.toml:95`). Its key bindings are not in this repo, so I did not confirm them.

### ⌘← in vim

- **When:** vim is open in the shell tab, in normal mode.
- **I do:** Press ⌘←.
- **I see:** Nothing moves, or the cursor goes to the start of the line, as Home does.
- **Expected:** Nothing unexpected happens to vim's buffer.
- **Status:** Not built (no milestone). The path is the same `Ignore` as step 1, so nothing is sent.

### Ctrl+U on Linux

- **When:** the same line on the Linux machine.
- **I do:** Press Ctrl+U.
- **I see:** The line clears.
- **Expected:** 0x15 reaches bash. `terminal/view/src/keys.rs:66-78` leaves Ctrl letters to the program on the non-Mac branch.
- **Status:** Code, no test. Linux runs wait for the LINUX milestone (blocked).

## Expected outcomes

- [ ] ⌘← moves the cursor to the start of the shell line (sends 0x01).
- [ ] ⌘→ moves the cursor to the end of the shell line (sends 0x05).
- [ ] ⌘⌫ deletes the text before the cursor (sends 0x15).
- [ ] ⌘Home and ⌘End scroll the history, and the program never gets them.
- [ ] ⌘A does nothing in the terminal, or selects visible text. It does not run an action that nothing handles.
- [ ] Ctrl+U on Linux clears the line.

The first three fail today.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 ⌘← | Not built (no milestone) | `terminal/view/src/keys.rs:57-63` returns `Ignore` |
| 2 ⌘→ | Not built (no milestone) | Same path |
| 3 ⌘⌫ | Not built (no milestone) | Same path; `ui/src/keys.rs` binds none |
| 4 ⌘A | Likely bug | `desktop/src/main.rs:246` |
| 5 ⌘Home | Fakes only | `terminal/view/src/keys.rs::history_keys_scroll_the_view_instead_of_reaching_the_program` |
| Branch: composer ⌘⌫ | Code, no test | `gpui-kit` keymap, outside this repo |
| Branch: vim ⌘← | Not built (no milestone) | Same path as step 1 |
| Branch: Linux Ctrl+U | Code, no test | `terminal/view/src/keys.rs:66-78` |

## Gaps and likely bugs

- Not built: no Mac line-edit shortcuts exist for the shell line. A search of `SPEC.md`, `DECISIONS.md` and `workflow/milestones/TERM.md` found no entry for ⌘⌫, ⌘← or ⌘→. No milestone owns them.
- Likely bug: `terminal/view/src/keys.rs:57-63`. On Mac, Cmd with no Ctrl or Alt maps only c, v and the four scroll keys. Every other Cmd key falls into `_ if !m.shift => scroll_key(key).map_or(KeyIntent::Ignore, ...)` and is ignored, so ⌘⌫, ⌘← and ⌘→ do nothing, and nothing else in the app binds them. The doc comment on `Platform::Mac` (line 33) says any other Cmd shortcut is the app's, but the app binds none of these. Confirm with a pane test in the style of `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` (Mac-only, recording PTY): press `cmd-backspace` and assert the PTY got `[0x15]`. The test fails today. Do the same for `cmd-left` (0x01) and `cmd-right` (0x05).
- Likely bug: `desktop/src/main.rs:246` shows Edit > Select All, but no handler for `input::SelectAll` exists in the terminal. With a shell tab focused, Select All does nothing. Confirm by choosing Edit > Select All in a shell tab.
- Design note, not a bug: ⌘Home and ⌘End go to the history, not to the shell (`keys.rs:61`). A user cannot reach the shell's Home key with ⌘. Decide whether that is wanted.
- Unknown: the composer's ⌘⌫ depends on the external `gpui-kit` keymap. It is not verified here.
