#!/bin/sh
# Drive the Rust view headlessly; produce screenshots without desktop input.
set -eu
prototype_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_dir=$(CDPATH= cd -- "$prototype_dir/../../.." && pwd)
cd "$workspace_dir"
export HUMATA2_UI_STEPS=${1:-'panel:Diff;expand;restore'}
export HUMATA2_UI_CAPTURE_DIR=${2:-"$prototype_dir/target/ui-captures/manual"}
exec .tools/bin/bazel run -c opt //apps/humata2/ui-prototype:ui_test -- \
    capture_named_steps --nocapture
