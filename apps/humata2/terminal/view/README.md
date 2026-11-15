# terminal_view

Humata 2's terminal view (decision D22): a `GPUI` view that draws any
`terminal_grid::Grid` and sends what the user does to a `TerminalHost`.
It's written for Humata rather than taken from a third-party widget, and
copies `Ghostty`'s renderer where it can.

## How a frame is drawn

1. `Grid::sync` refreshes the engine's snapshot of the viewport.
2. Only the rows the grid marks dirty are planned and shaped again, as
   `Ghostty`'s `renderer/generic.zig` (`rebuildCells`, `rebuildRow`) does.
   A new size, theme or `Dirty::Full` redraws every row.
3. Planning a row (`frame.rs`) is plain data: one fill per run of
   background, one text piece per run of style, and the underline,
   strikethrough and overline as quads. Pieces break where `Ghostty`'s run
   iterator (`font/shaper/run.zig`) breaks: on a change of style, at the
   selection's edges, and around wide characters, multi-scalar clusters and
   right-to-left letters.
4. Each piece is shaped once with the cell width forced, so glyphs stay on
   the grid, and kept until its row changes.
5. The cursor (`cursor.rs`) follows `renderer/cursor.zig`: block, bar,
   underline, an outlined block while the view isn't focused, and a 600 ms
   blink that restarts on typing.

`TerminalView::stats` says how many rows and pieces the last frame rebuilt,
which the tests and the `frame` bench use.

## Input

The view never writes to a pty. It turns window events into
`TerminalHost` calls, so the same view serves a local shell, a remote
session over gRPC or a test:

- Keys go to `TerminalHost::key` as `terminal_grid::KeyEvent`s with the
  typed text, except the copy and paste shortcuts (Cmd+C and Cmd+V on a
  Mac; Ctrl+Shift+C, Ctrl+Shift+V and Shift+Insert elsewhere). Other Cmd
  shortcuts are left to the app.
- Paste reads the clipboard into a `Paste`: files first, then an image
  unless real text came with it, then text. The host decides what an image
  or file paste does.
- A left-button drag selects cells through `TerminalHost::select`; a click
  alone clears the selection. A program that takes the mouse gets it
  through `TerminalHost::mouse`, unless Shift is held.
- Scrolling adds up to whole rows for `TerminalHost::scroll`.
- The view asks `TerminalHost::resize` once for each new size that fills
  it.

Text input through an input method (composing) isn't wired yet.

## In a browser

The view builds for `wasm32-unknown-unknown`, so the app can run on
`gpui_web`. It reads no clock, file, process, thread or socket: the cursor
blink is a timer on `GPUI`'s executor, the clipboard goes through `GPUI`,
and copy and paste shortcuts follow the system `GPUI` reports, so a browser
on a Mac gets Cmd+C and Cmd+V. It reaches the engine only through `Grid`,
so a `wasm` build of `libghostty-vt`, or grid diffs streamed from the
holder, can stand in for `ghostty_vt` without changing the view.

## Tests

`tests/view.rs` drives the view in a headless window with real input and a
`FakeGrid`; `frame.rs`, `cursor.rs`, `keys.rs` and `paste.rs` test their
planning as data. `benches/frame.rs` times 200×60 frames with the
platform's real text system.
