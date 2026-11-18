# The Linux VM batch

The LINUX milestone's exit run: every non-image suite, the view-tree
snapshots, the launch smoke test (D129), real-SSH machine flows, VM hard
resets and the containment suite (D128), in one VM with 2 CPUs, 4 GiB and
20 GiB. It runs once, after every other milestone is done. Everything that
can happen on the Mac happens first, so the VM only runs tests.

## 0. On the Mac, with no VM (any time before the batch)

```sh
apps/humata2/testing/linux/run.sh --build-only --batch apps/humata2/testing/linux/batch_targets.txt
bazel test //apps/humata2/testing/linux:in_vm_test
```

A build break here is a Linux gap to fix before the VM starts. Stage B
(`batch_targets_gui.txt`) also needs the glibc sysroot (D402); until it
lands, `bazel build --config=linux_arm64_gnu //apps/humata2/ui` is the
check that shows what it is missing.

## 1. Create and start the VM (once)

```sh
limactl create --name=humata2-linux --cpus=2 --memory=4 --disk=20 \
  --vm-type=vz --mount-none template:ubuntu-lts
limactl start humata2-linux
limactl shell humata2-linux sudo apt-get update
limactl shell humata2-linux sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
  procps git openssh-server openssh-client tmux \
  libxkbcommon0 libxkbcommon-x11-0 libfontconfig1 libfreetype6 libwayland-client0 \
  libx11-xcb1 libxcb1 mesa-vulkan-drivers libvulkan1 xvfb weston fonts-dejavu-core
limactl shell humata2-linux sudo install -d -m 1777 /v
```

`/v` is where `in_vm.sh` puts each test's runfiles, at the same absolute
paths as on the Mac, and it removes them after each test. No mounts: the
VM only sees what a test needs.

## 2. Stage A: every suite without a GUI (static musl)

```sh
apps/humata2/testing/linux/run.sh --batch apps/humata2/testing/linux/batch_targets.txt
```

Bazel runs two tests at a time (the VM's CPUs) and caches each pass, so
after a fix only the changed tests run again. To rerun one test:
`run.sh //apps/humata2/daemon/humatad:stop_sessions_test -- --test_filter=<name>`.
Outputs (`test.outputs/`) come back to `bazel-testlogs` as usual.

## 3. Stage B: GPUI suites and view-tree snapshots (glibc)

Needs D402's sysroot, the Linux `engine::headless` (lavapipe Vulkan in the
VM) and the ungate script that drops the macOS-only gates on the non-image
desktop suites. Then:

```sh
apps/humata2/testing/linux/run.sh --batch apps/humata2/testing/linux/batch_targets_gui.txt \
  -- --config=linux_arm64_gnu
```

## 4. Launch smoke test (D129, D403)

The real `humata2` binary (stage B build) under X11 (`Xvfb :1` with
`Xft.dpi: 192`) and Wayland (`weston --backend=headless --scale=2`), with
lavapipe. Pass when frame 01 is non-blank, the missing-glyph hook
(`App::on_missing_glyphs`) reports nothing for the bundled fonts, and the
window's scale factor is 2.

## 5. Real-SSH machine flows and VM hard resets

The flows in `daemon/machines/testing` against the VM's own sshd, then the
PER-28 hard-reset cases: `limactl stop -f humata2-linux` mid-write, start
it, and check the store opens whole and sessions come back.

## 6. Containment (SEC-6, D128, D405)

`//apps/humata2/daemon/contain/...` is in stage A. Until a Linux backend
exists, the Linux cases assert that containment refuses to start
unconfined (fail closed, D217).

## After the batch

`limactl delete -f humata2-linux` frees the 20 GiB. Record the run in the
LINUX milestone's Build gates.
