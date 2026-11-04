#!/bin/sh
# Runs lint-runner against a fake linter and checks its reports and exit
# status.
set -eu

runner="$1"
dir="${TEST_TMPDIR:?}"
tool="$dir/fake-lint"
cat >"$tool" <<'EOF'
#!/bin/sh
echo "Linting 2 files"
echo "/sandbox/execroot/_main/a.swift:3:5: warning: [Indentation] replace leading whitespace"
echo "b.swift:10:1: error: Line Length Violation: too long (line_length)" >&2
exit "$1"
EOF
chmod +x "$tool"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# Report mode: the runner succeeds and records the tool's status.
"$runner" --tool-name Fake --human "$dir/human" --machine "$dir/machine" \
  --exit-code "$dir/exit_code" -- "$tool" 2 a.swift || fail "report mode should succeed"
[ "$(cat "$dir/exit_code")" = 2 ] || fail "exit code file should hold the tool's status"
grep -q "line_length" "$dir/human" || fail "human report should keep stderr"
grep -q "^a.swift:3:5: " "$dir/human" || fail "absolute paths should map back to the input they name"
grep -q '"uri":"a.swift"' "$dir/machine" || fail "SARIF should use the input's relative path"
[ "$(grep -o '"ruleId"' "$dir/machine" | wc -l | tr -d ' ')" = 2 ] ||
  fail "SARIF should have one result per finding"
grep -q '"ruleId":"Indentation"' "$dir/machine" || fail "SARIF should carry swift-format rule IDs"
grep -q '"ruleId":"line_length"' "$dir/machine" || fail "SARIF should carry SwiftLint rule IDs"

# Fail mode: a failing tool fails the runner with its status and the hint.
if "$runner" --tool-name Fake --human "$dir/human" --machine "$dir/machine" \
  --hint "run the formatter" -- "$tool" 3 2>"$dir/stderr"; then
  fail "fail mode should fail when the tool does"
else
  status=$?
fi
[ "$status" = 3 ] || fail "fail mode should exit with the tool's status, got $status"
grep -q "run the formatter" "$dir/stderr" || fail "fail mode should print the hint"

# Fail mode, clean tool: the runner succeeds.
"$runner" --tool-name Fake --human "$dir/human" --machine "$dir/machine" \
  -- "$tool" 0 || fail "a clean run should succeed"

echo PASS
