# Brief micro-12: Open links
Output: `apps/humata2/flows/cases/micro/12-open-links.md`

Scenario: a test runner prints a URL and a file path with line number. I ⌘-click (Ctrl-click on Linux) the URL, plain-click it (must not open), double-click the path to copy it, and ⌘-click an OSC 8 hyperlink whose text differs from its target.

Branches: a link to `file://` or a path on a remote box (what opens?); a link with an odd scheme (refused or asked?); a link split across a wrap; link in a TUI with the mouse captured.

Pointers: terminal/pane/tests/links.rs (a modifier-click opens a link), terminal/view/src/view.rs (Cmd/Ctrl-click), TERM milestone batch 2 `terminal_link` (workflow/milestones/TERM.md), terminal/pane/tests/pane.rs (double-click copies a whole path). Check how the app opens a URL (grep ui/src for open_url) and whether that path is tested.
