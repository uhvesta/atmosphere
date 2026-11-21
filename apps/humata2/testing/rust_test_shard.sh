#!/bin/sh
# Runs this Bazel shard's share of a libtest binary (RUST_TEST_BIN).
# rules_rust doesn't shard libtest binaries natively, so this lists the cases,
# sorts them by name and runs every TEST_TOTAL_SHARDS-th one from
# TEST_SHARD_INDEX.
set -eu

bin="${RUST_TEST_BIN:?set RUST_TEST_BIN to the libtest binary}"
total="${TEST_TOTAL_SHARDS:-0}"
if [ "$total" = 0 ]; then
    if [ -n "${TESTBRIDGE_TEST_ONLY:-}" ]; then
        exec "$bin" "$TESTBRIDGE_TEST_ONLY" "$@"
    fi
    exec "$bin" "$@"
fi

shard="${TEST_SHARD_INDEX:?Bazel sets TEST_SHARD_INDEX with TEST_TOTAL_SHARDS}"
if [ -n "${TEST_SHARD_STATUS_FILE:-}" ]; then
    touch "$TEST_SHARD_STATUS_FILE"
fi
if [ -n "${TESTBRIDGE_TEST_ONLY:-}" ]; then
    if [ "$shard" = 0 ]; then
        exec "$bin" "$TESTBRIDGE_TEST_ONLY" "$@"
    fi
    exit 0
fi
if [ $# -gt 0 ]; then
    if [ "$shard" = 0 ]; then
        exec "$bin" "$@"
    fi
    exit 0
fi

listed="$("$bin" --list --format terse)"
names="$(printf '%s\n' "$listed" | sed -n 's/: test$//p' | LC_ALL=C sort)"
set -f
set --
index=0
for name in $names; do
    if [ $((index % total)) -eq "$shard" ]; then
        set -- "$@" "$name"
    fi
    index=$((index + 1))
done

if [ $# -eq 0 ]; then
    exit 0
fi
exec "$bin" --exact "$@"
