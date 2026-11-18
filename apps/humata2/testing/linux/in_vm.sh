#!/usr/bin/env bash
# Bazel --run_under wrapper: runs one test in the humata2-linux Lima VM.
#
# run.sh passes it as `--run_under=<abs path>/in_vm.sh` with
# `--strategy=TestRunner=local`, so Bazel's test runner calls it on this Mac
# with the test's environment (TEST_SRCDIR, TEST_TMPDIR, --test_env values,
# BUILD `env`) and its argv. It streams the test's runfiles into the VM at the
# same absolute paths under $H2_VM_ROOT (default /v), runs the test there with
# the same environment and arguments, copies the undeclared outputs back and
# exits with the test's status. Nothing stays behind in the VM.
#
# Needs: HUMATA2_LINUX_VM (VM name), H2_LIMACTL (absolute path to limactl).
# VM-side scripts are built in single quotes on purpose.
# shellcheck disable=SC2016
set -euo pipefail

die() {
  echo "in_vm.sh: $*" >&2
  exit 2
}

vm="${HUMATA2_LINUX_VM:-}"
limactl="${H2_LIMACTL:-}"
root="${H2_VM_ROOT:-/v}"
[[ -n "$vm" ]] || die "HUMATA2_LINUX_VM is not set. Run tests through apps/humata2/testing/linux/run.sh, which passes it with --test_env."
[[ -x "$limactl" ]] || die "H2_LIMACTL ('$limactl') is not an executable limactl. Run tests through apps/humata2/testing/linux/run.sh, which passes it with --test_env."
[[ $# -gt 0 ]] || die "no test binary given. Bazel calls this with the test's command line; use it only as --run_under."
srcdir="${TEST_SRCDIR:-}"
tmpdir="${TEST_TMPDIR:-}"
[[ "$srcdir" == /* && "$tmpdir" == /* ]] || die "TEST_SRCDIR and TEST_TMPDIR must be absolute paths (got '$srcdir', '$tmpdir'). Run with --strategy=TestRunner=local so Bazel's local test runner sets them."
case "$srcdir" in
  */execroot/*) base="${srcdir%%/execroot/*}" ;;
  *) die "TEST_SRCDIR ('$srcdir') is not inside a Bazel output base (no /execroot/). Run the test through bazel test." ;;
esac

status="$("$limactl" list --format '{{.Status}}' "$vm" 2>/dev/null || true)"
[[ "$status" == "Running" ]] || die "the Linux VM '$vm' isn't running (status: ${status:-missing}). Start it with 'limactl start $vm', or create it as apps/humata2/testing/linux/BATCH.md shows."

# Paths under the output base move under $root in the VM. (The replacement
# stays unquoted: bash 3.2, the macOS bash, keeps quotes there literally.)
moved="$root$base"
remap() {
  local v="$1"
  printf '%s' "${v//"$base"/$moved}"
}

exe="$1"
shift
[[ "$exe" == /* ]] || exe="$PWD/$exe"

stage="$(mktemp -d "$tmpdir/h2run.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
vm_tmp="$(remap "$tmpdir")"

{
  echo '#!/bin/bash'
  echo 'set -u'
  for name in $(compgen -e); do
    case "$name" in
      PATH | HOME | TMPDIR | PWD | OLDPWD | SHLVL | _ | SHELL | USER | LOGNAME) continue ;;
      HUMATA2_LINUX_VM | H2_LIMACTL | H2_VM_ROOT | LIMA_* | RUNFILES_MANIFEST_FILE | DYLD_* | __CF* | XPC_* | TERM_PROGRAM* | SSH_AUTH_SOCK | COPYFILE_DISABLE) continue ;;
    esac
    printf 'export %s=%q\n' "$name" "$(remap "${!name}")"
  done
  printf 'export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n'
  printf 'export HOME=%q TMPDIR=%q\n' "$vm_tmp/home" "$vm_tmp"
  printf 'mkdir -p "$HOME"\n'
  if [[ -n "${TEST_UNDECLARED_OUTPUTS_DIR:-}" ]]; then
    printf 'mkdir -p %q\n' "$(remap "$TEST_UNDECLARED_OUTPUTS_DIR")"
  fi
  printf 'cd %q || exit 2\n' "$(remap "$PWD")"
  printf 'exec %q' "$(remap "$exe")"
  for arg in "$@"; do
    printf ' %q' "$(remap "$arg")"
  done
  echo
} >"$stage/run"

# What the VM needs: the runfiles tree (links followed), the binary if it
# lies outside it, the test's temp dir (with the run script) and the cwd.
paths=("${srcdir#/}" "${tmpdir#/}")
[[ "$exe" == "$srcdir"/* ]] || paths+=("${exe#/}")
[[ "$PWD" == "$srcdir"* || "$PWD" == "$tmpdir"* ]] || paths+=("${PWD#/}")

tar_flags=(-ch)
[[ "$(uname -s)" == Darwin ]] && tar_flags+=(--no-mac-metadata)
vm_tar='tar -xf - $(tar --version 2>/dev/null | grep -q GNU && echo --warning=no-unknown-keyword)'
q_root="$(printf %q "$root")"

COPYFILE_DISABLE=1 tar "${tar_flags[@]}" -f - -C / "${paths[@]}" |
  "$limactl" shell --workdir / "$vm" /bin/bash -c \
    "set -e; [ -d $q_root ] || sudo -n install -d -m 1777 $q_root; cd $q_root && $vm_tar" ||
  die "could not copy the test's runfiles into '$vm' under $root. Check the VM has free disk ('limactl shell $vm df -h $root'); if $root is missing, the VM user needs passwordless sudo once to create it."

set +e
"$limactl" shell --workdir / "$vm" /usr/bin/env -i /bin/bash "$(remap "$stage")/run" </dev/null
code=$?
set -e

# Undeclared outputs and the runner's status files come back to the host.
# Each is extracted into its own folder: going through / would cross the
# /var -> /private/var link on macOS, which tar refuses.
outs="${TEST_UNDECLARED_OUTPUTS_DIR:-}"
if [[ "$outs" == /* ]]; then
  mkdir -p "$outs"
  "$limactl" shell --workdir / "$vm" /bin/bash -c \
    "d=$(printf %q "$(remap "$outs")"); [ ! -d \"\$d\" ] || tar -cf - -C \"\$d\" ." </dev/null |
    tar -xf - -C "$outs" ||
    echo "in_vm.sh: could not copy the test's undeclared outputs back from '$vm'; the test status still stands." >&2
fi
for f in "${TEST_SHARD_STATUS_FILE:-}" "${TEST_PREMATURE_EXIT_FILE:-}" "${TEST_INFRASTRUCTURE_FAILURE_FILE:-}"; do
  [[ "$f" == /* ]] || continue
  if "$limactl" shell --workdir / "$vm" /usr/bin/test -e "$(remap "$f")" </dev/null; then
    "$limactl" shell --workdir / "$vm" /bin/cat "$(remap "$f")" </dev/null >"$f" ||
      echo "in_vm.sh: could not copy $f back from '$vm'." >&2
  fi
done

clean=()
for p in "${paths[@]}"; do
  clean+=("$root/$p")
done
"$limactl" shell --workdir / "$vm" /bin/rm -rf "${clean[@]}" ||
  echo "in_vm.sh: could not remove the test's files from '$vm' under $root. Free the disk with 'limactl shell $vm rm -rf $root/*' between batches." >&2

exit "$code"
