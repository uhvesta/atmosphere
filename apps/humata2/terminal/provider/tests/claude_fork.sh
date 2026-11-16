#!/bin/sh
set -eu
exec /usr/bin/python3 "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/provider/tests/fork_recovery.py" --provider claude
