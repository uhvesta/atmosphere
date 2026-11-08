#!/bin/bash
# Runs a Mac snapshot test from this terminal: run_mac.sh <target> [record].
#
# The test reads accessibility trees and captures windows, which needs the
# Accessibility and Screen Recording access given to the terminal in System
# Settings › Privacy & Security. `bazel test` runs tests under the Bazel
# server, which has neither, so this builds the test and runs its bundle
# here instead. With `record`, it writes the references into the source
# tree; review them with `git diff`.
set -euo pipefail

if [[ $# -lt 1 || ! $1 =~ ^//[^:]+:[^:]+$ ]]; then
  echo "usage: $0 //<package>:<mac snapshot test> [record]" >&2
  exit 2
fi
target=$1
package=${target#//}
package=${package%%:*}
name=${target##*:}

cd "$(git rev-parse --show-toplevel)"
bazel build --config=xcode "$target"
runfiles=$PWD/bazel-bin/$package/$name.runfiles
bundle=$(mktemp -d)
# The tests share this Mac's one screen, keyboard focus and pointer, so two
# at once type into each other's windows and capture each other in their
# images. Each run waits its turn; builds above still run side by side.
readonly lock=/private/tmp/flowkit-mac-test.lock
trap 'rm -rf "$bundle"; if [[ "$(cat "$lock/owner" 2>/dev/null || true)" == "$$" ]]; then rm -rf "$lock"; fi' EXIT
waited=0
until mkdir "$lock" 2>/dev/null; do
  owner="$(cat "$lock/owner" 2>/dev/null || true)"
  # A run that's gone, such as one stopped with Control-C before its trap ran.
  if [[ -n "$owner" ]] && ! kill -0 "$owner" 2>/dev/null; then
    rm -rf "$lock"
    continue
  fi
  if ((waited == 0)); then
    echo "Waiting for another Mac snapshot test (process ${owner:-starting}) to finish…" >&2
  fi
  waited=1
  sleep 2
done
echo "$$" >"$lock/owner"
unzip -q "$runfiles/_main/$package/$name.zip" -d "$bundle"

export FLOWKIT_SNAPSHOTS=SnapshotTests/__Snapshots__
export FLOWKIT_PACKAGE=$package
export FLOWKIT_TARGET=$target
export TEST_SRCDIR=$runfiles
if [[ ${2:-} == record ]]; then
  export FLOWKIT_RECORD=1
fi
cd "$bundle"
xcrun xctest "$bundle"/*.xctest
