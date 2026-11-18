# Humata 2 test tooling

Tools that Humata 2's tests use. TESTING.md, one directory up, says what to
test. This file says how to run these tools.

- `snapshot/` (`humata2-snapshot`) records snapshots of four kinds and
  checks them later. Snapshots are the data under what is drawn, as text,
  not pictures of it: cheaper to record and compare, and an agent or a
  reviewer can read them and their diffs directly. State, view and layout
  are text and are the default; take an image only for what text can't
  show.
  - **state**: `StateRecorder` wraps a tca `TestStore` and writes each action
    with the state diff it caused.
  - **view**: `view::capture(window)` prints the accessibility tree of a
    GPUI window as an indented outline: role, label, value, state, and the
    text drawn. Only visible elements are included. It says what the view
    shows and does, and is the default.
  - **layout**: `layout::capture(window)` prints the same window with where
    everything sits: each element, text, background quad and underline with
    its position and size in whole logical pixels, its background and
    border colours (`#rrggbb`, plus `aa` when translucent), border widths,
    corner radii and clipping, nested and in reading order.
    A border GPUI paints as several clipped copies is one line. Text sits
    under the element that holds its centre, so an input's text that
    overflows it stays under it. `Snapshots::assert_layout` checks it. Take
    one where layout is what the test proves, rather than an image; it
    changes with every layout tweak.
  - **image** (macOS only), the fallback for what only pixels show
    (glyphs, text colour and font, icons, shadows):
    `Snapshots::assert_image` compares pixels drawn off screen against a
    PNG, with a per-channel and per-pixel `Tolerance`.
- `flow/` (`humata2-flow`) plays a SPEC.md flow as a script of user
  actions (click, type, key, drag, scroll, paste). `.scroll(target, x, y)`
  uses pixel deltas; negative y moves down the content. It snapshots the steps you mark
  (`.snapshot()` for the view, `.snapshot_layout()` to add the layout,
  `.snapshot_image()` to add pixels) and writes the whole run to `flow.md`
  for review. `GpuiDriver` plays a flow in a GPUI window; the `Driver` trait lets a later driver play the
  same flows against real daemons.
- `flow/tests/counter_list.rs` tries each kind of snapshot on a small
  counter-and-list view.
- `linux/run.sh` runs Bazel test targets in the Linux VM: Bazel builds them for
  Linux here and runs each through `linux/in_vm.sh` (`--run_under`), so test
  env, args, runfiles and caching work as for any Bazel test. `linux/BATCH.md`
  is the runbook for the one VM batch.

## Nothing is shown on screen

Every test here is headless. View and layout snapshots and flows use
GPUI's test platform (`TestAppContext`), which draws nothing. Image snapshots use
`humata2_snapshot::headless_context()`, which draws with Metal into an
off-screen texture. It opens no window and doesn't take focus. Open
windows with `show: false` and `focus: false`, as the demo does.

## Running

Everything builds and runs through Bazel; don't run cargo.

```sh
bazel test //apps/humata2/testing/...                    # check every snapshot
bazel run //apps/humata2/testing/flow:counter_list_record # record one test's snapshots
```

Snapshots sit beside the test that takes them, in
`__snapshots__/<test file name>/`: `<name>.txt` for state and view,
`<name>.layout.txt` for layout, `<name>.png` for images. Commit them with
the test.

**The first run fails** because nothing is recorded yet. `bazel run` the
test's `_record` target once, read every file it wrote, and commit them.

## When a snapshot fails

The failure message gives the snapshot's path and a diff. What the test
produced goes to the test's undeclared outputs, in
`bazel-testlogs/<package>/<test>/test.outputs/`, at the snapshot's path
below the repository:

- text: `<name>.txt` or `<name>.layout.txt`, what the test saw;
- image: `<name>.old.png`, `<name>.new.png`, `<name>.diff.png` (changed
  pixels in red over a faded copy of the old image) and `<name>.review.png`
  (all three side by side).

A `_record` run that fails, or a store made outside Bazel, writes them to
`__snapshots__/<test>/.failed/` instead, which is already ignored by git.

If the change is intended, re-record and read the diff before committing:

```sh
bazel run //apps/humata2/diff:diff_view_test_record
git diff -- '**/__snapshots__/**'
```

Don't re-record without reading the diff. Recording accepts whatever the
code does now, so a regression recorded that way becomes the expected
result and no test will catch it.

## Flow artifacts

A flow writes its run to `<root>/<section>-<title>/flow.md`, plus PNGs when
the driver draws pixels. `<root>` is `HUMATA2_FLOW_OUT` when that is set.
Otherwise it is `humata2-flows` in the test's undeclared outputs under
`bazel test` (`bazel-testlogs/<package>/<test>/test.outputs/`). A run
deletes its own directory before it starts. The artifact is written
whether the flow passes or fails, so you can read a failing run up to the
step that broke. The test prints where it wrote the artifact; add
`--test_output=all` to see that line.

Each step's view snapshot is named `<section>-<title>.<step>`, with `00`
for the start; its layout is `<section>-<title>.<step>.layout.txt` and its
image `<section>-<title>.<step>.image.png`. Adding a step in the middle of a flow renames every
snapshot after it, so re-record the flow and review the diff.

## Platforms

State, view and layout snapshots and flows run on macOS and Linux. Image
snapshots need Metal, so they build and run only on macOS. A flow played on
Linux skips its image checks.

A layout depends on how text measures. `TestAppContext` measures every
glyph the same fixed width on every OS, so its layout snapshot is one file,
`<name>.layout.txt`, checked on macOS and Linux alike. A layout captured
with the platform's fonts, as in a `headless_context()` window, is kept per
OS, as `<name>.<os>.layout.txt` (`macos`, `linux`), the way images are.

What a layout snapshot can't show yet, because GPUI doesn't expose it to
tests: text colour, font and size; shadows, icons and images; and the
styles of elements that paint nothing. Image snapshots cover those.

## Bazel

Each crate is built once per GUI engine (D57): `:snapshot` and `:flow` on
ordinary GPUI, `:snapshot_fast` and `:flow_fast` on GPUI Fast, which turns
on the crates' `fast` feature. They are `testonly` and visible to the
Humata 2 UI packages (clients, desktop, diff, features, markdown, review,
terminal, ui).

- `//apps/humata2/testing/snapshot:snapshot`, `:snapshot_fast`
- `//apps/humata2/testing/flow:flow`, `:flow_fast`
- `humata2_snapshot_test` in `//apps/humata2/testing:defs.bzl`, for any
  test that takes snapshots.

`humata2_snapshot_test(name, srcs, deps, ...)` is a `rust_test` whose data
is the `__snapshots__/<test file stem>/` directory beside each source, and
it adds `<name>_record`, the same test with `HUMATA2_SNAPSHOT=record`,
tagged `manual`:

```python
load("//apps/humata2/testing:defs.bzl", "humata2_snapshot_test")

humata2_snapshot_test(
    name = "diff_view_test",
    srcs = ["tests/diff_view.rs"],
    deps = ["//apps/humata2/testing/flow", "//apps/humata2/testing/snapshot"],
)
```

```sh
bazel test //apps/humata2/diff:diff_view_test          # check
bazel run //apps/humata2/diff:diff_view_test_record    # record into the source tree
git diff -- '**/__snapshots__/**'                      # read before committing
```

Under `bazel test` the store reads the snapshots from the runfiles and
refuses to record, as the source tree is read-only there. A failed check
writes its `.failed` files, and a flow its artifacts, to the test's
undeclared outputs, `bazel-testlogs/<package>/<name>/test.outputs/`.
Under `bazel run` the store finds the source tree through
`BUILD_WORKSPACE_DIRECTORY` and writes there.
`flow/BUILD.bazel` uses the macro for `counter_list` and `counter_list_fast`.

## Engines and frames

The `_fast` targets run the same tests on GPUI Fast. A driver
made with `GpuiDriver::new` draws each step with a full refresh, which
builds every view again. `GpuiDriver::with_frames(cx, window,
FrameMode::Natural)` draws the frames the app would draw itself and checks
each against a full refresh, so a view that changed without `cx.notify()`
fails with `GpuiDriverError::Stale` on GPUI Fast, which reuses views that
weren't notified (D58). `flow/tests/natural_frames.rs` shows both.
