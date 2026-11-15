#!/bin/sh
# Drive the Rust view headlessly; produce screenshots without desktop input.
set -eu
prototype_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_dir=$(CDPATH= cd -- "$prototype_dir/../../.." && pwd)
cd "$workspace_dir"
export HUMATA2_UI_STEPS=${1:-'click:tool-Diff;click:widen-review;click:widen-review'}
export HUMATA2_UI_CAPTURE_DIR=${2:-"$prototype_dir/target/ui-captures/manual"}
cargo +1.100.0 test -p humata2-ui-prototype --lib --locked \
    --target-dir "$prototype_dir/target" --config profile.dev.debug=0 \
    capture_named_steps -- --nocapture
