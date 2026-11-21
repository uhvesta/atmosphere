#!/bin/sh
set -eu
exec python3 "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/web_todo/web/browser_check.py" \
  "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/web_todo/web/smoke_server" \
  "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/web_todo/web/bundle"
