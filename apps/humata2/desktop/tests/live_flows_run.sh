#!/bin/sh
# Runs a filtered slice of :live_flows_cases without compiling tests/live_flows
# a second time. Honors Bazel's --test_filter (TESTBRIDGE_TEST_ONLY) over the
# default module filter (LIVE_FLOWS_FILTER).
set -eu

cases="${LIVE_FLOWS_CASES:?set LIVE_FLOWS_CASES to the live flow test binary}"
filter="${TESTBRIDGE_TEST_ONLY:-${LIVE_FLOWS_FILTER:-}}"
if [ -n "$filter" ]; then
    exec "$cases" "$filter" "$@"
fi
exec "$cases" "$@"
