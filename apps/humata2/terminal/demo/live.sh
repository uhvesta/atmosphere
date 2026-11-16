#!/bin/sh
set -eu
terminal_demo_runfiles=${RUNFILES_DIR:-$0.runfiles}
if [ ! -d "$terminal_demo_runfiles" ]; then
    echo 'Could not locate the demo runfiles. Start with bazel run //apps/humata2/terminal/demo:live.' >&2
    exit 1
fi
RUNFILES_DIR=$terminal_demo_runfiles
export RUNFILES_DIR
terminal_demo_host=$1
terminal_demo_bundle=$2
shift 2
exec python3 "$terminal_demo_runfiles/_main/apps/humata2/terminal/demo/live.py" \
    --host "$terminal_demo_runfiles/$terminal_demo_host" --bundle "$terminal_demo_runfiles/$terminal_demo_bundle" "$@"
