# Focus follows intent: keys go to the terminal I am in

Layer: micro. Platforms: macOS (⌘ keys) first; Linux and web where the focus code is shared.

## Starting state

- Humata 2 is open in one window, and the window is focused.
- The ledger session is open on this Mac, with its Info panel showing and one shell tab (tab 0) selected.
- Machines: this-mac (online) and devbox-1 (online).
- No overlay, toast or Inbox request is on screen. Nothing has been typed yet.
- I hold the keyboard. The caret is in ledger's terminal.

## Steps

1. **I do:** press ⌘N, pick Shell, type the title "Fresh terminal", click Start, then type `pwd` and Enter without clicking the terminal.
   **I see:** the new session opens on its terminal tab. The typed `pwd` runs and prints `~/humata` and a prompt.
   **State and events:** the new session's terminal (tab 0 of the new session) receives the bytes `pwd\r`. Nothing else is typed into ledger.
   **Status:** Fakes only. `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click`. The test sends the start action itself rather than pressing ⌘N, and it writes to the fake daemon's `terminal_bytes`, not to a real shell. Not proven live: the ⌘N key and a real shell prompt in a new session.

2. **I do:** with ledger open, split the terminal (Shell in the second group), click the terminal in the left group, then click the terminal in the right group and type `pwd` and Enter.
   **I see:** the right group is marked as the focused group. The `pwd` output appears in the right group's terminal, not the left one.
   **State and events:** the session view's `focused_group` changes to 1 when I click the right group, then to 0 when I click the left one, then to 1 again. The keys reach the group that is focused.
   **Status:** Fakes only. `desktop/tests/native/terminal.rs::clicking_a_split_terminal_focuses_that_group` checks `focused_group` only and does not type. The reducer side is `features/session/tests/tabs.rs::terminal_focus_tracks_only_the_visible_tab_in_its_own_session`. Not proven: the keystrokes landing in the clicked split.

3. **I do:** press ⌘K to open the palette, press Esc to close it, then type `ls` and Enter.
   **I see:** the palette opens with its search field focused. Esc closes it. The `ls` output appears in the terminal I had open before the palette.
   **State and events:** when the palette closes, `ui/src/overlays.rs` gives focus back to the handle saved when it opened, provided the terminal target is the same. The keys reach that terminal.
   **Status:** Fakes only. `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click` (its last half: open the palette, Esc, then `ls` lands in the fork's terminal). Fake daemon only.

4. **I do:** with the caret in the terminal, switch to another app (⌘Tab), come back, and type `pwd`.
   **I see:** the caret is still in the same terminal and the typed text runs there.
   **State and events:** the window's focus is restored by the windowing layer. Humata does not record anything about activation.
   **Status:** Code, no test. The only activation call in Humata is `desktop/src/main.rs:385` (`cx.activate(true)` at launch). The tests say headless windows start inactive (`desktop/tests/native/terminal.rs:89`) and activate the window by hand, so no test deactivates and reactivates it.

5. **I do:** with ledger's terminal focused, open the Fork dialog from the session menu, then press Cancel (or Esc), then type `ls` and Enter.
   **I see:** the dialog closes. The typed `ls` appears in ledger's terminal.
   **State and events:** `ui/src/overlays.rs:190-217` (`follow_focus`) saves the focused handle when the first overlay opens and restores it when the last one closes, but only if the terminal target has not changed.
   **Status:** Code, no test. `desktop/tests/native/fork_move.rs` clicks `fork-cancel` (line 46) and `move-cancel` (line 75), and never types after the close. `desktop/tests/native/session.rs` (around line 452) presses Esc on the fork dialog but does not type afterwards. The same is true for the Move dialog.

6. **I do:** focus a text field that is not the terminal (the send-text box in the lab harness, or the palette), type a command, then click Send.
   **I see:** the text stays in the field. Neither shell prints it. Clicking the lab's Send button sends the text to the terminal I chose, and only to that one.
   **State and events:** the field's text reaches no PTY until Send is clicked. The bytes go to the chosen terminal alone.
   **Status:** Works (real shell). `terminal/lab/tests/app.rs::real_shells_render_move_paste_detach_resume_and_resize_headlessly` runs two real zsh shells. It turns echo on so a leak would show, types into a focused field, and asserts that both screens are unchanged. The palette-specific version is `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` (Fakes only).

7. **I do:** run a full-screen program that asks for focus reports (mode 1004). Then move focus away from the terminal and back (the find bar does this: ⌘F opens it, Esc closes it).
   **I see:** the program is told when it loses focus and when it gets it back. A program that did not ask is told nothing.
   **State and events:** the pane writes `ESC [ O` when it loses focus and `ESC [ I` when it gets focus, only after the program has asked. Both bytes go to the program's input.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::programs_that_ask_are_told_when_the_pane_gains_and_loses_focus`. It uses the real `ghostty_vt` emulator with a recording PTY, not a real shell. Not built live: no lab test runs a program that asks for focus reports.

## Branches

### A toast appears while I type

- **When:** a toast (for example "Archived …") appears while I am typing in the terminal.
- **I do:** keep typing.
- **I see:** the toast at the bottom of the window. My keys still go to the terminal.
- **Expected outcome:** focus does not move. The toast's Undo button is reachable by click, but the toast takes no keyboard focus on its own.
- **Status:** Code, no test. `ui/src/overlay_toasts.rs` makes no focus call. `ui/src/overlays.rs:288` calls `follow_focus` only with overlay kinds, and toasts are not one of them. No test types while a toast is up.

### An Inbox request arrives while I type

- **When:** a new request arrives for a session I am typing in.
- **I do:** keep typing.
- **I see:** the Inbox count or sidebar badge changes. The keys still go to the terminal.
- **Expected outcome:** focus stays in the terminal. No overlay, route or panel opens for the request.
- **Status:** Code, no test. Arrival of a request does not change the route or the overlay stack (`features/app/src/route.rs`, `ui/src/` has no `window.focus` on the Inbox path). `desktop/tests/live_inbox.rs` builds windows with `focus: false` and asserts nothing about focus.

### The session I was in is archived from elsewhere

- **When:** another client, or the CLI on this Mac, archives the open session while I am in its terminal.
- **I do:** keep typing.
- **I see (expected):** the page leaves the archived session, and my keys do not go to its terminal.
- **I see (code):** the session page stays up.
- **Expected outcome:** the session page leaves; keys reach no terminal of the archived session; focus lands on Home or the sidebar.
- **Status:** Works (real daemon): `desktop/tests/live_flows/lifecycle.rs::a_session_archived_from_another_window_closes_its_page_here` archives the open session through a second client of the daemon, and the page closes. The reducer side is `features/app/tests/nav_bugs.rs::a_session_archived_in_another_window_closes_its_page_here`. (Was a likely bug: no refresh cleared the open page; fixed by BUGS.)

## Expected outcomes

- [ ] A new shell session takes typed text with no click.
- [ ] Clicking a split makes that group the focused one.
- [ ] Typed keys reach the focused split, not the other one.
- [ ] Closing the palette with Esc gives the keyboard back to the terminal it was opened over.
- [ ] Switching apps and back leaves the caret in the same terminal.
- [ ] Closing the fork or move dialog gives the keyboard back to the terminal.
- [ ] Keys typed into a field that is not the terminal reach no PTY.
- [ ] A program that asked for focus reports gets `ESC [ I` and `ESC [ O`, and one that did not ask gets nothing.
- [ ] A toast does not take focus.
- [ ] An Inbox request does not take focus.
- [ ] When the open session is archived elsewhere, its page leaves and no keys go to its terminal.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. ⌘N session takes input at once | Fakes only | `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click` |
| 2. Click a split, type into it | Fakes only | `desktop/tests/native/terminal.rs::clicking_a_split_terminal_focuses_that_group`; `features/session/tests/tabs.rs::terminal_focus_tracks_only_the_visible_tab_in_its_own_session` |
| 3. Palette Esc returns focus | Fakes only | `desktop/tests/native/terminal.rs::new_sessions_and_selected_tabs_receive_input_without_an_extra_click` (second half) |
| 4. Switch app and back | Code, no test | `desktop/src/main.rs:385` (launch only); no activation test |
| 5. Fork or move dialog closes, focus returns | Code, no test | `ui/src/overlays.rs:190-217`; `desktop/tests/native/fork_move.rs:46,75` (no typing after close) |
| 6. Unfocused field gets no keystrokes | Works (real shell) | `terminal/lab/tests/app.rs::real_shells_render_move_paste_detach_resume_and_resize_headlessly` |
| 7. Focus report to a program that asked | Fakes only | `terminal/pane/tests/pane.rs::programs_that_ask_are_told_when_the_pane_gains_and_loses_focus` |
| Branch: toast appears | Code, no test | `ui/src/overlay_toasts.rs`; `ui/src/overlays.rs:288` |
| Branch: Inbox request arrives | Code, no test | `features/app/src/route.rs`; `desktop/tests/live_inbox.rs` (`focus: false` only) |
| Branch: session archived elsewhere | Works (real daemon) | `desktop/tests/live_flows/lifecycle.rs::a_session_archived_from_another_window_closes_its_page_here`; `features/app/tests/nav_bugs.rs::a_session_archived_in_another_window_closes_its_page_here` |

## Gaps and likely bugs

- Fixed (was a likely bug): a session archived elsewhere now closes its page here. See the branch above.
- Fixed (was a latent likely bug): the Desk sheet and the Add machine dialog keep the keys from a terminal whose target changes: `features/app/tests/key_bugs.rs::a_desk_dialog_or_add_machine_holds_the_keys_from_a_terminal` (fakes).
- Code, no test: focus back after the Fork and Move dialogs, and focus after a window is switched away and back.
- Fakes only: the new-session and split tests check the fake daemon's bytes and `focused_group`, not a real shell. Only the lab test (terminal/lab/tests/app.rs) runs real shells, and it covers the non-terminal field case, not palette or dialog returns.
- Not built: none found for this case. The focus report is built and tested only with a recording PTY.
