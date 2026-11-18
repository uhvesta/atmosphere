#!/bin/sh
set -eu
# $1 is the CLI name (claude, codex or copilot).
exec /usr/bin/python3 -I "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/daemon/conversation/fixtures/fork_resume.py" "$1"
