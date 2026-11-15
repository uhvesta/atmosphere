#!/usr/bin/env bash
# Runs Bazel test targets on Linux from this Mac: builds each one for static
# musl (linux_arm64), copies the binary and its runfiles into the
# humata2-linux Lima VM, and runs it there. Exits non-zero if any test fails.
#
#   apps/humata2/testing/linux/run.sh //libs/tca-rust:all
#
# Create the VM once with:
#   limactl create --name=humata2-linux --cpus=4 --memory=4 --disk=20 \
#     --vm-type=vz --mount-none template:ubuntu-lts && limactl start humata2-linux
set -euo pipefail

VM="${HUMATA2_LINUX_VM:-humata2-linux}"
cd "$(dirname "$0")/../../../.."
export PATH="$PWD/.tools/bin:$PATH"

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <bazel test targets…>" >&2
  exit 2
fi

status="$(limactl list --format '{{.Status}}' "$VM" 2>/dev/null || true)"
if [[ "$status" != "Running" ]]; then
  echo "The Linux VM '$VM' isn't running (status: ${status:-missing}). Start it with 'limactl start $VM', or create it as the header of $0 shows." >&2
  exit 2
fi

flags=(--config=linux_arm64 "--extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64")
mapfile -t tests < <(bazel query "kind('rust_test|sh_test|cc_test', set($*))" 2>/dev/null)
if [[ ${#tests[@]} -eq 0 ]]; then
  echo "No test targets match: $*" >&2
  exit 2
fi
bazel build "${flags[@]}" "${tests[@]}"
bin="$(bazel info "${flags[@]}" bazel-bin)"

failed=()
for t in "${tests[@]}"; do
  rel="${t#//}"
  rel="${rel/://}"
  stage="$(mktemp -d)"
  tar -C "$bin" -chf "$stage/t.tar" "$rel" $( [[ -d "$bin/$rel.runfiles" ]] && echo "$rel.runfiles" )
  limactl copy "$stage/t.tar" "$VM:/tmp/h2t.tar" >/dev/null
  rm -rf "$stage"
  if limactl shell "$VM" bash -c "rm -rf /tmp/h2t && mkdir /tmp/h2t && tar -C /tmp/h2t -xf /tmp/h2t.tar && cd /tmp/h2t && TEST_TMPDIR=\$(mktemp -d) ./$rel" >/tmp/h2t.log 2>&1; then
    echo "PASS (linux) $t"
  else
    echo "FAIL (linux) $t"
    tail -40 /tmp/h2t.log
    failed+=("$t")
  fi
done

if [[ ${#failed[@]} -gt 0 ]]; then
  echo "${#failed[@]} of ${#tests[@]} test targets failed on Linux." >&2
  exit 1
fi
echo "All ${#tests[@]} test targets passed on Linux."
