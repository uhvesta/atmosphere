# Brief micro-14: Mouse in full-screen programs
Output: `apps/humata2/flows/cases/micro/14-mouse-in-full-screen-programs.md`

Scenario: in htop and in vim with `mouse=a` I click, drag and wheel; I hold ⇧ to select text instead; in `less` (alternate screen, no mouse) the wheel scrolls the file; I switch apps and the program hears focus out/in.

Steps: mouse tracking sends SGR mouse reports; ⇧ overrides capture; alternate screen with AlternateScroll turns wheel into arrows; otherwise the viewport scrolls; focus reports (CSI I / CSI O) only when the program asked.

Branches: mouse in a read-only attach (nothing sent); wheel over the scrollbar; a program that crashes and leaves mouse mode on (does the shell get garbage? does the pane reset?).

Pointers: terminal/pane/src/host.rs (wheel handling), terminal/vt/tests/input.rs (mouse SGR), terminal/pane/tests/pane.rs (program takes the mouse unless Shift; alternate-screen wheel sends arrows; focus reports to the program), terminal/lab/tests/neovim.rs `neovim_colors_mouse_keyboard_resize_and_resume` (real Neovim).
