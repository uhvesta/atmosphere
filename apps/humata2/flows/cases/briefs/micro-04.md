# Brief micro-04: Tab completion
Output: `apps/humata2/flows/cases/micro/04-tab-completion.md`

Scenario: I type `cd src/comp` and press Tab, then Tab twice on an ambiguous prefix, then Shift-Tab in a completion menu, then Tab on a remote shell over a slow SSH link.

Steps: single match completes; ambiguous double Tab lists; no match rings the bell (the app gets a bell event — check pane tests "titles, bells and clipboard writes reach the app"); Shift-Tab sends CSI Z; Tab never moves app focus away from the terminal (gpui focus traversal must not steal it).

Branches: Tab while the palette is open goes to the palette, not the shell; Tab in an unfocused terminal; Tab over the real SSH VM (terminal/lab/tests/*ssh*.rs); completion output that scrolls the view while I had scrolled up (view stays where I scrolled).

Pointers: terminal/vt/tests/input.rs (tab/shift-tab), terminal/pane/tests/pane.rs (bells, scroll holds), desktop/tests/native/terminal.rs (`an unfocused terminal gets no palette keystrokes`), ui/src/keys.rs (is tab bound anywhere?). Check whether any real-shell test presses Tab.
