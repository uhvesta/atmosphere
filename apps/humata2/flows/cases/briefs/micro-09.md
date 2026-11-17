# Brief micro-09: Resize and split
Output: `apps/humata2/flows/cases/micro/09-resize-and-split.md`

Scenario: I drag the window narrower while `htop` runs, split the session with ⌘\ (Ctrl+Shift+\ on Linux) or the plus menu, try a third split, then resize vertically over SSH.

Steps: the program hears the resize (SIGWINCH) and redraws; long lines rewrap; the split opens a second group; a third split is refused (two groups max); joining keeps the program running.

Branches: two viewers of different sizes attached to one holder (who decides the size? terminal/client FitPolicy); a resize during a big output burst; a vertical resize over the real SSH VM.

Pointers: terminal/pane/tests/pane.rs (resizing tells the program), terminal/pane/src/pane.rs (split shortcut), features/session/tests/tabs.rs (split limited to two groups), desktop/tests/native/session.rs `the_plus_menu_splits_once_and_joining_keeps_the_program_running`, desktop/tests/native/terminal.rs (clicking a split focuses that group), terminal/lab/tests/vertical_resize_ssh.rs, terminal/client/src/replica.rs (FitPolicy).
