# Keyboard navigation opens sessions, folds the sidebar and reaches every sheet

Micro. Platforms: macOS, with Linux noted where its keys differ. Web is not covered by this case.

## Starting state

- Humata 2 is open on this Mac, on Home. The design fixture data is loaded.
- The sidebar is visible and not collapsed. It shows the channels and their session trees, and a Desk section.
- Focus is on the Home page, not the sidebar and not a terminal.
- No overlay is open. No session is open. The Back trail is empty.
- The mouse is not used after the first step.

## Steps

1. **I do:** press ⌘L.
   **I see:** the sidebar takes focus. No row shows the cursor yet.
   **State and events:** the sidebar's focus handle is focused. The sidebar cursor is still empty.
   **Status:** Code, no test
   Evidence: `ui/src/keys.rs:107` binds `cmd-l` to `FocusSidebar`. `ui/src/shell.rs:535-537` calls `window.focus(&this.sidebar_focus)`. No test presses ⌘L.

2. **I do:** press ↓ twice.
   **I see:** the cursor moves down two rows. Section headings are skipped.
   **State and events:** `SidebarState.cursor` moves to the next row that has a key. The first ↓ with no cursor selects the first row.
   **Status:** Fakes only
   Evidence: `features/sidebar/tests/desk.rs::down_walks_from_the_first_desk_row_through_the_desk_into_the_channels` and `with_the_desk_rows_not_showing_down_skips_them`. These are reducer tests. They cover Desk rows and channels, not every tree row. The key itself is not pressed. Bindings: `ui/src/keys.rs:112-113` (sidebar context only), and `features/sidebar/src/lib.rs:291-317`.

3. **I do:** press → on a closed channel, then → again, then ←, then End, then Home.
   **I see:** → opens the channel's tree and the cursor moves to its first child. ← on a child moves to its parent. End goes to the last row. Home goes to the first row.
   **State and events:** `Move::In`, `Move::Out`, `Move::First` and `Move::Last` change the cursor. Opening a channel with → toggles its folding.
   **Status:** Code, no test
   Evidence: `features/sidebar/src/lib.rs:318-347` for In and Out. `features/sidebar/src/lib.rs:297-317` for First and Last. Bindings: `ui/src/keys.rs:112-117` (sidebar context only). No test calls these moves.

4. **I do:** press Enter on a session row.
   **I see:** the session opens. The sidebar row stays under the cursor and is marked as current.
   **State and events:** the sidebar sends `Delegate::OpenSession`, and the app switches to `Route::Session`. The Back trail is cleared (`features/app/src/route.rs:104-109`).
   **Status:** Fakes only
   Evidence: `features/sidebar/tests/desk.rs::enter_on_a_desk_row_opens_that_session` and `opening_a_desk_row_keeps_the_cursor_on_it`. Both use Desk rows. Enter on a row in a channel tree has no test.

5. **I do:** press ⌥⌘→, then ⌥⌘←.
   **I see:** ⌥⌘→ opens every channel and fork tree in the sidebar. ⌥⌘← folds them all again.
   **State and events:** `SidebarAction::ExpandAll` clears the folding. `SidebarAction::CollapseAll` folds every channel and session. The result is kept as fold state.
   **Status:** Code, no test
   Evidence: `ui/src/keys.rs:100-101` (sidebar context only). `ui/src/shell.rs:509-515`. `features/sidebar/src/lib.rs:186-209`. `features/app/tests/every_action.rs` checks only that the key maps to a palette command.

6. **I do:** press ⌘0.
   **I see:** the sidebar folds into a thin strip, 14 points wide, with a chevron and the label `Show sidebar`.
   **State and events:** `state.layout` marks the sidebar pane as collapsed (`features/app/src/layout.rs:123-126` and `:174`). The next ⌘0 is in branch 6.
   **Status:** Code, no test
   Evidence: `ui/src/keys.rs:102` binds `cmd-0`. `ui/src/shell.rs:515-521` sends `PaneAction::Toggle(PaneId::Sidebar)` when the sidebar is shown. `ui/src/shell.rs:575-594` draws the strip. No test covers the toggle.

7. **I do:** press ⌘1, then ⌘2, then ⌘3.
   **I see:** Home, then the Inbox, then the Board. The Board opens for the whole workspace.
   **State and events:** `AppAction::Switch` to `Route::Home`, `Route::Inbox` and `Route::Board(Scope::Global)`. Each switch clears the Back trail.
   **Status:** Code, no test
   Evidence: `ui/src/keys.rs:103-105`. `ui/src/shell.rs:523-531` (the `GoHome`, `GoInbox` and `GoBoard` handlers). `features/app/src/route.rs:104-109` clears the trail. `features/app/tests/every_action.rs` checks that each key pairs with its palette command and its shortcut text. It does not press the keys.

8. **I do:** from Home, open a session from a card (not the sidebar), then press ⌘[.
   **I see:** Home is back in front, with the scroll position it had.
   **State and events:** the drill-in pushes Home on the trail. ⌘[ pops it (`features/app/src/route.rs:176-184`).
   **Status:** Fakes only
   Evidence: `features/app/tests/navigation.rs::switching_screens_clears_the_drill_in_trail` covers `Navigate`, then `Back`. `features/app/tests/find_conversation.rs` (around line 122) covers `Back` after a search hit. The key is not pressed, and the scroll position is not checked.

9. **I do:** press ⌘/.
   **I see:** the Keyboard shortcuts sheet opens over the page. It lists every shortcut, as `ui/src/keys.rs` `SHORTCUTS` has it, and it scrolls when the list is long.
   **State and events:** `Overlay::Shortcuts` is pushed on the overlay stack.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/overlays.rs::shortcuts_sheet_light`, `shortcuts_sheet_dark` and `shortcuts_sheet_scrolled_light`. They open the sheet with `AppAction::ShowShortcuts`, not the key. Binding: `ui/src/keys.rs:99`.

10. **I do:** press Escape.
    **I see:** the sheet closes. The page underneath is unchanged.
    **State and events:** the top overlay is popped from `state.overlays`, and nothing else changes.
    **Status:** Code, no test
    Evidence: `features/app/src/overlay.rs` (`escape`) pops the top overlay. No test covers the Shortcuts overlay specifically. The other overlay tests are for dialogs and menus (`features/app/tests/machines.rs::escape_closes_the_open_dialog`).

11. **I do:** press ⌘,.
    **I see:** Settings opens. The route is Settings, and the Back trail is cleared.
    **State and events:** `AppAction::Switch(Route::Settings)`.
    **Status:** Code, no test
    Evidence: `ui/src/keys.rs:106`. `ui/src/shell.rs:532-534`. The palette's `Open Settings` pairs with the same key in `features/app/tests/every_action.rs`.

12. **I do:** press ⌘N.
    **I see:** the New session sheet opens on the first channel, with This Mac ticked.
    **State and events:** `AppAction::StartNewSession` opens the sheet. The sheet's first channel and machine come from the loaded workspace.
    **Status:** Works (real daemon)
    Evidence: `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` presses `secondary-n` on a real daemon and asserts that the sheet is open. Binding: `ui/src/keys.rs:95`.

13. **I do:** press ⇧⌘N.
    **I see:** the New channel sheet opens, with the repositories `gh` can see and This Mac ticked.
    **State and events:** `AppAction::StartNewChannel` opens the sheet.
    **Status:** Fakes only
    Evidence: `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac` sends `A::StartNewChannel`, not the key. Binding: `ui/src/keys.rs:96`.

14. **I do:** open a session, then press ⌥⌘M.
    **I see:** the palette opens, asking which machine the session moves to. The list has only machines.
    **State and events:** `AppAction::OpenPalette`, then `PaletteAction::Begin(Command::MoveSession)`. The palette's ask state is set (`features/palette/src/lib.rs`, `pick_command`).
    **Status:** Code, no test
    Evidence: `ui/src/keys.rs:109` binds `alt-cmd-m`. `ui/src/shell.rs:541-551`. The palette ask is covered in micro-21, step 4. No test presses ⌥⌘M.

## Branches

### ⌘[ with no history

- **When:** the Back trail is empty, for example after a sidebar open or on the first page.
- **I do:** press ⌘[.
- **I see:** nothing changes. The route and the page stay the same.
- **Expected outcome:** the route does not change and no error shows.
- **Status:** Fakes only
  `features/app/tests/navigation.rs::switching_screens_clears_the_drill_in_trail` ends with a `Back` on an empty trail, and the state does not change. The reducer is `features/app/src/route.rs:176-184`.

### Esc with nothing to dismiss, terminal focused

- **When:** no overlay is open, and the focus is in a session's terminal.
- **I do:** press Escape.
- **I see:** the program in the terminal gets the Escape key. A shell ignores it. Neovim leaves insert mode.
- **Expected outcome:** the Escape reaches the program, and the app does nothing. No stray Escape reaches the terminal when the focus is elsewhere.
- **Status:** Code, no test
  `ui/src/keys.rs:98` binds `escape` to `Dismiss`. In `terminal/view/src/keys.rs:164` Escape is a key for the program, so `KeyIntent::Send` is returned. `terminal/view/src/view.rs:929` stops propagation for a sent key. gpui's `dispatch_key_event` runs key listeners before key bindings and stops when propagation is stopped, so `Dismiss` does not run. This reading is from the gpui source in the Bazel cache. The Neovim lab test `terminal/lab/tests/neovim.rs:197` sends Escape to a real Neovim, but it does not run the app shell.

### Esc with nothing to dismiss, focus in the sidebar or a page

- **When:** no overlay is open, and the focus is on the sidebar or a page.
- **I do:** press Escape.
- **I see:** nothing changes.
- **Expected outcome:** nothing happens and no key is sent anywhere.
- **Status:** Code, no test
  `features/app/src/overlay.rs` `escape` returns `Effect::none()` when no overlay, dialog or drawer is open.

### ↑ at the first row, ↓ at the last row, Home and End

- **When:** the cursor is on the first or last row.
- **I do:** press ↑ on the first row, then ↓ on the last row.
- **I see:** the cursor stays on the first row for ↑ and on the last row for ↓.
- **Expected outcome:** the cursor stays put at each end, with no wrap.
- **Status:** Fakes only
  `features/sidebar/tests/desk.rs::up_from_the_first_desk_row_stays_there` covers ↑ at the top, for Desk rows only. `features/sidebar/src/lib.rs:316-317` clamps both ends. No test covers ↓ at the bottom of the tree, or Home and End.

### An app shortcut pressed while the terminal is focused

- **When:** the terminal has focus, and the person presses a key the app also binds.
- **I do:** focus a terminal, press ⌘1, then press Ctrl+C, then press ⇧⌘N.
- **I see:** ⌘1 goes to the app and opens Home. Ctrl+C goes to the program, which stops. ⇧⌘N opens the New channel sheet.
- **Expected outcome:** ⌘ combinations belong to the app. Escape and Ctrl combinations go to the program when the terminal is focused.
- **Status:** Code, no test
  `terminal/view/src/keys.rs:65` returns `KeyIntent::Ignore` for ⌘ keys that are not copy, paste or scroll. The terminal's own unit tests cover only the mapping, for example `terminal/view/src/keys.rs:301` (`cmd-t` is `Ignore`). No app-level test presses a shortcut with the terminal focused.

### ⌘0 a second time, with the sidebar hidden

- **When:** the sidebar is already hidden (branch of step 6).
- **I do:** press ⌘0.
- **I see:** the sidebar appears as an overlay over the page. It does not come back as a docked column. When the overlay closes, the sidebar is hidden again.
- **Expected outcome:** the sidebar comes back docked, at the width it had before (NV-13). The Palette's `Toggle sidebar` command does this.
- **Status:** Likely bug
  `ui/src/shell.rs:515-521` sends `AppAction::ToggleNavigationOverlay` whenever `collapsed` is true. `features/app/src/lib.rs:749-751` only flips `navigation_overlay`. The palette's `Toggle sidebar` (`features/app/src/commands.rs:29-31`) calls `state.layout.toggle`, which docks it. Confirm: press ⌘0 twice on a session page. The sidebar stays hidden after the overlay closes.

### ⌘L with the sidebar hidden

- **When:** the sidebar is hidden, and the overlay is closed.
- **I do:** press ⌘0 to hide the sidebar, then press ⌘L.
- **I see:** nothing visible happens. The sidebar is not drawn, so nothing has focus.
- **Expected outcome:** the sidebar opens as the overlay and takes focus, so the keys reach it.
- **Status:** Likely bug
  `ui/src/shell.rs:535-537` focuses `sidebar_focus` without checking the hidden state. The sidebar is drawn only when shown (`ui/src/shell.rs:572`), or inside the overlay when `navigation_overlay` is open (`ui/src/shell.rs:687`). Confirm by pressing ⌘0, then ⌘L, then ↓, and watching whether the cursor moves.

### ⌥⌘→ with focus outside the sidebar

- **When:** the focus is in the diff, the Inbox or the Board.
- **I do:** open the Inbox with ⌘2, then press ⌥⌘→.
- **I see:** nothing happens.
- **Expected outcome:** the groups in the Inbox expand. NV-18 and NV-19 say every tree or grouped list, including Inbox groups, the Board table's groups and the diff's repo groups, responds to ⌥⌘→ when it has focus.
- **Status:** Not built (NV-18, NV-19)
  `ui/src/keys.rs:100-101` binds the key only in the `Sidebar` context.

### ⌘[ after opening a session from the sidebar

- **When:** a session was opened from a sidebar row (a lateral move).
- **I do:** open a session from the sidebar, then press ⌘[.
- **I see:** nothing changes. Back does not go to the previous page.
- **Expected outcome:** nothing changes. NV-4 says lateral moves get no Back and clear the trail.
- **Status:** Code, no test
  `features/app/src/lib.rs:544-546` sends `Delegate::OpenSession` to `self.switch`, and `features/app/src/route.rs:104-109` clears the trail. No test covers this path.

### ⌥⌘M outside a session

- **When:** the route is Home, the Inbox or a channel page.
- **I do:** press ⌥⌘M.
- **I see:** nothing. The palette does not open, and no message says why.
- **Expected outcome:** the shortcut does nothing outside a session. A short note or the palette's Move entry would make it clearer.
- **Status:** Code, no test
  `ui/src/shell.rs:541-551` checks the route before it opens the palette.

### Linux: the same keys

- **When:** the app runs on Linux.
- **I do:** press Ctrl+1, then Ctrl+L, then Ctrl+, on Linux.
- **I see:** none of them does what the sheet says. The bindings need Super (the platform key), not Ctrl.
- **Expected outcome:** each key works with Ctrl on Linux, as the shortcut sheet and the palette hints show ⌘.
- **Status:** Likely bug
  `ui/src/keys.rs:94-119` binds every shortcut with `cmd-`, and none with `secondary-` or `ctrl-`. In gpui's keystroke parser, `cmd`, `super` and `win` set the platform modifier. The sheet in `ui/src/keys.rs` `SHORTCUTS` still shows ⌘. Confirm on a Linux build.

## Expected outcomes

- [ ] ⌘L gives the sidebar focus, and ↓ moves the cursor by one visible row.
- [ ] → opens a closed channel, ← goes to the parent, Home goes to the first row and End goes to the last row.
- [ ] Enter on a session row opens it, and the sidebar cursor stays on that row.
- [ ] ⌥⌘→ opens every channel and fork tree, and ⌥⌘← folds them all, while the sidebar has focus.
- [ ] ⌘0 folds the sidebar to a strip. A second ⌘0 brings the sidebar back docked.
- [ ] ⌘1, ⌘2 and ⌘3 open Home, the Inbox and the Board.
- [ ] ⌘[ returns to the page the person drilled in from, and does nothing when the trail is empty.
- [ ] ⌘/ opens the shortcuts sheet, and Escape closes it.
- [ ] ⌘, opens Settings.
- [ ] ⌘N opens the New session sheet, and ⇧⌘N opens the New channel sheet.
- [ ] ⌥⌘M opens the palette asking for a machine, but only in a session.
- [ ] Escape with nothing to dismiss does nothing in the app, and reaches the terminal when the terminal has focus.
- [ ] ⌥⌘→ expands the groups of the Inbox, the Board and the diff when they have focus.
- [ ] On Linux, the same shortcuts work with Ctrl.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. ⌘L focuses the sidebar | Code, no test | `ui/src/keys.rs:107`; `ui/src/shell.rs:535-537` |
| 2. ↓ moves the cursor | Fakes only | `features/sidebar/tests/desk.rs::down_walks_from_the_first_desk_row_through_the_desk_into_the_channels` |
| 3. → ← Home End in the tree | Code, no test | `features/sidebar/src/lib.rs:297-347` |
| 4. Enter opens the row | Fakes only | `features/sidebar/tests/desk.rs::enter_on_a_desk_row_opens_that_session` |
| 5. ⌥⌘→ and ⌥⌘← | Code, no test | `ui/src/keys.rs:100-101`; `features/sidebar/src/lib.rs:186-209` |
| 6. ⌘0 folds the sidebar | Code, no test | `ui/src/shell.rs:515-521` |
| 7. ⌘1 ⌘2 ⌘3 | Code, no test | `ui/src/keys.rs:103-105`; `ui/src/shell.rs` (GoHome, GoInbox, GoBoard) |
| 8. ⌘[ goes back after a drill-in | Fakes only | `features/app/tests/navigation.rs::switching_screens_clears_the_drill_in_trail` |
| 9. ⌘/ opens the shortcuts sheet | Fakes only | `desktop/tests/native/overlays.rs::shortcuts_sheet_light` (action, not key) |
| 10. Escape closes the sheet | Code, no test | `features/app/src/overlay.rs` (`escape`) |
| 11. ⌘, opens Settings | Code, no test | `ui/src/keys.rs:106`; `ui/src/shell.rs:532-534` |
| 12. ⌘N opens New session | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` |
| 13. ⇧⌘N opens New channel | Fakes only | `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac` (action, not key) |
| 14. ⌥⌘M asks for a machine in a session | Code, no test | `ui/src/shell.rs:541-551` |
| Branch: ⌘[ with no history | Fakes only | `features/app/tests/navigation.rs::switching_screens_clears_the_drill_in_trail` |
| Branch: Esc, terminal focused | Code, no test | `terminal/view/src/view.rs:929`; `terminal/view/src/keys.rs:164` |
| Branch: Esc, nothing focused | Code, no test | `features/app/src/overlay.rs` (`escape`) |
| Branch: arrows at the ends | Fakes only | `features/sidebar/tests/desk.rs::up_from_the_first_desk_row_stays_there` |
| Branch: app key in terminal | Code, no test | `terminal/view/src/keys.rs:65`; `terminal/view/src/keys.rs:301` (intent only) |
| Branch: ⌘0 a second time | Likely bug | `ui/src/shell.rs:515-521` |
| Branch: ⌘L while hidden | Likely bug | `ui/src/shell.rs:535-537` |
| Branch: ⌥⌘→ outside the sidebar | Not built (NV-18, NV-19) | `ui/src/keys.rs:100-101` |
| Branch: ⌘[ after a sidebar open | Code, no test | `features/app/src/lib.rs:544-546`; `features/app/src/route.rs:104-109` |
| Branch: ⌥⌘M outside a session | Code, no test | `ui/src/shell.rs:541-551` |
| Branch: Linux keys | Likely bug | `ui/src/keys.rs:94-119` |

Steps: 1 Works (real daemon), 5 Fakes only, 8 Code no test, 0 Not built, 0 Likely bug (14 steps).
Branches: 2 Fakes only, 5 Code no test, 3 Likely bug, 1 Not built (11 branches).

## Gaps and likely bugs

- Likely bug: `ui/src/shell.rs:515-521`. A second ⌘0 with the sidebar hidden opens it as an overlay and does not dock it. The palette's `Toggle sidebar` does dock it.
- Likely bug: `ui/src/shell.rs:535-537`. ⌘L with the sidebar hidden focuses a handle that is not drawn. Confirm by pressing ⌘0, then ⌘L, then ↓.
- Likely bug: `ui/src/keys.rs:94-119`. Every shortcut is `cmd-`, which is Super on Linux. None has a Ctrl form, though the sheet and the palette show ⌘. The same applies to micro-21.
- Likely bug: `features/palette/src/lib.rs:673-674`. The palette shows `⌘4` for Hosts and `⌘5` for Files, but `ui/src/keys.rs` binds only ⌘1 to ⌘3. Confirm by pressing ⌘4 and ⌘5.
- Not built: NV-18 and NV-19. ⌥⌘→ and ⌥⌘← work only in the sidebar. The Inbox groups, the Board table's groups and the diff's repo groups have no binding.
- Gap: no test presses ⌘1 to ⌘3, ⌘, ⌘/, ⌘0, ⌘L, ⌥⌘→, ⌥⌘M or ⇧⌘N. The tests call the actions directly.
- Gap: the sidebar's tree moves (← → Home End) and Enter on a tree row have no test. Only Desk rows are covered.
- Gap: the per-step tags say "Code, no test" for ⌘0, ⌘L, ⌘1 to ⌘3, ⌘, and ⌥⌘M, because no test presses them. The keys are mapped, and the actions they send are simple, so these are low risk, but they are unproven.
- Gap: Esc with the terminal focused is decided by gpui's dispatch order, which is read from the gpui source, not run. A live test would confirm it.
