#!/usr/bin/env bash
# shellcheck shell=bash
# Behaviour tests for in_vm.sh, the --run_under wrapper that runs one Bazel
# test inside the Linux VM. A fake limactl runs the "VM" side on this machine
# under a separate root, so every path the wrapper moves is checked for real.
set -euo pipefail

wrapper="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
# Physical paths, as Bazel uses (/private/var, not the /var link, on macOS).
work="$(cd "$TEST_TMPDIR" && pwd -P)/w"
fails=0

fail() {
  echo "FAIL: $*" >&2
  fails=$((fails + 1))
}

# A fake limactl: `list` reports $FAKE_STATUS, `shell [--workdir D] VM cmd…`
# runs cmd here, in D, and records that the VM was entered.
make_fake() {
  mkdir -p "$work/bin"
  cat >"$work/bin/limactl" <<'FAKE'
#!/usr/bin/env bash
set -eu
case "$1" in
  list) echo "$FAKE_STATUS" ;;
  shell)
    shift
    dir=/
    if [[ "$1" == --workdir ]]; then dir="$2"; shift 2; fi
    shift
    echo entered >>"$FAKE_LOG"
    cd "$dir"
    exec "$@"
    ;;
  *) exit 64 ;;
esac
FAKE
  chmod +x "$work/bin/limactl"
}

# A fake Bazel layout: an output base with a runfiles tree whose test
# binary records what it saw, then exits with the code in its first arg.
make_layout() {
  rm -rf "$work/ob" "$work/vm" "$work/log"
  local rf="$work/ob/execroot/_main/bazel-out/cfg/bin/pkg/t.runfiles"
  mkdir -p "$rf/_main/pkg" "$work/ob/external/x" "$work/vm" \
    "$work/ob/execroot/_main/bazel-out/cfg/testlogs/pkg/t/test.outputs" \
    "$work/ob/execroot/_main/_tmp/t"
  echo "runfile data" >"$work/ob/external/x/data.txt"
  ln -s "$work/ob/external/x/data.txt" "$rf/_main/pkg/data.txt"
  cat >"$work/ob/execroot/_main/bazel-out/cfg/bin/pkg/t" <<'BIN'
#!/bin/bash
out="$TEST_UNDECLARED_OUTPUTS_DIR"
{
  echo "srcdir=$TEST_SRCDIR"
  echo "custom=$CUSTOM"
  echo "path=$PATH"
  echo "home=$HOME"
  echo "pwd=$PWD"
  echo "data=$(cat pkg/data.txt)"
  printf 'arg=%s\n' "$@"
} >"$out/seen.txt"
exit "$1"
BIN
  chmod +x "$work/ob/execroot/_main/bazel-out/cfg/bin/pkg/t"
  ln -s "$work/ob/execroot/_main/bazel-out/cfg/bin/pkg/t" "$rf/_main/pkg/t"
  RF="$rf"
}

# Runs the wrapper the way Bazel's local test runner does.
run_wrapper() {
  local status="$1"
  shift
  (
    cd "$RF/_main"
    env -i PATH="$work/hostonly:/usr/bin:/bin" \
      TEST_SRCDIR="$RF" \
      TEST_TMPDIR="$work/ob/execroot/_main/_tmp/t" \
      TEST_UNDECLARED_OUTPUTS_DIR="$work/ob/execroot/_main/bazel-out/cfg/testlogs/pkg/t/test.outputs" \
      CUSTOM="value with spaces" \
      HUMATA2_LINUX_VM=fake-vm H2_LIMACTL="$work/bin/limactl" H2_VM_ROOT="$work/vm" \
      FAKE_STATUS="$status" FAKE_LOG="$work/log" \
      "$wrapper" pkg/t "$@"
  )
}

make_fake
outputs="$work/ob/execroot/_main/bazel-out/cfg/testlogs/pkg/t/test.outputs"

# A passing test passes, sees its env, args and runfiles, and its outputs
# come back to the host.
make_layout
if run_wrapper Running 0 "two words" >/dev/null 2>&1; then :; else fail "a passing test should exit 0"; fi
seen="$outputs/seen.txt"
if [[ -f "$seen" ]]; then
  grep -qx "srcdir=$work/vm$RF" "$seen" || fail "TEST_SRCDIR should point at the VM copy of the runfiles"
  grep -qx "custom=value with spaces" "$seen" || fail "test env values should reach the test unchanged"
  grep -q "hostonly" "$seen" && fail "the host PATH should not reach the VM"
  grep -qx "home=$work/vm$work/ob/execroot/_main/_tmp/t/home" "$seen" || fail "HOME should be a fresh dir in the test's VM temp dir"
  grep -qx "pwd=$work/vm$RF/_main" "$seen" || fail "the test should run in the VM copy of its working dir"
  grep -qx "data=runfile data" "$seen" || fail "symlinked runfiles should be readable in the VM"
  grep -qx "arg=two words" "$seen" || fail "an argument with spaces should arrive as one argument"
else
  fail "the test's undeclared outputs should be copied back to the host"
fi
[[ -e "$work/vm$RF" ]] && fail "the VM copy of the runfiles should be removed after the run"
[[ -e "$work/vm$work/ob/execroot/_main/_tmp/t" ]] && fail "the VM copy of the temp dir should be removed after the run"

# A failing test fails with its own exit code.
make_layout
code=0
run_wrapper Running 3 >/dev/null 2>&1 || code=$?
[[ "$code" == 3 ]] || fail "a test exiting 3 should make the wrapper exit 3 (got $code)"
[[ -e "$work/vm$RF" ]] && fail "the VM copy should be removed after a failing run too"

# A VM that isn't running fails the test without entering the VM.
make_layout
code=0
run_wrapper Stopped 0 >/dev/null 2>&1 || code=$?
[[ "$code" != 0 ]] || fail "a stopped VM should fail the test"
[[ -e "$work/log" ]] && fail "a stopped VM should not be entered"

# Outside a Bazel output base the wrapper refuses before touching the VM.
make_layout
code=0
(
  cd "$work"
  # $work is under TEST_TMPDIR, inside Bazel's output base: a path under it
  # would hold /execroot/. This one is outside every output base.
  env -i PATH=/usr/bin:/bin TEST_SRCDIR="/h2-not-an-output-base/runfiles" TEST_TMPDIR="$work" \
    HUMATA2_LINUX_VM=fake-vm H2_LIMACTL="$work/bin/limactl" H2_VM_ROOT="$work/vm" \
    FAKE_STATUS=Running FAKE_LOG="$work/log" "$wrapper" pkg/t 0 >/dev/null 2>&1
) || code=$?
[[ "$code" != 0 ]] || fail "a TEST_SRCDIR outside an output base should fail"
[[ -e "$work/log" ]] && fail "a TEST_SRCDIR outside an output base should not enter the VM"

if [[ "$fails" -gt 0 ]]; then
  echo "$fails check(s) failed." >&2
  exit 1
fi
echo "all in_vm.sh checks passed"
