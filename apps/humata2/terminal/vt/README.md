# ghostty_vt

Safe Rust over `libghostty-vt`, the terminal core of `Ghostty` (decision D10).
It parses what a program writes and keeps the screen: the grid, its history,
cursor, title and working directory. It copies and searches text, prepares
pasted text, and turns keys and mouse events into the bytes a program reads.
It draws nothing and touches no clipboard: the `GPUI` view in `terminal/view`
reads the screen through the `Grid` trait from `terminal/grid` and draws it.

`Terminal` implements `Grid` over `libghostty-vt`'s render state
(`render.h`), as `Ghostty`'s own renderer does: `sync` captures the viewport
once a frame and copies only the rows that changed, and the view redraws
the rows `is_row_dirty` names.

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
    // Titles, the bell, replies to write back to the pty, and clipboard
    // requests (OSC 52) for the host to allow or refuse.
}
terminal.sync(); // then read rows, the cursor and colors through `Grid`
let bytes = terminal.paste(text, confirmed)?; // write to the pty
```

Clipboard requests are only ever raised, never carried out, because the
program may be on another machine: `Event::ClipboardWrite` carries the text
a program wants copied, and `Event::ClipboardReadRequested` waits for the
host to write `clipboard_read_reply` with the text, or with `None` to
refuse. `paste` refuses text with a line break unless the program asked for
bracketed paste or the user confirmed it.

A `Terminal` is `Send` but not `Sync`: keep it on one thread at a time.

## Checkpoints and passive replicas

`checkpoint(max_bytes)` returns an opaque binary snapshot of both screens,
scrollback, cursor, modes, colors, hyperlinks, and unfinished VT/UTF-8 input.
Its size is measured before allocating the returned buffer. Save the
`CHECKPOINT_FORMAT` identifier alongside the bytes, and restore with
`Terminal::from_checkpoint` or `restore_checkpoint`. Restoration installs
fresh callbacks and emits no historical clipboard requests, bells or PTY
replies. `restore_checkpoint` keeps the existing terminal if decoding fails.

The envelope is versioned and uses fixed-width little-endian dimensions;
the body is Ghostty's binary snapshot format. These bytes must never be
passed to `feed`. Feed only output after the checkpoint's host-owned output
offset. A passive client must suppress automatic PTY replies: the
authoritative host parses that same output and answers the program once.
The host also owns persistence policy, compression, atomic disk writes,
checkpoint quotas and authorization; this crate performs none of those IO
operations.

Continuation tracking is bounded to 1 MiB. A larger unfinished control
sequence can make checkpoint creation fail; retain the previous checkpoint
and retry when the sequence completes. `set_scrollback_limits` updates the
native line and byte budgets; eviction is page-granular.

Restore accepts at most 16 MiB of snapshot input. Every terminal grid is
limited to 4096 cells per dimension and 1,048,576 cells total, including
creation and resize. A restored terminal retains a 128 MiB custom-allocation
quota for its lifetime. Native Ghostty page storage uses the OS allocator
directly, so restoration also validates every embedded page capacity against
a conservative 256 MiB estimate before decoding. These are allocation
budgets, not a promise about total process memory. Corrupt embedded string
lengths cannot bypass the custom allocator by fitting in a small file.

Snapshots reset presentation state: selection, scrolled viewport, focus and
hover state. Keep any desired per-viewer selection coordinates and scroll
position separately, then apply them after restore against the restored grid.

## WebAssembly

`libghostty_vt_wasm.bzl` builds the same pinned engine for
`wasm32-freestanding`, verifies the C structure layout against its headers,
and exports a static archive for the Rust `wasm32-unknown-unknown` client.
The archive and Rust share one linear memory. Each allocator frees its own
allocations; the wrapper encodes checkpoints into Rust-owned buffers.
The browser renders through the existing GPUI terminal view. PTYs and SSH
remain in the native host.

`@libghostty_vt_wasm//:wasm_module` also exports Ghostty's standalone Wasm
module for the manual cross-architecture checkpoint test. Run its native
fixture and Node acceptance probe through Bazel:

```sh
bazel test //apps/humata2/terminal/vt:wasm_checkpoint_test \
  --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_NODE=/absolute/path/to/node --test_output=all
```

The probe restores a native 64-bit checkpoint in wasm32 repeatedly, resumes
split UTF-8 input, and checks alternate-screen and primary-history content.
It also encodes a new snapshot in wasm32 and restores it on the native host.

## If `libghostty-vt` falls short

The fallback is `alacritty_terminal` behind the same `Grid` trait and the
same `Terminal` methods, so the view doesn't change.
