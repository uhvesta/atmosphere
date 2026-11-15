# Humata 2 GUI spike (I2)

One binary, `humata2-gui-spike`, built on gpui-kit 0.9.1 (DECISIONS.md D8,
research/GUI.md). Its window has three screens, switched by the tab bar at
the top or chosen at start with `--screen`:

- **tree**: a sidebar of 200 channels with 24 sessions each, 5,000 rows when
  all are open, in a virtualised `uniform_list`. Which channels are open and
  which row is selected live in a tca-rust reducer (`src/tree.rs`); the view
  only sends it actions. Arrow keys move the selection, Right opens a channel
  or enters it, Left closes it or goes up to it. Expand all and Collapse all
  sit above the list.
- **diff**: one 20,000-line diff across three repositories with many files,
  about 8% of lines 180 to 420 characters long, in a virtualised `list` that
  measures every row. The Wrap button (or `--wrap off`) switches wrapping, so
  rows go from one height to many.
- **terminal**: a 200×60 cell grid drawn by Humata 2's own terminal view
  (`terminal/view`): one quad per run of background colour and one shaped
  text piece per run of style, with only the rows that changed drawn again.
  Output runs at 60 lines a second while Output is running (or with
  `--animate`). The fake output is written into a `FakeGrid`; libghostty-vt's
  terminal implements the same `Grid` trait.

It uses GPUI's own `uniform_list` for the tree rather than gpui-component's
Tree, because the Tree keeps its own expanded and selected state and the
spike's point is that the reducer owns it.

It builds with Bazel, once per engine (D57): `:spike` on ordinary GPUI and
`:spike_fast` on GPUI Fast.

## Build

```sh
bazel build -c opt //apps/humata2/spike/gui:spike //apps/humata2/spike/gui:spike_fast
```

The commands below run the binary with `bazel run -c opt`, which builds it
first. From the repository root, `S=//apps/humata2/spike/gui:spike` (or
`:spike_fast`).

## Run

```sh
bazel run -c opt $S                      # opens on the tree
bazel run -c opt $S -- --screen diff --wrap off
bazel run -c opt $S -- --screen terminal --animate
bazel run -c opt $S -- --exit-after-ms 3000 # quits after 3 s
bazel run -c opt $S -- --help
```

## Benchmarks

Each benchmark prints one JSON line on stdout and quits. Run them in the
optimised build (`-c opt`), with the window on screen and nothing else busy.

To compare the engines (D57), run each benchmark below with `S` set to
`//apps/humata2/spike/gui:spike` and then to
`//apps/humata2/spike/gui:spike_fast`. They open a window on screen. The same
cases measured headless, without the GPU or the display, are
`bazel run -c opt //apps/humata2/engine:frames_bench` and
`//apps/humata2/terminal/view:frame_bench` (each with a `_fast` twin); they
are no substitute for these.

### Scroll: frame times while scrolling

```sh
bazel run -c opt $S -- --screen tree --bench scroll
bazel run -c opt $S -- --screen diff --bench scroll
bazel run -c opt $S -- --screen diff --bench scroll --wrap off
bazel run -c opt $S -- --screen terminal --bench scroll --bench-secs 10
```

The tree opens every channel first, then scrolls all 5,000 rows. The diff
scrolls all 20,000 lines. Both move `--step-px` pixels a frame (default
200). The terminal does not scroll: it runs its output at 60 lines a second
for `--bench-secs` seconds and records every frame.

```json
{"bench":"scroll","screen":"diff","wrap":true,"step_px":200,"frames":2950,"elapsed_ms":49200.0,"refresh_ms":8.333,"p50_ms":8.33,"p95_ms":8.4,"p99_ms":9.1,"max_ms":16.6,"work":{"p50_ms":2.1,"p95_ms":3.0,"p99_ms":4.2,"max_ms":7.9}}
```

- `p50_ms` … `max_ms`: the time between frames. On a display that keeps up
  it sits at `refresh_ms`; a dropped frame shows as a multiple of it.
- `work`: from the start of a frame to the end of its paint, the part of the
  frame the spike itself spent. Compare it with `refresh_ms` to see the
  headroom left.
- `refresh_ms`: the display's refresh interval as GPUI reports it, or null.

The first 10 frames are left out, while the screen settles.

### Idle: CPU and memory while nothing happens

```sh
bazel run -c opt $S -- --screen tree --bench idle
bazel run -c opt $S -- --screen terminal --bench idle --idle-ms 10000
```

Waits `--idle-ms` (default 10,000) after the first paint, then reports:

```json
{"bench":"idle","screen":"tree","idle_ms":10001.2,"cpu_ms":3.1,"cpu_percent":0.031,"frames_painted":0,"max_rss_mb":142.5}
```

- `cpu_ms`, `cpu_percent`: user plus system CPU time the process used during
  the wait (`getrusage`), and that as a share of the wait.
- `frames_painted`: frames drawn during the wait; idle should draw none.
- `max_rss_mb`: the process's peak resident memory since it started.

Add `--animate` on the terminal to measure it running rather than idle.

### Start: time to the first frame

```sh
bazel run -c opt $S -- --screen tree --bench start
```

```json
{"bench":"start","screen":"tree","first_paint_ms":180.4,"next_frame_ms":188.9}
```

- `first_paint_ms`: from the start of `main` to the end of the first paint.
- `next_frame_ms`: to the start of the frame after it, by when the first
  frame has been handed to the display.

## Tests

```sh
bazel test //apps/humata2/spike/gui/...
```

`tests/tree.rs` opens the tree in a headless window (gpui-kit's
`test-support`), presses the arrow keys and clicks Expand all and Collapse
all, and checks the reducer's state and the rows drawn. The unit tests check
the reducer, the generated diff and grid, the percentiles and the command
line.
