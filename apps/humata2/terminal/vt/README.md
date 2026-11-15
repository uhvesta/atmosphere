# ghostty_vt

Safe Rust over `libghostty-vt`, the terminal core of `Ghostty` (decision D10).
It parses what a program writes and keeps the screen: the grid, its history,
cursor, title and working directory. It copies and searches text, and turns
keys and mouse events into the bytes a program reads. It draws nothing: the
`GPUI` view in `terminal/view` reads the grid through the `Grid` trait and
draws it.

## How it's built

`libghostty_vt.bzl` downloads a pinned `Zig` and the pinned `libghostty-vt`
source, then builds a static library for each platform:

- macOS on Apple silicon
- Linux on `aarch64` and `x86_64`, against `musl`

Other hosts get a stub that's incompatible with every platform, so targets
that depend on it are skipped there. A macOS build needs the Xcode command
line tools.

To move to a new release, change `_VERSION` and `_SHA256` together, and
`_ZIG_VERSION` and `_ZIG_SHA256` if the release needs another `Zig`.

## Using it

```rust
let mut terminal = ghostty_vt::Terminal::new(80, 24, 10_000)?;
terminal.feed(bytes_from_the_pty);
for event in terminal.drain_events() {
    // Titles, the bell, and replies to write back to the pty.
}
let row = terminal.viewport_row(0)?;
```

A `Terminal` is `Send` but not `Sync`: keep it on one thread at a time.

## If `libghostty-vt` falls short

The fallback is `alacritty_terminal` behind the same `Grid` trait and the
same `Terminal` methods, so the view doesn't change.
