#!/bin/sh
set -eu
terminal_fixture_root="$TEST_SRCDIR/$TEST_WORKSPACE"
exec "${TERMINAL_LAB_NODE:-node}" "$terminal_fixture_root/apps/humata2/terminal/demo/fixture.mjs" "$terminal_fixture_root/apps/humata2/terminal/web/browser_smoke.mjs" "$terminal_fixture_root/apps/humata2/terminal/platform_test/linux_browser.sh" "$terminal_fixture_root/apps/humata2/terminal/demo/send_text.mjs"
