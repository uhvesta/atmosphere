#!/bin/sh
set -eu
exec /usr/bin/python3 "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/provider/tests/copilot_fork.py"
