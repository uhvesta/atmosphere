#!/bin/sh
# Runs this Bazel shard's share of the live flow cases (LIVE_FLOWS_CASES,
# the libtest binary). rules_rust doesn't shard a test binary, so this
# lists its cases, sorts them by name and runs every TEST_TOTAL_SHARDS-th
# one from TEST_SHARD_INDEX: a module's cases sit together in the sorted
# list, so they spread over the shards instead of piling into one.
# Without sharding every case runs. Arguments (`--test_arg=find::`) pick
# the cases themselves, so the first shard runs just those and the others
# none.
set -eu

cases="${LIVE_FLOWS_CASES:?set LIVE_FLOWS_CASES to the live flow test binary}"

# Runs the libtest binary on "$@". A case still hung past its grace stops
# the binary (live_flows/leftovers.rs), which first names it in
# LIVE_FLOWS_HUNG. That case fails, and the cases that hadn't reported run
# again without it, so one hang fails one case and the others still report.
# Case names and filters hold no spaces, so they pass as words.
run() {
    set -f
    base="$*"
    tmp="${TEST_TMPDIR:-${TMPDIR:-/tmp}}"
    hung_file="$tmp/live_flows_hung"
    log="$tmp/live_flows_run.log"
    code_file="$tmp/live_flows_run.code"
    reported="$tmp/live_flows_reported"
    LIVE_FLOWS_HUNG="$hung_file"
    export LIVE_FLOWS_HUNG
    skips=""
    hung=""
    failed=0
    while :; do
        rm -f "$hung_file"
        # shellcheck disable=SC2086 # words on purpose, see above
        {
            if "$cases" $base $skips; then
                echo 0 >"$code_file"
            else
                echo $? >"$code_file"
            fi
        } 2>&1 | tee "$log"
        code="$(cat "$code_file")"
        if grep -q '^test [^ ]* \.\.\. FAILED$' "$log"; then
            failed=1
        fi
        if [ ! -s "$hung_file" ]; then
            break
        fi
        while IFS= read -r name; do
            hung="$hung $name"
            skips="$skips --skip $name"
        done <"$hung_file"
        sed -nE 's/^test ([^ ]+) \.\.\. (ok|FAILED|ignored)$/\1/p' "$log" >"$reported"
        while IFS= read -r name; do
            skips="$skips --skip $name"
        done <"$reported"
        echo "live flows: hung, so failed:$hung; running the cases that hadn't ended again" >&2
    done
    if [ -n "$hung" ]; then
        echo "live flows: these cases hung past their limit and failed:$hung" >&2
        exit 1
    fi
    if [ "$failed" -ne 0 ] && [ "$code" -eq 0 ]; then
        exit 1
    fi
    exit "$code"
}

total="${TEST_TOTAL_SHARDS:-0}"
if [ "$total" = 0 ]; then
    run "$@"
fi
shard="${TEST_SHARD_INDEX:?Bazel sets TEST_SHARD_INDEX with TEST_TOTAL_SHARDS}"
# Tells Bazel the test shards.
if [ -n "${TEST_SHARD_STATUS_FILE:-}" ]; then
    touch "$TEST_SHARD_STATUS_FILE"
fi
if [ $# -gt 0 ]; then
    if [ "$shard" = 0 ]; then
        run "$@"
    fi
    exit 0
fi

listed="$("$cases" --list --format terse)"
names="$(printf '%s\n' "$listed" | sed -n 's/: test$//p' | LC_ALL=C sort)"
# A case's name is a Rust path: no spaces and nothing a glob would expand.
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
run --exact "$@"
