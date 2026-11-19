#!/bin/sh
# Humata 2 test tiers.
#
#   presubmit.sh                 cheap check: the tests affected by the files
#                                changed in the tree (vs HEAD), minus slow.txt
#   presubmit.sh <targets...>    cheap check over these targets (a train of
#                                several lanes' targets), minus slow.txt
#   presubmit.sh --with-slow <targets...>
#                                these targets exactly, slow ones included
#                                (scoped: one slow target a fix is about)
#   presubmit.sh --case <name> <target>
#                                one case of one target, slow or not: the
#                                fast repro of a slow target's failure
#                                (rules_rust tests and live_flows_test;
#                                passes --test_arg=<name> --test_arg=--exact)
#   presubmit.sh --full          postsubmit: every test under //apps/humata2
#                                (not manual), slow ones included; once
#                                before a release
#
# Each run first compiles everything it will test (`bazel build`), so a
# change that doesn't build fails in minutes, not after the tests.
set -eu
cd "$(dirname "$0")/../../../.."
export PATH="$PWD/.tools/bin:$PATH"
slow="apps/humata2/tools/tests/slow.txt"
flags="--keep_going --test_output=errors"

if [ "${1:-}" = "--full" ]; then
    bazel build --keep_going //apps/humata2/...
    exec bazel test $flags --test_tag_filters=-manual //apps/humata2/...
fi

keep_slow=0
case_name=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --with-slow)
            keep_slow=1
            shift
            ;;
        --case)
            if [ "$#" -lt 2 ]; then
                echo "presubmit: --case needs a test case's name, as \`--list\` prints it without \`: test\`" >&2
                exit 2
            fi
            case_name="$2"
            keep_slow=1
            shift 2
            ;;
        *) break ;;
    esac
done
if [ -n "$case_name" ] && [ "$#" -ne 1 ]; then
    echo "presubmit: --case $case_name runs one case of one target; name exactly one target after it" >&2
    exit 2
fi
if [ -n "$case_name" ]; then
    flags="$flags --test_arg=$case_name --test_arg=--exact"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@" > "$tmp/targets"
else
    # Files changed against HEAD, tracked and untracked, that a package owns.
    { git diff --name-only HEAD -- apps/humata2; git ls-files --others --exclude-standard -- apps/humata2; } \
        | sort -u | while read -r f; do [ -f "$f" ] && printf '%s\n' "$f"; done > "$tmp/files"
    if [ ! -s "$tmp/files" ]; then
        echo "presubmit: nothing changed under apps/humata2" >&2
        exit 0
    fi
    # A changed BUILD file means every test in its package.
    sed -n 's#^\(.*\)/BUILD\.bazel$#//\1:all#p' "$tmp/files" > "$tmp/pkgs"
    grep -v '/BUILD\.bazel$' "$tmp/files" > "$tmp/srcs" || true
    expr=""
    [ -s "$tmp/srcs" ] && expr="rdeps(//apps/humata2/..., set($(tr '\n' ' ' < "$tmp/srcs")))"
    if [ -s "$tmp/pkgs" ]; then
        p="tests(set($(tr '\n' ' ' < "$tmp/pkgs")))"
        expr="${expr:+$expr + }$p"
    fi
    bazel query --keep_going --output=label \
        "tests($expr) except attr(tags, 'manual', //apps/humata2/...)" \
        2>/dev/null > "$tmp/targets" || true
fi

if [ "$keep_slow" = 0 ]; then
    grep -v '^#' "$slow" | grep -vxF -f - "$tmp/targets" > "$tmp/run" || true
    skipped=$(($(wc -l < "$tmp/targets") - $(wc -l < "$tmp/run")))
    [ "$skipped" -gt 0 ] && echo "presubmit: skipping $skipped slow target(s); they run in --full" >&2
else
    cp "$tmp/targets" "$tmp/run"
fi

if [ ! -s "$tmp/run" ]; then
    echo "presubmit: no fast tests cover these changes" >&2
    exit 0
fi
echo "presubmit: $(wc -l < "$tmp/run" | tr -d ' ') target(s)" >&2
# shellcheck disable=SC2046
bazel build --keep_going $(cat "$tmp/run")
if [ -z "$case_name" ]; then
    # shellcheck disable=SC2046
    exec bazel test $flags $(cat "$tmp/run")
fi
# shellcheck disable=SC2046
bazel test $flags $(cat "$tmp/run")
# A name that matches no case runs nothing and passes, so say so.
label="$(cat "$tmp/run")"
logs="bazel-testlogs/$(printf '%s' "${label#//}" | tr ':' '/')"
if ! cat "$logs"/test.log "$logs"/shard_*/test.log 2>/dev/null | grep -Eq '^running [1-9][0-9]* tests?$'; then
    echo "presubmit: no case of $label is named $case_name, so nothing ran; list them with the test binary's --list" >&2
    exit 1
fi
