# Brief micro-23: Session tabs and splits
Output: `apps/humata2/flows/cases/micro/23-session-tabs-and-splits.md`

Scenario: in a session I open a second shell tab from the plus menu, open a file in the tab row, split, join, close a tab, toggle the panel with ⌥⌘0, and try ⌘T, ⌘W, ⌃Tab and ⇧⌘↩ (terminal full screen) out of habit.

Coach findings to verify:
- ui/src/keys.rs has no cmd-w, cmd-t, ctrl-tab or shift-cmd-enter bindings; desktop/src/main.rs binds only cmd-q, cmd-h, alt-cmd-h, cmd-m plus menus. SPEC NV-37 says ⌘W closes the active tab and ⇧⌘↩ is Terminal full screen. So these are Not built (NV-37) or a Likely bug.
- terminal/pane/src/tabs.rs has a TabStrip with ⌘T, ⌘W, ⌘⇧], ⌘⇧[, ⌃Tab and ⌘1–9, and terminal/client/src/replica.rs has opt-in `register_host_shortcuts` (cmd-t, cmd-k, ctrl-tab, ctrl-shift-tab). Check whether the app calls register_host_shortcuts or uses TabStrip (grep ui/src and desktop/src). Coach believes it does not.
- ui/src/session_tabs.rs: SessionAction::CloseTab is mouse-driven.
- FullScreen::Terminal exists in ui/src/shell.rs; find how it is reached.

Branches: closing the last tab; closing a tab whose shell is running (asked?); a third split refused; files open in the tab row and close back to the terminal.

Pointers: features/session/tests/tabs.rs, desktop/tests/native/session.rs (`the_plus_menu_splits_once_and_joining_keeps_the_program_running`, `files_open_in_the_tab_row_and_close_back_to_the_terminal`, `terminal_tabs_show_only_the_selected_terminals_live_output`, `session_lifecycle_and_full_screen_reference_images`), terminal/pane/tests/tabs.rs, SPEC NV-37.
