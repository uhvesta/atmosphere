#!/bin/sh
# Compile the same Rust UI for the browser. No JavaScript UI implementation.
set -eu
prototype_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_dir=$(CDPATH= cd -- "$prototype_dir/../../.." && pwd)
cd "$workspace_dir"
cargo +nightly build -p humata2-ui-prototype --locked --lib \
    --target wasm32-unknown-unknown --target-dir "$prototype_dir/target" \
    --config profile.dev.debug=0 \
    --config 'target.wasm32-unknown-unknown.rustflags=["-C","link-arg=-zstack-size=8388608","--cfg","getrandom_backend=\"wasm_js\""]'
"$prototype_dir/.tools/bin/wasm-bindgen" --target web --out-dir "$prototype_dir/web/pkg" \
    "$prototype_dir/target/wasm32-unknown-unknown/debug/humata2_ui_prototype.wasm"
if [ "${1:-serve}" = build ]; then exit 0; fi
exec python3 -I -m http.server 8740 --bind 127.0.0.1 --directory "$prototype_dir/web"
