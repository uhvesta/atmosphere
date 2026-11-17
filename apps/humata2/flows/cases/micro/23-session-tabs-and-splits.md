# Session tabs open, split and close from the tab row, and the tab shortcuts I expect do nothing

Micro. Platforms: macOS. Linux and web are not checked for this case.

## Starting state

- Humata 2 is open on this Mac, on the session "ledger" from the design fixture. Its machine is online.
- The side panel is open on Changes. The sidebar is visible. The window is not in full screen.
- The tab row shows one group with the agent's terminal (`claude`, tab 0), selected. Its **+** is at the end of the row.
- The terminal has focus. No file tab is open. The session has one group.
- The Back trail is empty.

## Steps

1. **I do:** click **+** at the end of the tab row.
   **I see:** a menu opens with `zsh` (a new shell tab), the agent entries, and "Open in second group" entries. Cancel is at the bottom.
   **State and events:** `SessionAction::TerminalMenu(Some(0))` is set. The menu is for group 0.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` clicks `add-tab-0` and checks that `new-terminal-menu` is shown. The menu is in `ui/src/session_tabs.rs` (`menu`, around line 284).

2. **I do:** click `zsh`.
   **I see:** a second terminal tab, `zsh`, appears after the agent's tab. It is selected, and its terminal has focus. Typing goes to it.
   **State and events:** `SessionAction::OpenTerminal { agent: Shell, split: false }` runs `terminals.open`. Then `TerminalOpened` adds `Tab::Terminal(n)` to group 0 and selects it (`features/session/src/lib.rs:854-900`).
   **Status:** Fakes only
   Evidence: the same reducer path runs in the split test above, with `split: true`. No test clicks the `zsh` item (`new-terminal-0-zsh`). Unproven live: a real shell opening from the menu.

3. **I do:** open the Files panel, select `api/src/client.rs`, and click **Open big**.
   **I see:** a file tab for `client.rs` appears in the tab row, after the terminals, with a ✕ on it. It is selected.
   **State and events:** `SessionAction::OpenFile` adds `Tab::File { repo, path }` to the group and selects it.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::files_open_in_the_tab_row_and_close_back_to_the_terminal` (the file tab is selected, and the Files panel calls `files-open-big`).

4. **I do:** click **+** and then "Open in second group" on the agent's entry.
   **I see:** the tab row splits into two groups side by side. Each group has its own tabs. The new terminal is in the second group, and the second group is focused.
   **State and events:** `TerminalOpened` with `split: true` calls `show_tab(_, 1, true)`, which creates group 1 (`features/session/src/tabs.rs:100-127`).
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` (asserts two groups and `terminal-secondary`). `features/session/tests/tabs.rs::split_is_limited_to_two_groups_and_attaches_an_existing_agent`.

5. **I do:** click **+** in group 1 and choose "Open in second group" again.
   **I see:** the row still has two groups. The new terminal joins the second group. No third group appears.
   **State and events:** `show_tab` with `split: true` targets group 1 again. The reducer does not refuse the split. The comment in `features/session/src/tabs.rs:101-102` says another split reuses the second group.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` (asserts `groups.len() == 2` after the second split).

6. **I do:** open the palette (⌘K), type `Join tab groups`, and press Enter.
   **I see:** one group again, with every tab of both groups in it. The programs keep running: the shell count is the same as before.
   **State and events:** `Command::JoinTabGroups` sends `SessionAction::CloseGroup(1)` (`features/app/src/commands.rs:82-84`). `close_group` moves group 1's tabs into group 0 (`features/session/src/tabs.rs:129`).
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` (picks `Command::JoinTabGroups` and checks the shell count). The palette side is in `features/palette/tests/join_tab_groups.rs`. The join is only in the palette: no group button exists (see the gaps).

7. **I do:** click the ✕ on the `client.rs` file tab.
   **I see:** the file tab is gone. The terminal that was under it is shown.
   **State and events:** `SessionAction::CloseTab { group, index }` removes the tab. The group selects its terminal. The conversation is cleared if it was the closed tab.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/session.rs::files_open_in_the_tab_row_and_close_back_to_the_terminal` (clicks `close-tab-0-2` and checks that `session-file` is gone and `terminal` shows).

8. **I do:** press ⌥⌘0.
   **I see:** the side panel hides. Press ⌥⌘0 again and it comes back on Changes.
   **State and events:** `PaneAction::Toggle(Panel)` flips `layout.panel.collapsed` (`features/app/src/lib.rs:842-851`).
   **Status:** Code, no test
   Evidence: `ui/src/shell.rs:538-540` binds the action. `features/app/tests/every_action.rs` maps the key to a palette command, but no test presses the key or checks the panel state. The per-session problem is a Likely bug (see the branches).

9. **I do:** press ⌘T.
   **I see:** nothing new opens. The terminal gets no bytes for ⌘T.
   **State and events:** none.
   **Status:** Not built (no milestone). The key is not bound in `ui/src/keys.rs`. The only ⌘T handling is in the `TabStrip` of `terminal/pane/src/tabs.rs` (lines 544-566), and the app does not mount it (D74 made tabs secondary). `terminal/client/src/replica.rs:54-66` would bind `cmd-t` through `register_host_shortcuts`, but only `terminal/client/tests/component.rs:151` calls it.

10. **I do:** press ⌘W.
    **I see:** the active tab closes. For a file tab, the terminal returns. Nothing else happens.
    **State and events:** the expected event is the tab close, with no program stopped (NV-37, D73).
    **Status:** Not built (NV-37). `ui/src/keys.rs` binds no `cmd-w`, and `desktop/src/main.rs` binds only `cmd-q`, `cmd-h`, `alt-cmd-h` and `cmd-m` (lines 203 and 219-221). Today ⌘W does nothing.

11. **I do:** press ⌃Tab.
    **I see:** nothing changes in the tab row. The terminal gets the Tab key with Control.
    **State and events:** none in the app.
    **Status:** Not built (no milestone). The app binds no `ctrl-tab`. `terminal/view/src/keys.rs` sends Control keys that are not copy or paste to the program. The `ctrl-tab` binding exists only in `terminal/client/src/replica.rs:71` and in the unmounted `TabStrip`.

12. **I do:** press ⇧⌘↩ (Return).
    **I see:** nothing happens. The palette, though, lists "Terminal full screen" with the hint `⇧⌘↩`.
    **State and events:** none. The full screen is not reached by this key.
    **Status:** Not built (NV-37). The key is not bound. The palette hint is a Likely bug, see the gaps. The button "Expand terminal" (`ui/src/pages.rs:289-296`) and the palette command (`features/app/src/commands.rs:66-67`) do reach full screen.

## Branches

### Closing the only tab of a group

- **When:** a group has one file tab left, and no split is open.
- **I do:** click the ✕ on the file tab.
- **I see:** the terminal is shown again. The group has its agent's terminal in it.
- **Expected outcome:** the group goes back to a terminal, and nothing is stopped.
- **Status:** Fakes only
  `features/session/tests/tabs.rs::a_file_opens_once_and_closing_it_returns_to_the_terminal`. The reducer is `features/session/src/lib.rs:991-1020`: a single tab in a single group becomes `Tab::Terminal(0)`.

### Closing the only file in a split

- **When:** group 1 has one file tab, and two groups are open.
- **I do:** click the ✕ on that file tab.
- **I see:** group 1 goes away. Its remaining tabs are all in group 0.
- **Expected outcome:** one group with every remaining tab, and the focus on group 0.
- **Status:** Fakes only
  `features/session/tests/tabs.rs::closing_the_only_file_in_a_split_removes_that_group`. The reducer is `features/session/src/lib.rs:1000-1006`.

### Closing a shell tab

- **When:** the tab is a shell (`zsh`) or the agent's terminal.
- **I do:** look for a ✕ on the `zsh` tab.
- **I see:** no ✕. The tab cannot be closed from the tab row. Its program keeps running.
- **Expected outcome:** the spec does not say how a shell tab closes. SE-8 gives ✕ to file and review tabs only, and SE-9 says nothing about closing a shell tab.
- **Status:** Not built (no milestone). `ui/src/session_tabs.rs:155` shows ✕ only when `!matches!(tab, Tab::Terminal(_))`. The only other way out is the shell exiting, which this case does not check.

### A process is running when its file tab closes

- **When:** the shell is running a command, and the person closes a file tab.
- **I do:** start `sleep 600` in the `zsh` tab, then close a file tab.
- **I see:** the file tab goes. No prompt appears, and the `sleep` keeps running.
- **Expected outcome:** closing a view never stops a program (D73).
- **Status:** Code, no test
  `features/session/src/lib.rs:991-1020` (`CloseTab`) sends no stop request. The pane crate test `terminal/pane/tests/tabs.rs::closing_a_tab_closes_its_view_and_not_its_process` checks this for the unmounted `TabStrip`, not for the app.

### A third split

- **When:** two groups are open, and the person opens "Open in second group" again.
- **I do:** repeat step 5.
- **I see:** no refusal and no third group. The new terminal lands in group 1, the second group.
- **Expected outcome:** two groups at most, as NV-36 says ("more than that is a fork"). The current code does this by reusing group 1, not by refusing the split.
- **Status:** Fakes only
  `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running`. The menu item has no condition for an existing split (`ui/src/session_tabs.rs:318-321`), so it is always offered.

### A file opened in the tab row and then closed

- **When:** a file tab is open, and the person selects the terminal tab, then comes back to the file.
- **I do:** select the file tab, select the terminal, select the file again.
- **I see:** the file is shown again with its contents. Its tab is not duplicated.
- **Expected outcome:** one tab per file. `features/session/tests/tabs.rs::a_file_opens_once_and_closing_it_returns_to_the_terminal` checks "opens once".
- **Status:** Fakes only
  The "opens once" check is in `features/session/tests/tabs.rs`. The return to the file after a terminal tab is not checked live.

### Full screen with the wrong key

- **When:** the person tries ⇧⌘↩ and then Escape, expecting to leave full screen with Escape.
- **I do:** press ⇧⌘↩, then Escape.
- **I see:** ⇧⌘↩ does nothing. Escape does nothing either, since `features/app/src/overlay.rs` `escape` does not handle full screen.
- **Expected outcome:** full screen is entered and left with the button or the palette command. The button "Expand terminal" toggles it.
- **Status:** Code, no test
  `desktop/tests/native/shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` covers the button, with fakes. No test covers ⇧⌘↩ or Escape in full screen.

### ⌥⌘0 with no panel open

- **When:** the session has no panel open.
- **I do:** press ⌥⌘0.
- **I see:** the Changes panel opens.
- **Expected outcome:** the panel opens on Changes, as the code does, and the sidebar and terminal keep their sizes.
- **Status:** Code, no test
  `features/app/src/lib.rs:831-837` sends `SetPanel(Some(Changes))` when the route is a session and no panel is open.

### ⌥⌘0 hides the panel in every session

- **When:** the panel is hidden in session A, and the person opens session B that has a panel open.
- **I do:** press ⌥⌘0 in A to hide the panel, then open B.
- **I see:** B's panel is hidden too.
- **Expected outcome:** each session keeps its own panel state (NV-37 says "Both states are kept per session (SE-13)").
- **Status:** Likely bug
  `features/app/src/layout.rs:72-77` keeps `Layout.panel.collapsed` as one value for the whole app. `features/app/src/lib.rs:842-851` flips that value. Confirm: hide the panel in one session, then open another session that has a panel open. Its panel should stay open, and the code says it will be hidden.

## Expected outcomes

- [ ] The **+** menu opens for group 0 and lists a shell, the agents and the split entries.
- [ ] Choosing `zsh` adds a second shell tab, which is selected and has focus.
- [ ] A file opened big adds one file tab with a ✕, and the tab is selected.
- [ ] "Open in second group" makes two groups side by side, and the new terminal is in the second group.
- [ ] A second "Open in second group" keeps two groups, and creates no third group.
- [ ] `Join tab groups` in the palette makes one group, and no program stops.
- [ ] The ✕ on a file tab closes it and shows a terminal. The shell count does not change.
- [ ] ⌥⌘0 hides the side panel, and a second ⌥⌘0 shows it again.
- [ ] ⌘T, ⌘W, ⌃Tab and ⇧⌘↩ each do what NV-37 or the tab rules say, or are listed as not built.
- [ ] A panel hidden in one session stays hidden only in that session.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. **+** opens the menu | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| 2. Choose `zsh` for a second shell tab | Fakes only | Same reducer as the split test; no test clicks `zsh` |
| 3. Open a file big into the tab row | Fakes only | `desktop/tests/native/session.rs::files_open_in_the_tab_row_and_close_back_to_the_terminal` |
| 4. Split into two groups | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| 5. A second split keeps two groups | Fakes only | Same test; `features/session/tests/tabs.rs::split_is_limited_to_two_groups_and_attaches_an_existing_agent` |
| 6. Join tab groups in the palette | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| 7. ✕ closes a file tab | Fakes only | `desktop/tests/native/session.rs::files_open_in_the_tab_row_and_close_back_to_the_terminal` |
| 8. ⌥⌘0 toggles the panel | Code, no test | `ui/src/shell.rs:538-540`; `features/app/src/lib.rs:842-851` |
| 9. ⌘T | Not built (no milestone) | `ui/src/keys.rs` (no binding); `terminal/pane/src/tabs.rs:544-566` (unmounted) |
| 10. ⌘W | Not built (NV-37) | `ui/src/keys.rs` (no binding); `desktop/src/main.rs:203, 219-221` |
| 11. ⌃Tab | Not built (no milestone) | `terminal/client/src/replica.rs:70-71` (not called by the app) |
| 12. ⇧⌘↩ | Not built (NV-37) | `ui/src/keys.rs` (no binding); palette hint at `features/palette/src/lib.rs:701` |
| Branch: close the only tab of a group | Fakes only | `features/session/tests/tabs.rs::a_file_opens_once_and_closing_it_returns_to_the_terminal` |
| Branch: close the only file in a split | Fakes only | `features/session/tests/tabs.rs::closing_the_only_file_in_a_split_removes_that_group` |
| Branch: close a shell tab | Not built (no milestone) | `ui/src/session_tabs.rs:155` |
| Branch: process running when a file tab closes | Code, no test | `features/session/src/lib.rs:991-1020` |
| Branch: third split | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| Branch: file opened, closed, reopened | Fakes only | `features/session/tests/tabs.rs::a_file_opens_once_and_closing_it_returns_to_the_terminal` |
| Branch: full screen with ⇧⌘↩ and Escape | Code, no test | `desktop/tests/native/shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` (button only) |
| Branch: ⌥⌘0 with no panel open | Code, no test | `features/app/src/lib.rs:831-837` |
| Branch: ⌥⌘0 hides the panel in every session | Likely bug | `features/app/src/layout.rs:72-77`; `features/app/src/lib.rs:842-851` |

Steps: 0 Works, 7 Fakes only (steps 1 to 7), 1 Code no test (step 8), 4 Not built (steps 9 to 12), 0 Likely bug. Total 12.
Branches: 4 Fakes only, 3 Code no test, 1 Not built, 1 Likely bug. Total 9.

## Gaps and likely bugs

- Not built (NV-37): ⌘W does not close the active tab. Nothing binds `cmd-w`.
- Not built (NV-37): ⇧⌘↩ does not toggle Terminal full screen. The key is not bound.
- Not built (no milestone): ⌘T, ⌃Tab and the other `TabStrip` shortcuts. The `TabStrip` in `terminal/pane/src/tabs.rs` has them and is tested, but the app does not mount it (D74 made tabs secondary). `register_host_shortcuts` in `terminal/client/src/replica.rs:54-70` is called only by a test, so the app does not bind `cmd-t`, `cmd-k`, `ctrl-tab` or `ctrl-shift-tab`.
- Not built (SE-27): the group ✕. `ui/src/session_tabs.rs` has no `close-group` button, so the only way to join is the palette command. The native test asserts that no `close-group` button exists.
- Not built (SE-27): moving a tab "from its menu". Dragging works (`ui/src/session_tabs.rs:243` and `:585`), but the tab menu has no move entry.
- Not built (no milestone): closing a shell tab. No ✕ is shown, and the spec does not name another way.
- Likely bug: `features/palette/src/lib.rs:699-701` shows the hint `⇧⌘↩` for Terminal full screen, but nothing binds that key. Confirm by pressing ⇧⌘↩ with the palette showing the hint.
- Likely bug: `features/app/src/layout.rs:72-77` and `features/app/src/lib.rs:842-851`. The panel's collapsed state is global, not per session as NV-37 and SE-13 say. Confirm with the two-session check above.
- Question for the owner: the "Open in second group" menu item is offered even when two groups exist (`ui/src/session_tabs.rs:318-321`). NV-36 says more than two is a fork. The code reuses group 1. Should the menu hide the split when two groups exist?
- Gap: full screen is set by `AppAction::ToggleFullScreen` in `features/app/src/lib.rs` (the `FullScreen` type is in `features/app/src/layout.rs`). It is reached by the button in `ui/src/pages.rs:289-296` and by the palette command, not by a key.
- Gap: no live test covers a session's tab row. All of the tab tests use fakes.
