#!/bin/sh
# Runs the Swift hello binary built for the host and checks its output.
set -eu

out="$("$1")"
printf '%s\n' "$out"
case "$out" in
  *"hello from Swift on "*"uuid: ok"*) ;;
  *)
    echo "unexpected output" >&2
    exit 1
    ;;
esac
