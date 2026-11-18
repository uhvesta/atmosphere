# Navigation: keyboard shortcuts, the Shortcuts sheet and the menu bar

Layer: explored · macOS · harness: desktop/tests/live_flows/navigation_keys.rs, live_flows/tabs_and_palette.rs, live_flows/keys.rs, native/overlays.rs, report.rs (phase 1 maps from code; phase 2 fills PENDING RUN)

## Setup

Phase 1 (this record): mapped from `apps/humata2/ui/src/keys.rs` (bindings, labels, the Shortcuts list), `ui/src/shell.rs` (where each action is handled), `ui/src/overlay_shortcuts.rs`, `desktop/src/main.rs` `menus()` (lines 235-304), `features/app/src/{overlay,route,commands}.rs` (what each action does), and the tests named in each step. Snapshot evidence: `apps/humata2/desktop/tests/__snapshots__/native/`.

Phase 2 (after the coach says go): the focused commands in the PENDING RUN lines, from the repo root, one explorer at a time. The live targets start a real `humatad2` (tag `exclusive`, size large). Evidence goes to `<scratchpad>/verify/evidence/X-J/` (test logs copied from `bazel-testlogs/apps/humata2/desktop/<target>/test.log`). No window is shown: the rigs use headless windows.

Key names: `app_key(k)` is ⌘k on macOS (`live_flows.rs:115`). Off a Mac every ⌘ chord moves to Ctrl+Shift (`keys.rs` `off_mac`). These steps are macOS only.

## Steps

### 1. ⌘L: the sidebar takes focus, with no cursor yet
- **Screen:** the sidebar column with the Sidebar navigation landmark. Focus and the keyboard cursor are not in any snapshot (the snapshot shows the current route, not the cursor).
- **Offered:** ↑ ↓ Home End → Enter (sidebar context only, `keys.rs:216-222`); ⌘L again keeps focus. If the sidebar is docked-hidden, ⌘L shows it first; if it is a narrow-window overlay, ⌘L opens the overlay (`shell.rs:569-582`).
- **Action:** press ⌘L with the sidebar shown.
- **Result:** focus moves to the sidebar. `state.sidebar.cursor` stays `None`, so Enter does nothing until ↓ or ↑ picks the first row (`features/sidebar/src/lib.rs` `step`, the `(Move::Down|Up|In|Out, None)` arm).
- **Evidence:** PENDING RUN.
- **PENDING RUN:** `export PATH=$PWD/.tools/bin:$PATH; bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=the_keyboard_walks_the_sidebar`
- **Replay check:** the same target and filter; the test asserts `cursor == None` after ⌘L, then `Some(0)` after the first ↓.

### 2. ↓ ↑ Home End: move the sidebar cursor over selectable rows
- **Screen:** the sidebar tree with a cursor on one row. Selectable rows are channels, sessions, forks and the Desk rows shown on the Desk page. Section titles, Home, Inbox, Board, Hosts, machines and Settings are not in the walk (see sidebar record, surprise 1).
- **Offered:** ↑, ↓, Home (first), End (last).
- **Action:** ↓ twice, ↑ at the first row, ↓ at the last row, End, Home.
- **Result:** ↑ on the first row and ↓ on the last stay put. Home and End jump to the ends.
- **Evidence:** PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, the asserts after the "↑ on the first row stays there" and "↓ on the last row stays there" lines.

### 3. ← and →: fold, unfold, go to parent, go to first child
- **Screen:** the channel tree with the cursor on a channel that has sessions.
- **Offered:** ← (fold the open channel; on a child go to the parent), → (unfold a closed channel, keep the cursor; on an open channel go to its first child).
- **Action:** ← on the channel, → on it, → again, ← again.
- **Result:** folding keeps the cursor on the channel, and the channel's sessions leave the row list. → on a folded channel opens it. → on an open channel goes to child depth 1. ← on a child goes to the parent and does not fold it.
- **Evidence:** PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, the block under "3. ← folds the open channel".

### 4. Enter on a session row: open it, cursor stays, no trail
- **Screen:** the session page after Enter. The sidebar row is highlighted as the current row (`snapshot 16-sidebar-fork-selected`, `Partial BRIN on posted_at selected`, shows the highlight).
- **Offered:** Enter (sidebar context), ⌘[ (Back).
- **Action:** walk to the second session, press Enter, press ⌘[.
- **Result:** route is the session; the cursor stays on that row and is also the current row. `trail` is empty, so ⌘[ does nothing and the route stays on the session (`switch` clears the trail, `route.rs:101`).
- **Evidence:** existing snapshot `apps/humata2/desktop/tests/__snapshots__/native/16-sidebar-fork-selected.view.txt` for the highlight; the navigation itself is PENDING RUN.
- **PENDING RUN:** same command as step 1.
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "4. Enter on a session row".

### 5. ⌥⌘← and ⌥⌘→: fold all, open all (sidebar context only)
- **Screen:** the sidebar tree, all channels folded and then all open.
- **Offered:** ⌥⌘← (CollapseAll), ⌥⌘→ (ExpandAll). Both are bound with the sidebar context (`keys.rs:200-201`), but the shell handles them with no predicate (`shell.rs:541-546`), so they act from anywhere in the app.
- **Action:** press ⌥⌘← then ⌥⌘→.
- **Result:** CollapseAll folds every channel and every session that has forks. ExpandAll clears all folding, sessions included (`lib.rs` `ExpandAll` sets `Folding::default()`).
- **Evidence:** existing snapshot `17-sidebar-channels-folded.view.txt` shows the folded result; the key presses are PENDING RUN.
- **PENDING RUN:** same command as step 1.
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "5. ⌥⌘← folds every channel".

### 6. ⌘1 ⌘2 ⌘3: Home, Inbox, Board, with no trail
- **Screen:** Home (`snapshot 25-home`), the global Inbox (`26-inbox`), the global Board (`21-board-kanban`).
- **Offered:** ⌘1, ⌘2, ⌘3; they also appear in the View menu (Home, Inbox, Board) and the palette (Go to Home/Inbox/Board).
- **Action:** press ⌘1, ⌘2, ⌘3, ⌘1 in turn.
- **Result:** each press calls `Switch`, which clears the back trail (`route.rs:101-106`). The route becomes Home, Inbox, `Board(Global)`.
- **Evidence:** PENDING RUN (same command as step 1). Snapshot pages `25-home.view.txt`, `26-inbox.view.txt`, `21-board-kanban.view.txt` show the screens.
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "7. ⌘1, ⌘2 and ⌘3".

### 7. ⌘4 and ⌘5: Hosts and Files
- **Screen:** Hosts (`28-hosts.view.txt`) and Files (the file manager page).
- **Offered:** ⌘4 (`GoHosts`), ⌘5 (`GoFiles`). The palette lists them as "Go to Hosts ⌘4" and "Go to Files ⌘5" (`palette/src/lib.rs:742-743`).
- **Action:** press ⌘4, then ⌘5.
- **Result:** `Switch(Route::Hosts)` and `Switch(Route::Files)` (`shell.rs:583-588`). Neither key is in the Shortcuts sheet (`keys.rs` SHORTCUTS) and neither is in the View menu.
- **Evidence:** PENDING RUN is not needed for the mapping; no headless test presses these keys. Snapshot `28-hosts.view.txt` shows the Hosts page.
- **Replay check:** none. The coach should treat this step as code-only until a test presses ⌘4 or ⌘5 (no run is planned for it).

### 8. ⌘[: back along the drill-in trail
- **Screen:** a session opened from Home by a card (drill-in), so the trail holds Home.
- **Offered:** ⌘[ (Back), which is also in the palette as "Go back ⌘[".
- **Action:** from Home, drill into a session, then ⌘[. Also press ⌘[ with an empty trail.
- **Result:** the drill-in pushes Home onto `trail` (`navigate`, `route.rs:89-98`); ⌘[ pops it and the route is Home with an empty trail. With an empty trail, ⌘[ does nothing.
- **Evidence:** PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, blocks "8. A drill-in from Home" and the first ⌘[ at the start of the test.

### 9. Escape with nothing to close: nothing changes
- **Screen:** Home, with no overlay open and the sidebar not focused.
- **Offered:** Escape (`Dismiss`, bound with no context, `keys.rs:198`).
- **Action:** press Escape at Home, and again with the sidebar focused.
- **Result:** `escape()` (`features/app/src/overlay.rs:52`) walks its list of open things and returns with no effect when none is open. The route, trail and overlays are unchanged.
- **Evidence:** PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, the first Escape block and the "Escape with the sidebar focused" block.

### 10. ⌘0: hide the sidebar; again: dock it; ⌘L shows it
- **Screen:** the sidebar docked, then hidden (`snapshot 13-columns-collapsed.view.txt`: `Button "Show sidebar"`, `Button "Show side panel"`).
- **Offered:** ⌘0 (`ToggleSidebar`, also "Toggle sidebar ⌘0" in the palette and "Show or Hide Sidebar" in the View menu); the sidebar's own "Hide sidebar" button.
- **Action:** ⌘0 twice; then ⌘0 to hide and ⌘L to show and focus.
- **Result:** a folded sidebar in a wide window docks again on the second ⌘0; it is not drawn over the page (`navigation_overlay` stays false). ⌘L with it hidden shows it, then ↓ moves the cursor.
- **Evidence:** snapshot `13-columns-collapsed.view.txt` for the hidden state; the presses are PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "6. ⌘0 folds the sidebar"; and `features/app:key_bugs_test --test_filter=a_sidebar_folded_in_a_wide_window_docks_again` (no run planned; code-listed).

### 11. ⌘/: the Keyboard shortcuts sheet, closed by Escape or its Close button
- **Screen:** a sheet titled "Keyboard shortcuts" over the page: a list of `SHORTCUTS` rows (key label, then what it does), and a Close button. Snapshot: `apps/humata2/desktop/tests/__snapshots__/native/shortcuts-sheet-light.view.txt` shows 15 rows; `shortcuts-sheet-scrolled-light` shows the end.
- **Offered:** scroll the list; Close (AppAction::Escape); Escape. ⌘/ again does nothing (`show_shortcuts` only pushes when the top overlay is not already Shortcuts).
- **Action:** ⌘/, then Escape.
- **Result:** the sheet is pushed; Escape pops it and leaves the page where it was (`live_flows_test` asserts route stays Home).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/shortcuts-sheet-light.view.txt` and `shortcuts-sheet-scrolled-light.view.txt`.
- **Replay check:** `native_test --test_filter=shortcuts_sheet` (phase 2, see the PENDING RUN list) and `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "9 and 10".

### 12. ⌘,: Settings, with no trail
- **Screen:** Settings (`29-settings.view.txt`).
- **Offered:** ⌘, (`OpenSettings`, `shell.rs:566`), the Humata menu "Settings…", and the palette "Open Settings ⌘," and "Settings › <section>" rows.
- **Action:** press ⌘, then ⌘1.
- **Result:** `Switch(Route::Settings)`, so the trail is cleared. ⌘1 goes back to Home.
- **Evidence:** snapshot `29-settings.view.txt`; the presses are PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "11. ⌘, opens Settings with no trail".

### 13. ⇧⌘N: the New channel sheet; Escape closes it
- **Screen:** the New channel sheet (`snapshot 44-new-channel.view.txt` in `native/`).
- **Offered:** ⇧⌘N (`NewChannel`), File > New Channel, palette "New channel… ⇧⌘N", the sidebar's "New channel" button; Escape closes it.
- **Action:** press ⇧⌘N, then Escape.
- **Result:** the top overlay is `NewChannel`; Escape pops it.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/44-new-channel.view.txt`; presses PENDING RUN (same command as step 1).
- **Replay check:** `live_flows_test --test_filter=the_keyboard_walks_the_sidebar`, block "13. ⇧⌘N opens the New channel sheet".

### 14. ⌥⌘0: the side panel opens Changes, then hides and shows
- **Screen:** a session with its side panel (`snapshot 09-session-no-panel.view.txt` shows the no-panel state; `15-sidebar-pinned` shows the panel open).
- **Offered:** ⌥⌘0 (`TogglePanel`, `shell.rs:603`), the palette "Toggle side panel ⌥⌘0", the session panel's Toggle side panel button.
- **Action:** with no panel, ⌥⌘0; then ⌥⌘0 again; then open a second session and back.
- **Result:** with no panel, ⌥⌘0 opens Changes and shows it; a further press hides it; each session keeps its own collapsed state.
- **Evidence:** PENDING RUN (`tabs_and_palette`, see step 16 for the command).
- **Replay check:** `live_flows_test --test_filter=the_tab_row_opens_and_closes`, blocks "8, and ⌥⌘0 with no panel".

### 15. ⇧⌘↩: terminal full screen, and how to leave it
- **Screen:** the session with the terminal full screen (`snapshot 10-terminal-full-screen.view.txt`).
- **Offered:** ⇧⌘↩ (`TerminalFullScreen`, only on a session route), the "Expand terminal" button, the palette "Terminal full screen ⇧⌘↩", and "Return to columns" when full. Escape does not leave full screen (`escape()` has no full-screen branch, `overlay.rs:52-170`).
- **Action:** ⇧⌘↩, ⇧⌘↩, then "Expand terminal", then "Return to columns".
- **Result:** each of the key and the button toggles `FullScreen::Terminal` on and off. The palette shows the same entry only inside a session.
- **Evidence:** snapshot `10-terminal-full-screen.view.txt` for the full-screen layout; presses PENDING RUN (`tabs_and_palette`).
- **Replay check:** `live_flows_test --test_filter=the_tab_row_opens_and_closes`, block "⇧⌘↩ enters terminal full screen and leaves it".

### 16. ⌥⌘M: move the open session to another machine
- **Screen:** with a session open, the palette opens already asking "Move to which machine?" (see palette record). With no session open, a toast says "Nothing to move: open a session, then press ⌥⌘M to move it to another machine." (`commands.rs`, `Command::MoveSession`.)
- **Offered:** ⌥⌘M (`MoveSession`), the palette "Move session to another machine… ⌥⌘M" (only when the session is live).
- **Action:** press ⌥⌘M with a session open, pick a machine. Then press it with no session open.
- **Result:** inside a session the key opens the palette and begins `MoveSession`; outside one, a toast says so and nothing moves.
- **Evidence:** PENDING RUN (`keys` target).
- **PENDING RUN:** `export PATH=$PWD/.tools/bin:$PATH; bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=the_move_key_asks_for_a_machine`
- **Replay check:** the same target and filter; the test's asserts on the palette asking and the toast text.

### 17. ⇧⌘F: fork the open session
- **Screen:** the fork dialog (`snapshot fork-dialog.view.txt`, native `fork-dialog`) once the fork starts.
- **Offered:** ⇧⌘F (`ForkSession`), the palette "Fork “<title>” ⇧⌘F" (not offered on a Desk session), the header Fork button.
- **Action:** with a channel session open, press ⇧⌘F.
- **Result:** runs the palette's Fork command (`shell.rs:594-602`). A Desk session has no fork (`desk.rs` `fork_unless_desk`), so the key does nothing there.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/fork-dialog.view.txt` (existing, opened by the fork flow); the key press is not in any test. Code-only.
- **Replay check:** none exact. The coach should treat this step as code-only.

### 18. ⇧⌘R and ⇧⌘S: bug report recording and snapshot
- **Screen:** a red "Stop" dot at the top right while recording (`report_view.rs` `dot`); the bug report form after a snapshot or a stop (see overlays record, steps 16-19).
- **Offered:** ⇧⌘R (`ToggleRecording`), ⇧⌘S (`TakeSnapshot`), the Help > Report a Bug menu (Start or Stop Recording, Take Snapshot), palette "Start recording a bug report ⇧⌘R", "Take a snapshot for a bug report ⇧⌘S".
- **Action:** ⇧⌘S with nothing recording.
- **Result:** the report form opens headless with the snapshot (`report_test a_snapshot_opens_the_form`).
- **Evidence:** PENDING RUN (`report_test`).
- **PENDING RUN:** `export PATH=$PWD/.tools/bin:$PATH; bazel test --jobs=4 //apps/humata2/desktop:report_test --test_filter=a_snapshot_opens_the_form`
- **Replay check:** the same target and filter.

## Not reachable headless

- **The menu bar** (`desktop/src/main.rs` `menus()`, lines 235-304): menus are set with `cx.set_menus`, and no harness drives the NSMenu. What each menu holds, from the code:
  - Humata: Settings…, Services, Hide Humata (⌘H), Hide Others (⌥⌘H), Show All, Quit Humata (⌘Q).
  - File: New Session (⌘N), New Channel (⇧⌘N).
  - Edit: Undo, Redo, Cut, Copy, Paste, Select All (OS actions to the text input).
  - View: Command Palette (⌘K), Show or Hide Sidebar (⌘0), Home (⌘1), Inbox (⌘2), Board (⌘3).
  - Window: Minimize (⌘M), Zoom.
  - Help: Keyboard Shortcuts (⌘/), Report a Bug > Start or Stop Recording (⇧⌘R), Take Snapshot (⇧⌘S).
- **Menu key equivalents** are drawn by the OS from the same bindings; not checked.
- **Escape inside a terminal** (vim, less): the shell's Dismiss is bound app-wide and the terminal propagates it unless its find bar is open (`terminals.rs:736`). Whether a running program gets Escape is a terminal-case question; not run here.
- **Tab order** between sidebar, page and sheet: not in any harness.

## Surprises

1. **The Shortcuts sheet is missing four shortcuts that work.** `SHORTCUTS` (`keys.rs:76-96`) has 19 rows and omits ⌘4 (Hosts), ⌘5 (Files), ⇧⌘↩ (terminal full screen) and ⇧⌘F (fork). The palette does show all four (`palette/src/lib.rs:742-743, 770, 797`).
2. **The View menu also omits them**, and the Hosts, Files, Desk and Views pages, Expand all, Collapse all and Toggle side panel (⌥⌘0) are in no menu. Help > Report a Bug has no "Report a bug…" entry (the form is in the palette only).
3. **⌘1 and the palette's "Go to Home" leave different back trails.** The key is `Switch` (it clears the trail, `shell.rs:557`, `route.rs:101`). The palette's `Command::Home` calls `navigate` (`commands.rs`), which pushes the old page onto the trail. The same split holds for Inbox, Board, Hosts, Files, Desk, Views and Settings (⌘, is `Switch`). So ⌘[ after a palette jump goes back, and after a key press it does nothing.
4. **Escape does not leave terminal full screen.** `escape()` has no branch for `FullScreen`. Leaving needs ⇧⌘↩ or "Return to columns".
5. **⌥⌘→ and ⌥⌘← are bound with a sidebar context in `keys.rs`, but the shell handles them with no context** (`shell.rs:541-546`). They work from any page. The sheet still lists them as "in the sidebar". Facts only.
6. **Escape is the app's Dismiss everywhere.** The code says "a text box included" (`terminals.rs:736`). Text fields that use Escape for their own purpose are not checked here.
