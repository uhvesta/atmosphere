# Brief micro-22: Keyboard navigation
Output: `apps/humata2/flows/cases/micro/22-keyboard-navigation.md`

Scenario: I never touch the mouse for ten minutes: ⌘L to the sidebar, arrows and Enter to open a session, ⌥⌘→ expand all, ⌥⌘← collapse all, ⌘0 hide the sidebar, ⌘1/⌘2/⌘3 Home/Inbox/Board, ⌘[ back, ⌘/ the shortcuts sheet, ⌘, Settings, Esc dismiss, ⌘N New session, ⇧⌘N New channel, ⌥⌘M Move session.

Branches: ⌘[ with no history; Esc with nothing to dismiss (must not reach the terminal as a stray ESC? check); arrows in the sidebar at the ends (Home/End); a key pressed while the terminal is focused that the app also binds (which wins?); Linux equivalents (check ui/src/keys.rs for non-mac bindings).

Pointers: ui/src/keys.rs (`pub fn bind`), desktop/src/main.rs (menus), features/app/tests/navigation.rs, features/sidebar/tests/*, desktop/tests/native/*, desktop/tests/design (shortcuts sheet snapshot `shortcuts_sheet_scrolled_light`). SPEC NV-* IDs (grep `\*\*NV-` in SPEC.md).
