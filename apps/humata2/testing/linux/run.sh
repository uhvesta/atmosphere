#!/usr/bin/env bash
# Runs Bazel test targets on Linux from this Mac. Bazel builds each test for
# static musl (linux_arm64) here, then runs it through in_vm.sh, which streams
# the test and its runfiles into the humata2-linux Lima VM, runs it there with
# its full test environment and arguments, and brings its outputs back. Test
# results are cached like any Bazel test, so a rerun only runs what changed.
#
#   apps/humata2/testing/linux/run.sh //libs/tca-rust:all
#   apps/humata2/testing/linux/run.sh --batch apps/humata2/testing/linux/batch_targets.txt
#   apps/humata2/testing/linux/run.sh --build-only --batch …   # no VM needed
#
# Options after `--` go to `bazel test` (for example --test_filter=…). A
# target written `-//pkg:t` is left out, as in any Bazel target list.
# Create the VM as BATCH.md in this folder shows.
set -euo pipefail

VM="${HUMATA2_LINUX_VM:-humata2-linux}"
here="$(cd "$(dirname "$0")" && pwd -P)"
cd "$here/../../../.."
export PATH="$PWD/.tools/bin:$PATH"

targets=()
extra=()
build_only=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --batch)
      [[ -f "${2:-}" ]] || {
        echo "--batch needs a target list file (got '${2:-}'). Use apps/humata2/testing/linux/batch_targets.txt or a copy of it." >&2
        exit 2
      }
      while IFS= read -r line; do
        line="${line%%#*}"
        line="${line//[[:space:]]/}"
        [[ -n "$line" ]] && targets+=("$line")
      done <"$2"
      shift 2
      ;;
    --build-only)
      build_only=1
      shift
      ;;
    --)
      shift
      extra=("$@")
      break
      ;;
    *)
      targets+=("$1")
      shift
      ;;
  esac
done
if [[ ${#targets[@]} -eq 0 ]]; then
  echo "usage: $0 [--build-only] [--batch <file>] [<bazel test targets…>] [-- <bazel test options…>]" >&2
  exit 2
fi

flags=(
  --config=linux_arm64
  "--extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64"
  --build_tests_only
  --skip_incompatible_explicit_targets
)

# Build everything first: it needs no VM, and a build break shows up before
# the VM is started.
bazel build "${flags[@]}" -- "${targets[@]}"
[[ "$build_only" == 1 ]] && exit 0

limactl="$(command -v limactl || true)"
if [[ -z "$limactl" ]]; then
  echo "limactl isn't on PATH. Install Lima (brew install lima) and create the VM as BATCH.md in this folder shows." >&2
  exit 2
fi
status="$("$limactl" list --format '{{.Status}}' "$VM" 2>/dev/null || true)"
if [[ "$status" != "Running" ]]; then
  echo "The Linux VM '$VM' isn't running (status: ${status:-missing}). Start it with 'limactl start $VM', or create it as BATCH.md in this folder shows." >&2
  exit 2
fi

# The VM has 2 CPUs; two tests at a time keeps it busy without thrashing 4 GiB.
bazel test "${flags[@]}" \
  "--run_under=$here/in_vm.sh" \
  --strategy=TestRunner=local \
  --local_test_jobs=2 \
  --test_output=errors \
  "--test_env=HUMATA2_LINUX_VM=$VM" \
  "--test_env=H2_LIMACTL=$limactl" \
  "--test_env=LIMA_HOME=${LIMA_HOME:-$HOME/.lima}" \
  "--test_env=HOME=$HOME" \
  ${extra[@]+"${extra[@]}"} \
  -- "${targets[@]}"
