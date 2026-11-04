#!/bin/sh
# Runs each polyglot binary built for the host and checks that it reached the
# C, Rust and Zig implementations.
set -eu

for bin in "$@"; do
  out="$("$bin")"
  printf '%s\n' "$out"
  case "$out" in
    *"calling C, Rust and Zig (2 + 3 = 5)"*) ;;
    *)
      echo "unexpected output from $bin" >&2
      exit 1
      ;;
  esac
done
