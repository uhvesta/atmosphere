#!/bin/sh
set -eu
exec "${TERMINAL_LAB_NODE:-node}" "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/web/browser_smoke.mjs"
