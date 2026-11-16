#!/bin/sh
set -eu
: "${TERMINAL_LAB_NODE:?Set TERMINAL_LAB_NODE to an installed Node runtime for this manual acceptance test}"
exec "$TERMINAL_LAB_NODE" \
  "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/vt/tests/wasm_checkpoint.mjs" \
  "$TEST_SRCDIR/$TEST_WORKSPACE/$NATIVE_FIXTURE" \
  "$TEST_SRCDIR/$TEST_WORKSPACE/$GHOSTTY_WASM" "$TEST_TMPDIR"
