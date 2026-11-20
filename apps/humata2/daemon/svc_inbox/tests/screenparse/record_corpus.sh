#!/usr/bin/env bash
# Records the SCREENPARSE corpus: each agent CLI, on a cheap model, in an
# isolated home, follows the step scripts in scripts/ under a real holder,
# and screenparse_record writes its labelled screens to
#   daemon/svc_inbox/tests/screens/<agent>/<cli-version>/
#
# Usage: record_corpus.sh <screenparse_record binary> <scratch dir> [agent...] [-- script...]
#   agents: claude codex copilot (default: all three; agy needs GEMINI_API_KEY)
#
# Never touches the real ~/.claude, ~/.claude.json, ~/.codex or ~/.copilot:
# each agent gets its own copy of only what it needs to be signed in, and
# the run checks afterwards that none of the real ones changed. Tokens are
# passed in the environment only.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <screenparse_record binary> <scratch dir> [agent...] [-- script...]" >&2
  exit 2
fi
RECORDER=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
SCRATCH=$2
shift 2
HERE=$(cd "$(dirname "$0")" && pwd)
OUT_ROOT=$(cd "$HERE/.." && pwd)/screens

AGENTS=()
SCRIPTS=()
while [[ $# -gt 0 ]]; do
  if [[ $1 == "--" ]]; then shift; SCRIPTS=("$@"); break; fi
  AGENTS+=("$1"); shift
done
[[ ${#AGENTS[@]} -eq 0 ]] && AGENTS=(claude codex copilot)
if [[ ${#SCRIPTS[@]} -eq 0 ]]; then
  for f in "$HERE"/scripts/*.script; do
    name=$(basename "$f" .script)
    [[ $name == *.* ]] || SCRIPTS+=("$name")
  done
fi

CLAUDE_MODEL=${CLAUDE_MODEL:-claude-haiku-5-5}
CODEX_MODEL=${CODEX_MODEL:-gpt-6-luna}
COPILOT_MODEL=${COPILOT_MODEL:-mai-code-2-flash}
AGY_MODEL=${AGY_MODEL:-gemini-3.8-flash-low}

mkdir -p "$SCRATCH"
SCRATCH=$(cd "$SCRATCH" && pwd)
MARKER="$SCRATCH/.marker"
touch "$MARKER"

# Files under the real config folders that changed since the marker, minus
# logs of sessions that were already running.
real_changes() {
  local dirs=()
  for d in "$HOME/.claude" "$HOME/.codex" "$HOME/.copilot"; do [[ -d $d ]] && dirs+=("$d"); done
  { find "${dirs[@]}" -newer "$MARKER" -type f 2>/dev/null; \
    [[ $HOME/.claude.json -nt $MARKER ]] && echo "$HOME/.claude.json"; } \
    | grep -vE '/(log|logs|sessions|projects|session-env|shell-snapshots|history\.jsonl|statsig|cache)(/|$)|\.log$|\.sqlite(-wal|-shm)?$' || true
}

# Asks each CLI its version in a scratch home: even `--version` leaves
# files in the home it runs under.
version_of() {
  local vhome="$SCRATCH/version-home"
  mkdir -p "$vhome/claude" "$vhome/codex" "$vhome/copilot"
  local env=(env HOME="$vhome" CLAUDE_CONFIG_DIR="$vhome/claude" CODEX_HOME="$vhome/codex"
    COPILOT_HOME="$vhome/copilot")
  case $1 in
    claude) "${env[@]}" claude --version | awk '{print $1}' ;;
    codex) "${env[@]}" codex --version | awk '{print $2}' ;;
    copilot) "${env[@]}" copilot --version | head -1 | awk '{print $4}' | sed 's/\.$//' ;;
    agy) "${env[@]}" agy --version | head -1 ;;
  esac
}

# Fills the global LAUNCH (program + args) and ENVS for one agent run.
setup() {
  local agent=$1 script=$2 home=$3 work=$4
  LAUNCH=()
  ENVS=("TERM=xterm-256color" "COLORTERM=truecolor" "LANG=en_US.UTF-8")
  local mcp="$home/mcp.json"
  case $agent in
    claude)
      mkdir -p "$home/claude"
      if [[ $script != login ]]; then
        python3 - "$HOME/.claude.json" "$home/claude/.claude.json" <<'EOF'
import json, sys
keep = ["hasCompletedOnboarding", "lastOnboardingVersion", "oauthAccount", "userID",
        "installMethod", "migrationVersion", "firstStartTime", "numStartups",
        "opusProMigrationComplete", "sonnet1m45MigrationComplete", "hasSeenTasksHint"]
src = json.load(open(sys.argv[1]))
json.dump({k: src[k] for k in keep if k in src}, open(sys.argv[2], "w"))
EOF
        ENVS+=("CLAUDE_SECURESTORAGE_CONFIG_DIR=")
      else
        mkdir -p "$home/claude-storage"
        ENVS+=("CLAUDE_SECURESTORAGE_CONFIG_DIR=$home/claude-storage")
      fi
      echo '{}' >"$home/claude/settings.json"
      ENVS+=("CLAUDE_CONFIG_DIR=$home/claude" "DISABLE_AUTOUPDATER=1")
      LAUNCH=("$(command -v claude)" --model "$CLAUDE_MODEL")
      [[ $script == plan ]] && LAUNCH+=(--permission-mode plan)
      if [[ $script == mcp ]]; then
        printf '{"mcpServers":{"screenparse":{"command":"python3","args":["%s"]}}}' "$HERE/mcp_echo.py" >"$mcp"
        LAUNCH+=(--strict-mcp-config --mcp-config "$mcp")
      fi
      ;;
    codex)
      mkdir -p "$home/codex"
      [[ $script != login ]] && cp "$HOME/.codex/auth.json" "$home/codex/auth.json"
      printf 'model = "%s"\n' "$CODEX_MODEL" >"$home/codex/config.toml"
      # HOME too: Codex keeps helper files under $HOME/.codex/tmp even
      # when CODEX_HOME points elsewhere.
      ENVS+=("HOME=$home" "CODEX_HOME=$home/codex")
      # on-request + read-only: every write and command asks.
      LAUNCH=("$(command -v codex)" --model "$CODEX_MODEL" --ask-for-approval on-request --sandbox read-only)
      if [[ $script == mcp ]]; then
        LAUNCH+=(-c 'mcp_servers.screenparse.command="python3"' -c "mcp_servers.screenparse.args=[\"$HERE/mcp_echo.py\"]")
      fi
      ;;
    copilot)
      mkdir -p "$home/copilot"
      if [[ $script != login ]]; then
        python3 - "$HOME/.copilot/config.json" "$home/copilot/config.json" <<'EOF'
import json, sys
text = "\n".join(l for l in open(sys.argv[1]).read().splitlines() if not l.lstrip().startswith("//"))
src = json.loads(text)
keep = ["firstLaunchAt", "appTipShown", "lastLoggedInUser", "loggedInUsers"]
json.dump({k: src[k] for k in keep if k in src}, open(sys.argv[2], "w"))
EOF
        if [[ -n ${SCREENPARSE_COPILOT_TOKEN:-} ]]; then ENVS+=("COPILOT_GITHUB_TOKEN=$SCREENPARSE_COPILOT_TOKEN"); fi
      fi
      ENVS+=("COPILOT_HOME=$home/copilot")
      LAUNCH=("$(command -v copilot)" --model "$COPILOT_MODEL")
      [[ $script == plan ]] && LAUNCH+=(--plan)
      if [[ $script == mcp ]]; then
        printf '{"mcpServers":{"screenparse":{"type":"local","command":"python3","args":["%s"],"tools":["*"]}}}' "$HERE/mcp_echo.py" >"$mcp"
        LAUNCH+=(--additional-mcp-config "@$mcp")
      fi
      ;;
    agy)
      if [[ -z ${GEMINI_API_KEY:-} && $script != login ]]; then
        echo "skip agy/$script: agy signs in only through the keychain; set GEMINI_API_KEY (D900)" >&2
        return 1
      fi
      mkdir -p "$home/agyhome"
      ENVS+=("HOME=$home/agyhome")
      [[ $script != login ]] && ENVS+=("GEMINI_API_KEY=$GEMINI_API_KEY")
      LAUNCH=("$(command -v agy)" --model "$AGY_MODEL")
      [[ $script == plan ]] && LAUNCH+=(--mode plan)
      ;;
    *) echo "unknown agent $agent (claude, codex, copilot, agy)" >&2; return 1 ;;
  esac
}

status=0
for agent in "${AGENTS[@]}"; do
  version=$(version_of "$agent")
  out="$OUT_ROOT/$agent/$version"
  mkdir -p "$out"
  for script in "${SCRIPTS[@]}"; do
    source_file="$HERE/scripts/$script.$agent.script"
    [[ -f $source_file ]] || source_file="$HERE/scripts/$script.script"
    run="$SCRATCH/$agent/$script"
    rm -rf "$run"
    mkdir -p "$run"
    # The key that trusts a new folder differs: Claude Code's trust list
    # starts on "No, exit".
    case $agent in
      claude) trust="down enter" ;;
      *) trust="enter" ;;
    esac
    file="$run/$script.script"
    sed "s/@trust@/$trust/" "$source_file" >"$file"
    home="$run/home"
    work="$run/work"
    mkdir -p "$home" "$work"
    echo hello >"$work/notes.txt"
    git -C "$work" init -q
    setup "$agent" "$script" "$home" "$work" || continue
    args=(--agent "$agent" --cli-version "$version" --program "${LAUNCH[0]}")
    for a in "${LAUNCH[@]:1}"; do args+=(--arg "$a"); done
    for e in "${ENVS[@]}"; do args+=(--env "$e"); done
    args+=(--cwd "$work" --script "$file" --out "$out" --cols 80 --rows 40 --widths 60,80,120,200)
    echo "== $agent $version $script" >&2
    if ! "$RECORDER" "${args[@]}" 2>"$run/record.log"; then
      echo "   failed: see $run/record.log" >&2
      status=1
    fi
  done
done

changed=$(real_changes)
if [[ -n $changed ]]; then
  echo "WARNING: these real config files changed during the run (check whether a run touched them):" >&2
  echo "$changed" >&2
  status=1
fi
exit $status
