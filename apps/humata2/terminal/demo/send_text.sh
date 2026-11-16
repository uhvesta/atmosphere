#!/bin/sh
set -eu
terminal_demo_runfiles=${RUNFILES_DIR:-$0.runfiles}
exec "${TERMINAL_LAB_NODE:-node}" "$terminal_demo_runfiles/_main/apps/humata2/terminal/demo/send_text.mjs" "$@"
