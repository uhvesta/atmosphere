#!/bin/sh
# The shard script (live_flows_shard.sh) on a stand-in case binary with cases
# a, b and c, where b hangs the way leftovers.rs reports a hang: it names
# itself in LIVE_FLOWS_HUNG and the binary exits 101. The script must fail,
# name b, and still run c, without running a again.
set -eu

shard="$PWD/${LIVE_FLOWS_SHARD:?set LIVE_FLOWS_SHARD to live_flows_shard.sh}"
work="${TEST_TMPDIR:?Bazel sets TEST_TMPDIR}/shard_test"
rm -rf "$work"
mkdir -p "$work/tmp"
ran="$work/ran"
: >"$ran"

cat >"$work/cases" <<EOF
#!/bin/sh
skips=" "
names=""
while [ \$# -gt 0 ]; do
    case "\$1" in
        --skip) skips="\$skips\$2 "; shift 2 ;;
        --*) shift ;;
        *) names="\$names \$1"; shift ;;
    esac
done
for name in \$names; do
    case "\$skips" in *" \$name "*) continue ;; esac
    echo "\$name" >>"$ran"
    if [ "\$name" = b ]; then
        echo b >>"\$LIVE_FLOWS_HUNG"
        exit 101
    fi
    echo "test \$name ... ok"
done
exit 0
EOF
chmod +x "$work/cases"

set +e
LIVE_FLOWS_CASES="$work/cases" TEST_TMPDIR="$work/tmp" TEST_TOTAL_SHARDS=0 \
    sh "$shard" --exact a b c >"$work/out" 2>&1
status=$?
set -e
cat "$work/out"

fail() {
    echo "FAIL: $1" >&2
    exit 1
}
[ "$status" -ne 0 ] || fail "a hung case left the run passing"
grep -q 'failed: b$' "$work/out" || fail "the run didn't name b as the hung case"
[ "$(grep -c '^a$' "$ran")" -eq 1 ] || fail "a, which had passed, ran again"
[ "$(grep -c '^b$' "$ran")" -eq 1 ] || fail "b, which hung, ran again"
[ "$(grep -c '^c$' "$ran")" -eq 1 ] || fail "c, which hadn't run, didn't run once"
echo PASS
