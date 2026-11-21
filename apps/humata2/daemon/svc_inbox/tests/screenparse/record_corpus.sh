#!/usr/bin/env bash
# Records the SCREENPARSE corpus: each agent CLI, on a cheap model, in an
# isolated home, follows the step scripts in scripts/ under a real holder,
# and screenparse_record writes its labelled screens to
#   daemon/svc_inbox/tests/screens/<agent>/<cli-version>/
#
# Usage: record_corpus.sh <screenparse_record binary> <scratch dir> [agent...] [-- script...]
#   agents: claude codex copilot (default: all three; agy needs GEMINI_API_KEY)
#
# Never touches the real ~/.claude, ~/.claude.json, ~/.codex, ~/.copilot or ~/.gemini:
# each agent gets its own copy of only what it needs to be signed in, and
# the run checks afterwards that none of the real ones changed. Tokens are
# passed in the environment only.
set -euo pipefail

# Clear host CLI/XDG path overrides so non-`env -i` invocations cannot leak
# real-home paths into `version_of`, the recorder, or any child CLI.
unset \
  CLAUDE_CONFIG_DIR \
  CLAUDE_SECURESTORAGE_CONFIG_DIR \
  CODEX_HOME \
  COPILOT_HOME \
  COPILOT_CACHE_HOME \
  COPILOT_PKG_CACHE_HOME \
  GEMINI_CLI_HOME \
  XDG_CONFIG_HOME \
  XDG_CACHE_HOME \
  XDG_DATA_HOME \
  XDG_STATE_HOME

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

# Files and directories under the real config folders that changed since the
# marker. No exclusions: any real-home write during a run must fail the check.
real_changes() {
  local dirs=()
  for d in "$HOME/.claude" "$HOME/.codex" "$HOME/.copilot" "$HOME/.gemini"; do
    [[ -d $d ]] && dirs+=("$d")
  done
  if [[ ${#dirs[@]} -gt 0 ]]; then
    find "${dirs[@]}" -newer "$MARKER" \( -type f -o -type d -o -type l \) 2>/dev/null || true
  fi
  if [[ -e $HOME/.claude.json && $HOME/.claude.json -nt $MARKER ]]; then
    echo "$HOME/.claude.json"
  fi
}

# Asks each CLI its version in a scratch home: even `--version` leaves
# files in the home it runs under.
version_of() {
  local vhome="$SCRATCH/version-home"
  mkdir -p \
    "$vhome/claude" "$vhome/claude-storage" \
    "$vhome/codex" \
    "$vhome/copilot" "$vhome/copilot-cache" "$vhome/copilot-pkg-cache" \
    "$vhome/gemini" \
    "$vhome/xdg-config" "$vhome/xdg-cache" "$vhome/xdg-data" "$vhome/xdg-state"
  local env=(env
    HOME="$vhome"
    CLAUDE_CONFIG_DIR="$vhome/claude"
    CLAUDE_SECURESTORAGE_CONFIG_DIR="$vhome/claude-storage"
    DISABLE_AUTOUPDATER=1
    CODEX_HOME="$vhome/codex"
    COPILOT_HOME="$vhome/copilot"
    COPILOT_CACHE_HOME="$vhome/copilot-cache"
    COPILOT_PKG_CACHE_HOME="$vhome/copilot-pkg-cache"
    COPILOT_AUTO_UPDATE=false
    GEMINI_CLI_HOME="$vhome/gemini"
    XDG_CONFIG_HOME="$vhome/xdg-config"
    XDG_CACHE_HOME="$vhome/xdg-cache"
    XDG_DATA_HOME="$vhome/xdg-data"
    XDG_STATE_HOME="$vhome/xdg-state"
  )
  case $1 in
    claude) "${env[@]}" claude --version | awk '{print $1}' ;;
    codex) "${env[@]}" codex --version | awk '{print $2}' ;;
    copilot) "${env[@]}" copilot --no-auto-update --version | head -1 | awk '{print $4}' | sed 's/\.$//' ;;
    agy) "${env[@]}" agy --version | head -1 ;;
  esac
}

# Fills the global LAUNCH (program + args) and ENVS for one agent run.
setup() {
  local agent=$1 script=$2 home=$3 work=$4
  LAUNCH=()
  mkdir -p \
    "$home/claude" "$home/claude-storage" \
    "$home/codex" \
    "$home/copilot" "$home/copilot-cache" "$home/copilot-pkg-cache" \
    "$home/gemini" \
    "$home/xdg-config" "$home/xdg-cache" "$home/xdg-data" "$home/xdg-state"
  ENVS=(
    "TERM=xterm-256color"
    "COLORTERM=truecolor"
    "LANG=en_US.UTF-8"
    "HOME=$home"
    "CLAUDE_CONFIG_DIR=$home/claude"
    "DISABLE_AUTOUPDATER=1"
    "CODEX_HOME=$home/codex"
    "COPILOT_HOME=$home/copilot"
    "COPILOT_CACHE_HOME=$home/copilot-cache"
    "COPILOT_PKG_CACHE_HOME=$home/copilot-pkg-cache"
    "COPILOT_AUTO_UPDATE=false"
    "GEMINI_CLI_HOME=$home/gemini"
    "XDG_CONFIG_HOME=$home/xdg-config"
    "XDG_CACHE_HOME=$home/xdg-cache"
    "XDG_DATA_HOME=$home/xdg-data"
    "XDG_STATE_HOME=$home/xdg-state"
  )
  local claude_storage="$home/claude-storage"
  local mcp="$home/mcp.json"
  case $agent in
    claude)
      if [[ $script != login ]]; then
        if [[ -f $HOME/.claude.json ]]; then
          python3 - "$HOME/.claude.json" "$home/claude/.claude.json" <<'EOF'
import json, sys
keep = ["hasCompletedOnboarding", "lastOnboardingVersion", "oauthAccount", "userID",
        "installMethod", "migrationVersion", "firstStartTime", "numStartups",
        "opusProMigrationComplete", "sonnet1m45MigrationComplete", "hasSeenTasksHint"]
src = json.load(open(sys.argv[1]))
json.dump({k: src[k] for k in keep if k in src}, open(sys.argv[2], "w"))
EOF
        fi
        if [[ -n ${SCREENPARSE_CLAUDE_TOKEN:-} ]]; then
          ENVS+=("CLAUDE_CODE_OAUTH_TOKEN=$SCREENPARSE_CLAUDE_TOKEN")
        elif [[ -n ${ANTHROPIC_API_KEY:-} ]]; then
          ENVS+=("ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY")
        else
          # Keychain fallback; HOME and CLAUDE_CONFIG_DIR stay isolated in $home.
          claude_storage=""
        fi
      else
        ENVS+=("CLAUDE_CODE_OAUTH_TOKEN=" "ANTHROPIC_API_KEY=")
      fi
      echo '{}' >"$home/claude/settings.json"
      LAUNCH=("$(command -v claude)" --model "$CLAUDE_MODEL")
      [[ $script == plan ]] && LAUNCH+=(--permission-mode plan)
      if [[ $script == mcp ]]; then
        printf '{"mcpServers":{"screenparse":{"command":"python3","args":["%s"]}}}' "$HERE/mcp_echo.py" >"$mcp"
        LAUNCH+=(--strict-mcp-config --mcp-config "$mcp")
      fi
      ;;
    codex)
      [[ $script != login ]] && cp "$HOME/.codex/auth.json" "$home/codex/auth.json"
      printf 'model = "%s"\n' "$CODEX_MODEL" >"$home/codex/config.toml"
      # on-request + read-only: every write and command asks.
      LAUNCH=("$(command -v codex)" --model "$CODEX_MODEL" --ask-for-approval on-request --sandbox read-only)
      if [[ $script == mcp ]]; then
        LAUNCH+=(-c 'mcp_servers.screenparse.command="python3"' -c "mcp_servers.screenparse.args=[\"$HERE/mcp_echo.py\"]")
      fi
      ;;
    copilot)
      if [[ $script != login ]]; then
        if [[ -f $HOME/.copilot/config.json ]]; then
          python3 - "$HOME/.copilot/config.json" "$home/copilot/config.json" <<'EOF'
import json, sys
text = "\n".join(l for l in open(sys.argv[1]).read().splitlines() if not l.lstrip().startswith("//"))
src = json.loads(text)
keep = ["firstLaunchAt", "appTipShown", "lastLoggedInUser", "loggedInUsers"]
json.dump({k: src[k] for k in keep if k in src}, open(sys.argv[2], "w"))
EOF
        fi
        if [[ -n ${SCREENPARSE_COPILOT_TOKEN:-} ]]; then ENVS+=("COPILOT_GITHUB_TOKEN=$SCREENPARSE_COPILOT_TOKEN"); fi
      else
        ENVS+=("COPILOT_GITHUB_TOKEN=")
      fi
      LAUNCH=("$(command -v copilot)" --no-auto-update --model "$COPILOT_MODEL")
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
      if [[ $script != login ]]; then
        ENVS+=("GEMINI_API_KEY=$GEMINI_API_KEY")
      else
        ENVS+=("GEMINI_API_KEY=")
      fi
      LAUNCH=("$(command -v agy)" --model "$AGY_MODEL")
      [[ $script == plan ]] && LAUNCH+=(--mode plan)
      ;;
    *) echo "unknown agent $agent (claude, codex, copilot, agy)" >&2; return 1 ;;
  esac
  ENVS+=("CLAUDE_SECURESTORAGE_CONFIG_DIR=$claude_storage")
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
