#!/bin/sh
set -eu
: "${TERMINAL_DEMO_STATE:?Set TERMINAL_DEMO_STATE to the private state of the running demo.}"
terminal_demo_live=$1
terminal_demo_host=$2
terminal_demo_bundle=$3
terminal_demo_smoke=$4
terminal_demo_runfiles=${RUNFILES_DIR:-$TEST_SRCDIR}
if [ ! -f "$TERMINAL_DEMO_STATE/server.json" ] || [ ! -f "$TERMINAL_DEMO_STATE/browser.token" ]; then
  echo 'Start the demo through bazel run //apps/humata2/terminal/demo:live before verifying a milestone.' >&2
  exit 1
fi
set -- --state "$TERMINAL_DEMO_STATE" --port "${TERMINAL_DEMO_PORT:-8741}" --quiet --require-running
if [ -n "${TERMINAL_DEMO_PROFILES:-}" ]; then
  set -- "$@" --profiles "$TERMINAL_DEMO_PROFILES"
fi
RUNFILES_DIR=$terminal_demo_runfiles
export RUNFILES_DIR
"$terminal_demo_runfiles/$terminal_demo_live" "$terminal_demo_host" "$terminal_demo_bundle" "$@" --stage
TERMINAL_VIEWER_URL_FILE=$TERMINAL_DEMO_STATE/candidate.url
export TERMINAL_VIEWER_URL_FILE
"${TERMINAL_LAB_NODE:-node}" "$terminal_demo_runfiles/$terminal_demo_smoke"
"$terminal_demo_runfiles/$terminal_demo_live" "$terminal_demo_host" "$terminal_demo_bundle" "$@"
echo 'Browser acceptance passed; the stable demo now serves this milestone. Existing sessions were preserved.'
