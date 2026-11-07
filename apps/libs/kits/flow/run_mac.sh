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
trap 'rm -rf "$bundle"' EXIT
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
