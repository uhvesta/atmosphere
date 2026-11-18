#!/usr/bin/env bash
# shellcheck shell=bash
# The checks are strings check() evaluates, so their $ waits on purpose.
# shellcheck disable=SC2016,SC2034 # `check` evals quoted conditions that read the variables
# pr_demo builds a home whose fake GitHub and bare remote agree on one open
# pull request of 300 files, and refuses a home it must not write to.
set -euo pipefail

demo=$1
# Short, so the daemon's socket fits; TEST_TMPDIR is too long on macOS.
scratch=$(mktemp -d /tmp/prdemo.XXXXXX)
trap 'rm -rf "$scratch"' EXIT
# pr_demo names the home by its real path (/tmp is /private/tmp on macOS).
scratch=$(cd "$scratch" && pwd -P)
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

check() {
  if ! eval "$1"; then
    echo "FAIL: $2" >&2
    exit 1
  fi
}

home="$scratch/home"
out=$(HOME="$scratch/me" "$demo" "$home")

bare="$home/remotes/acme/demo.git"
head=$(git --git-dir "$bare" rev-parse refs/pull/1/head)
check '[[ "$(git --git-dir "$bare" rev-parse refs/heads/mira/everything)" == "$head" ]]' \
  "the head branch and refs/pull/1/head are the same commit"
changed=$(git --git-dir "$bare" diff -M --name-only main "$head" | wc -l | tr -d ' ')
check '[[ "$changed" == 300 ]]' "the pull request changes 300 files, not $changed"
# Read whole, so no `grep -q` cuts git short (pipefail would see SIGPIPE).
status=$(git --git-dir "$bare" diff -M --name-status main "$head")
numstat=$(git --git-dir "$bare" diff --numstat main "$head")
raw=$(git --git-dir "$bare" diff-tree -r --raw main "$head")
check 'grep -q "^R100" <<<"$status"' "the pull request has a rename"
check 'grep -q "^-	-	" <<<"$numstat"' "the pull request has a binary"
check 'grep -q "^:100644 100755" <<<"$raw"' "the pull request has a mode change"

gh=$(printf '%s\n' "$out" | sed -n 's/^HUMATA2_GH=//p')
check '[[ "$gh" == "$home"/* && -x "$gh" ]]' "HUMATA2_GH names the home's fake gh, not '$gh'"
view=$("$gh" pr view https://github.com/acme/demo/pull/1 --json headRefOid,state)
check '[[ "$view" == *"\"headRefOid\":\"$head\""* && "$view" == *"\"state\":\"OPEN\""* ]]' \
  "the fake gh knows the open pull request at its head: $view"
check '[[ ! -e "$scratch/me" ]]' "nothing was written to the HOME pr_demo was started with"

# Refusals: each leaves the folder as it was.
mkdir "$scratch/used"
touch "$scratch/used/keep"
check '! HOME="$scratch/me" "$demo" "$scratch/used" 2>/dev/null' "a folder with files in it is refused"
check '[[ "$(ls -A "$scratch/used")" == keep ]]' "a refused folder is left as it was"
mkdir "$scratch/me"
check '! HOME="$scratch/me" "$demo" "$scratch/me" 2>/dev/null' "the home folder itself is refused, even empty"
check '[[ -z "$(ls -A "$scratch/me")" ]]' "the refused home is left empty"
mkdir "$scratch/me/.claude"
check '! HOME="$scratch/me" "$demo" "$scratch/me/.claude/demo" 2>/dev/null' "a folder inside an agent's home folder is refused"
check '[[ -z "$(ls -A "$scratch/me/.claude")" ]]' "nothing was made inside the agent's folder"

echo "pr_demo_test: ok"
