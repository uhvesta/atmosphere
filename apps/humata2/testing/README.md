# Humata 2 test tooling

Tools that Humata 2's tests use. TESTING.md, one directory up, says what to
test. This file says how to run these tools.

- `snapshot/` (`humata2-snapshot`) records snapshots of three kinds and
  checks them later:
  - **state**: `StateRecorder` wraps a tca `TestStore` and writes each action
    with the state diff it caused.
  - **view**: `view::capture(window)` prints the accessibility tree of a
    GPUI window as an indented outline: role, label, value, state, and the
    text drawn. Only visible elements are included.
  - **image** (macOS only): `Snapshots::assert_image` compares pixels drawn
    off screen against a PNG, with a per-channel and per-pixel `Tolerance`.
- `flow/` (`humata2-flow`) plays a SPEC.md flow as a script of user
  actions (click, type, key, drag, paste). It snapshots the steps you mark
  and writes the whole run to `flow.md` for review. `GpuiDriver` plays a
  flow in a GPUI window; the `Driver` trait lets a later driver play the
  same flows against real daemons.
- `flow/tests/counter_list.rs` tries each kind of snapshot on a small
  counter-and-list view.
- `linux/run.sh` runs Bazel test targets in the Linux VM. It is not part
  of this Cargo workspace.

## Nothing is shown on screen

Every test here is headless. View snapshots and flows use GPUI's test
platform (`TestAppContext`), which draws nothing. Image snapshots use
`humata2_snapshot::headless_context()`, which draws with Metal into an
off-screen texture. It opens no window and doesn't take focus. Open
windows with `show: false` and `focus: false`, as the demo does.

## Running

From this directory:

```sh
cargo test                           # check every snapshot
HUMATA2_SNAPSHOT=record cargo test   # write snapshots that are missing or changed
```

Snapshots sit beside the test that takes them, in
`__snapshots__/<test file name>/<name>.txt` or `.png`. Commit them with the
test.

**The first run fails** because nothing is recorded yet. Run it once with
`HUMATA2_SNAPSHOT=record`, read every file it wrote, and commit them.

## When a snapshot fails

The failure message gives the snapshot's path and a diff. The files to
review go in `__snapshots__/<test>/.failed/`, which is already ignored by
git:

- text: `<name>.txt`, what the test saw;
- image: `<name>.old.png`, `<name>.new.png`, `<name>.diff.png` (changed
  pixels in red over a faded copy of the old image) and `<name>.review.png`
  (all three side by side).

If the change is intended, re-record and read the diff before committing:

```sh
HUMATA2_SNAPSHOT=record cargo test <test name>
git diff -- '**/__snapshots__/**'
```

Don't re-record without reading the diff. Recording accepts whatever the
code does now, so a regression recorded that way becomes the expected
result and no test will catch it.

## Flow artifacts

A flow writes its run to `<root>/<section>-<title>/flow.md`, plus PNGs when
the driver draws pixels. `<root>` is `HUMATA2_FLOW_OUT` when that is set.
Otherwise it is `target/tmp/humata2-flows`. A run deletes its own directory
before it starts. The artifact is written whether the flow passes or fails,
so you can read a failing run up to the step that broke. The test prints
where it wrote the artifact. Use `cargo test -- --nocapture` to see that
line.

Each step's view snapshot is named `<section>-<title>.<step>`, with `00`
for the start. Adding a step in the middle of a flow renames every
snapshot after it, so re-record the flow and review the diff.

## Platforms

State and view snapshots and flows run on macOS and Linux. Image snapshots
need Metal, so they build and run only on macOS. A flow played on Linux
skips its image checks.

## Bazel

These crates build with Cargo for now and move into Bazel together with
gpui-kit. Under `bazel test` the source tree is read-only, so record with
`bazel run` on the test target. The snapshot store finds the source tree
through `BUILD_WORKSPACE_DIRECTORY`.
