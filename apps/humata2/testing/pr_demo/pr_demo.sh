#!/usr/bin/env bash
# shellcheck shell=bash
# DV-9: a pull request to review with no network. Builds an isolated home
# with a bare acme/demo remote, a fake `gh` and one open pull request,
# acme/demo#1, shaped like the daemon's stress test (pr_diff_stress.rs):
# every line of a 20,000-line file changed, a 200-character path, a rename,
# a binary, a mode change and 295 small files, 300 files in all. Then it
# prints how to start humatad2_admin and the app against that home.
#
#   bazel run //apps/humata2/testing/pr_demo -- /tmp/h2-pr
#
# The folder must be new or empty, and short: the daemon's socket goes
# inside it. Nothing here reads or writes the real home (~/.humata2,
# ~/.copilot, ~/.claude, ~/.codex, ~/.gitconfig) or runs the real `gh`.
set -euo pipefail

fail() {
  echo "pr_demo: $*" >&2
  exit 2
}

[[ $# -eq 1 && -n "$1" ]] || fail "give one folder for the demo home, for example: bazel run //apps/humata2/testing/pr_demo -- /tmp/h2-pr"

home=$1
if [[ "$home" != /* ]]; then
  home="${BUILD_WORKING_DIRECTORY:-$PWD}/$home"
fi
home=${home%/}
[[ "$home" =~ ^[A-Za-z0-9._/-]+$ ]] || fail "'$home' has characters the printed commands can't carry. Use letters, digits, '.', '_', '-' and '/' only, for example /tmp/h2-pr."
parent=$(dirname "$home")
base=$(basename "$home")
[[ "$base" != . && "$base" != .. ]] || fail "'$home' doesn't name a folder of its own. Give one, for example /tmp/h2-pr."
[[ -d "$parent" ]] || fail "'$parent' doesn't exist. Make it first, or give a folder in one that exists, for example /tmp/h2-pr."
# Symbolic links and '..' resolved, so the checks below see where files go.
home="$(cd "$parent" && pwd -P)/$base"
[[ ! -L "$home" ]] || fail "'$home' is a symbolic link. Give a real folder, for example /tmp/h2-pr."

# The real home, from the user database rather than $HOME, and $HOME too.
real_home=$(eval echo "~$(id -un)")
for guarded in "$real_home" "${HOME:-}"; do
  [[ -n "$guarded" ]] || continue
  if [[ -d "$guarded" ]]; then guarded=$(cd "$guarded" && pwd -P); fi
  guarded=${guarded%/}
  case "$guarded/" in
    "$home"/*) fail "'$home' is your home folder or holds it. Give a new folder of its own, for example /tmp/h2-pr." ;;
  esac
  for kept in .humata2 .copilot .claude .codex; do
    case "$home/" in
      "$guarded/$kept"/*) fail "'$home' is inside $guarded/$kept, which the real Humata or an agent keeps. Give a folder outside it, for example /tmp/h2-pr." ;;
    esac
  done
done
if [[ -e "$home" ]]; then
  [[ -d "$home" ]] || fail "'$home' is a file. Give a new or empty folder."
  if [[ -n "$(ls -A "$home")" ]]; then
    fail "'$home' isn't empty. Give a new folder; if it is an earlier pr_demo home you no longer need, delete it first."
  fi
fi
socket="$home/.humata2/humatad.sock"
# macOS allows 104 bytes for a socket's path, Linux 108.
(( ${#socket} <= 100 )) || fail "'$home' is too long for the daemon's socket ($socket is ${#socket} characters, at most 100 work). Use a shorter folder, for example /tmp/h2-pr."

runfiles=${RUNFILES_DIR:-${TEST_SRCDIR:-$0.runfiles}}
fake_gh="$runfiles/_main/apps/humata2/testing/pr_demo/fake_gh.sh"
if [[ ! -f "$fake_gh" ]]; then
  fake_gh="$(cd "$(dirname "$0")" && pwd)/fake_gh.sh"
fi
[[ -f "$fake_gh" ]] || fail "the fake gh (fake_gh.sh) isn't in the runfiles. Start it with bazel run //apps/humata2/testing/pr_demo -- <folder>."

mkdir -p "$home"
chmod 700 "$home"
mkdir -p "$home/.humata2"
chmod 700 "$home/.humata2"

# From here on, git and everything else sees only the demo home.
export HOME="$home"
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
unset XDG_CONFIG_HOME GH_TOKEN GITHUB_TOKEN GH_ENTERPRISE_TOKEN GH_CONFIG_DIR

owner=acme
name=demo
number=1
title="Change everything at once"
author=mira
head_branch=mira/everything

bare="$home/remotes/$owner/$name.git"
work="$home/work/$owner-$name"
mkdir -p "$(dirname "$bare")" "$work"
git init -q --bare --initial-branch=main "$bare"
git init -q --initial-branch=main "$work"
g() {
  git -C "$work" -c user.name=pr_demo -c user.email=pr_demo@example.invalid -c commit.gpgsign=false "$@"
}
g remote add origin "$bare"

# The base, on main.
cd "$work" || exit 1
printf '%s/%s\n' "$owner" "$name" > README.md
awk 'BEGIN { for (i = 1; i <= 20000; i++) print "line " i }' > big.txt
mkdir -p notes scripts
printf 'These notes move.\nThey keep every word they had.\n' > notes/old_name.md
printf '#!/bin/sh\necho run\n' > scripts/run.sh
chmod 644 scripts/run.sh
g add -A
g commit -q -m "The base of the pull request"
g push -q origin main

# The pull request's head: 300 files changed.
g checkout -q -b "$head_branch"
awk 'BEGIN { for (i = 1; i <= 20000; i++) print "line " i ", changed" }' > big.txt
long="$(printf 'a_folder_with_a_rather_long_name/%.0s' 1 2 3 4)and_a_file_name_that_is_long_too_"
while (( ${#long} < 197 )); do long="${long}x"; done
long="$long.rs"
mkdir -p "$(dirname "$long")"
printf 'pub fn deep() {}\n' > "$long"
mkdir -p docs
g mv notes/old_name.md docs/new_name.md
mkdir -p assets
printf '\211PNG\r\n\032\n\000\000\000\rIHDR\000\001\002\003\377\376\375\000\000\000\000' > assets/logo.bin
chmod 755 scripts/run.sh
mkdir -p src
for i in $(seq 1 295); do
  n=$(printf '%03d' "$i")
  printf 'pub fn f%d() -> u32 { %d }\n' "$i" "$i" > "src/file_$n.rs"
done
g add -A
g commit -q -m "$title"
g push -q origin "HEAD:refs/pull/$number/head" "HEAD:refs/heads/$head_branch"
head_oid=$(g rev-parse HEAD)
subject=$(g log -1 --format=%s)
read -r files added removed < <(g diff --numstat main...HEAD |
  awk -F'\t' '{ f++; a += ($1 == "-" ? 0 : $1); r += ($2 == "-" ? 0 : $2) } END { print f + 0, a + 0, r + 0 }')
cd "$home" || exit 1

# The fake GitHub: what `gh pr view` and the heads query answer.
gh_dir="$home/gh"
mkdir -p "$gh_dir"/{visible,prs,posted,listed,review-comments}
cp "$fake_gh" "$gh_dir/gh"
chmod 755 "$gh_dir/gh"
printf "FAKE_TOKEN='gho_PrDemoFakeTokenNotRealAtAll0123456789'\nFAKE_LOGIN='uhvesta'\n" > "$gh_dir/env"
: > "$gh_dir/visible/$owner-$name"
url="https://github.com/$owner/$name/pull/$number"
cat > "$gh_dir/prs/$owner-$name-$number.json" <<EOF
{"number":$number,"url":"$url","title":"$title","author":{"login":"$author"},"baseRefName":"main","headRefName":"$head_branch","headRefOid":"$head_oid","headRepositoryOwner":{"login":"$owner"},"isCrossRepository":false,"changedFiles":$files,"additions":$added,"deletions":$removed,"state":"OPEN","mergedAt":null,"commits":[{"oid":"$head_oid","messageHeadline":"$subject"}],"statusCheckRollup":[]}
EOF
cat > "$gh_dir/graphql.json" <<EOF
{"data":{"pr_${owner}_${name}_${number}":{"pullRequest":{"number":$number,"url":"$url","baseRefName":"main","headRefName":"$head_branch","headRefOid":"$head_oid","state":"OPEN","commits":{"totalCount":1}}}}}
EOF

# The agent a review starts: a shell loop, as in the daemon's tests, so no
# real agent CLI runs.
conv="$home/conv"
mkdir -p "$conv" "$home/channels"
cat > "$home/agent.sh" <<EOF
CONV_DIR='$conv'
mode=\$1; id=\$2
case "\$mode" in
  --session-id) f="\$CONV_DIR/\$id.jsonl"; : > "\$f" ;;
  --resume) f="\$CONV_DIR/\$id.jsonl" ;;
  --fresh) f="\$CONV_DIR/other.jsonl"; : > "\$f" ;;
esac
echo "agent up \$mode"
while IFS= read -r line; do echo "got: \$line"; printf '%s\n' "\$line" >> "\$f"; done
EOF
cat > "$home/agents.json" <<EOF
[{"name":"fake","program":"/bin/sh","start_args":["$home/agent.sh","--session-id","{conversation}"],"resume_args":["$home/agent.sh","--resume","{conversation}"],"conversation_file":"$conv/{conversation}.jsonl"}]
EOF

bin="${BUILD_WORKSPACE_DIRECTORY:-.}/bazel-bin"
cat <<EOF
pr_demo: $owner/$name#$number is open on a fake GitHub in $home ($files files, +$added -$removed).

Environment for the daemon and the app:
HOME=$home
HUMATA2_GH=$gh_dir/gh

1. Build the daemon, holder, CLI and app:
   bazel build //apps/humata2/daemon/humatad:humatad2_admin //apps/humata2/daemon/holder:humata2_holder //apps/humata2/cli:humata //apps/humata2/desktop:humata2

2. Start the daemon and leave it running:
   env HOME=$home HUMATA2_GH=$gh_dir/gh $bin/apps/humata2/daemon/humatad/humatad2_admin serve --admin --socket $socket --state $home/.humata2/state --root $home --holder $bin/apps/humata2/daemon/holder/humata2_holder --humata $bin/apps/humata2/cli/humata --channels $home/channels --agents $home/agents.json --default-agent fake --remote-template 'file://$home/remotes/{owner}/{name}.git'

3. Start the app on it:
   env HOME=$home $bin/apps/humata2/desktop/humata2 --socket $socket --data $home/app-data

4. In the app, start a pull request review of $url.

To start over, stop both and delete $home.
EOF
