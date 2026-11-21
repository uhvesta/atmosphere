#!/usr/bin/env bash
# Slow-tier live corpus re-recording test (`//apps/humata2/daemon/svc_inbox:screenparse_live`).
#
# Runs only when explicitly invoked (`manual` + `external` + `slow`) and skips
# cleanly (exit 0 with SKIP notice) when agent logins or CLI binaries are
# missing so missing credentials never cause spurious failures.
set -euo pipefail

ROOT="${TEST_SRCDIR:-.}/${TEST_WORKSPACE:-_main}"
RECORDER="$ROOT/apps/humata2/daemon/svc_inbox/screenparse_record"
RUNNER="$ROOT/apps/humata2/daemon/svc_inbox/tests/screenparse/record_corpus.sh"

if [[ ! -x "$RECORDER" ]]; then
  echo "SKIP: screenparse_record binary not found at $RECORDER"
  exit 0
fi

AGENTS=()

# Claude: requires binary + token/key or existing .claude.json login.
if command -v claude >/dev/null 2>&1 && {
  [[ -n ${SCREENPARSE_CLAUDE_TOKEN:-} ]] ||
  [[ -n ${ANTHROPIC_API_KEY:-} ]] ||
  [[ -f ${HOME:-}/.claude.json ]]
}; then
  AGENTS+=(claude)
else
  echo "SKIP: claude login or binary missing"
fi

# Codex: requires binary + ~/.codex/auth.json.
if command -v codex >/dev/null 2>&1 && [[ -f ${HOME:-}/.codex/auth.json ]]; then
  AGENTS+=(codex)
else
  echo "SKIP: codex login or binary missing"
fi

# Copilot: requires binary + token or existing ~/.copilot/config.json.
if command -v copilot >/dev/null 2>&1 && {
  [[ -n ${SCREENPARSE_COPILOT_TOKEN:-} ]] ||
  [[ -f ${HOME:-}/.copilot/config.json ]]
}; then
  AGENTS+=(copilot)
else
  echo "SKIP: copilot login or binary missing"
fi

if [[ ${#AGENTS[@]} -eq 0 ]]; then
  echo "SKIP: no agent logins available for screenparse_live"
  exit 0
fi

SCRATCH="${TEST_TMPDIR:-$(mktemp -d)}"
exec "$RUNNER" "$RECORDER" "$SCRATCH/corpus" "${AGENTS[@]}" -- probe
