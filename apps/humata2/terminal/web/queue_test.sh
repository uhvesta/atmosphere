#!/bin/sh
set -eu
exec "$TERMINAL_LAB_NODE" "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/web/queue_test.mjs" "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/web/src/lib.rs"
