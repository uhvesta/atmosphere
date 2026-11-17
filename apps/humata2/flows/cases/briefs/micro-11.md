# Brief micro-11: Select and copy
Output: `apps/humata2/flows/cases/micro/11-select-and-copy.md`

Scenario: I drag to select a stack trace, double-click a word, triple-click a line, ⌥-drag a column block, copy with ⌘C (Ctrl+Shift+C on Linux), and paste it elsewhere. A program also writes to my clipboard with OSC 52.

Steps: a selection across a soft wrap copies as one line; wide characters copy whole; selection in scrollback works; ⌘C copies but ⌃C still interrupts; Edit menu Copy.

Branches: a program that has the mouse (selection needs ⇧); output arriving while I select (does the selection hold?); OSC 52 from a remote shell over SSH (lab clipboard_ssh.rs) — is it asked or allowed?; copying in a read-only attach.

Pointers: terminal/vt/tests/selection.rs, terminal/view/tests/view.rs (double/triple click, alt-drag block), terminal/pane/tests/pane.rs (double-click copies a whole path; mouse goes to the program unless Shift; clipboard writes reach the app), terminal/lab/tests/interactive.rs, terminal/lab/tests/clipboard_ssh.rs.
