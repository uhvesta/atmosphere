#!/bin/sh
set -eu
terminal_demo_runfiles=${RUNFILES_DIR:-$0.runfiles}
exec python3 "$terminal_demo_runfiles/_main/apps/humata2/terminal/demo/live.py" --require-running "$@"
