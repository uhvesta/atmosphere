# Brief micro-15: vim and less
Output: `apps/humata2/flows/cases/micro/15-vim-and-less.md`

Scenario: I open a file in Neovim, edit with insert mode and Esc, resize the window, ⌃Z out to the shell and `fg` back, :wq, then `less` a log, search with /, and q out.

Steps: colors (truecolor) render; Esc reaches vim immediately (no app binding steals Esc while the terminal is focused — check ui/src/keys.rs Dismiss on escape and its context); after quitting, the shell screen is exactly as before (alternate screen restored).

Branches: Esc when an app overlay is open (closes overlay, not vim); ⌘S in vim on Mac (ignored by the terminal; does the app bind it?); vim over the SSH VM; the app restarts while vim is open (replay redraws it).

Pointers: terminal/lab/tests/neovim.rs, terminal/pane/tests/pane.rs (a full-screen program leaves the shell as it was; a hidden pane catches up when shown), ui/src/keys.rs (escape binding context), daemon/humatad/tests/terminal.rs (replay).
