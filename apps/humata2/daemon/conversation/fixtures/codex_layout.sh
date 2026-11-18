#!/bin/sh
set -eu
exec /usr/bin/python3 -I "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/daemon/conversation/fixtures/codex_layout.py"
