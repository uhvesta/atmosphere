#!/bin/sh
# Written by bazel run //tools/bootstrap/revup. Do not edit.
set -eu

tools_dir=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
root="$tools_dir/revup"
if [ ! -x "$root/python/bin/python3" ]; then
  printf 'revup is not installed. Run bazel run //tools/bootstrap/revup.\n' >&2
  exit 1
fi

# The token comes from the environment or from your gh login, read by the
# pinned gh, and is never written to disk. git hands it to revup and to git
# push through a credential helper.
REVUP_GITHUB_TOKEN=${REVUP_GITHUB_TOKEN:-${GH_TOKEN:-${GITHUB_TOKEN:-}}}
if [ -z "$REVUP_GITHUB_TOKEN" ]; then
  REVUP_GITHUB_TOKEN=$("$root/gh/gh" auth token --hostname github.com 2>/dev/null || :)
fi
if [ -z "$REVUP_GITHUB_TOKEN" ]; then
  printf 'No GitHub token. Log in with gh auth login or set GH_TOKEN.\n' >&2
  exit 1
fi
export REVUP_GITHUB_TOKEN

# Use the pinned git everywhere, including the git revup finds on PATH. Ignore
# system git config; keep the user's global config for identity and editor.
PATH="$root/git/bin:$PATH"
GIT_EXEC_PATH="$root/git/libexec/git-core"
GIT_TEMPLATE_DIR="$root/git/share/git-core/templates"
GIT_CONFIG_NOSYSTEM=1
export PATH GIT_EXEC_PATH GIT_TEMPLATE_DIR GIT_CONFIG_NOSYSTEM

# An empty helper clears inherited helpers for github.com, such as osxkeychain.
GIT_CONFIG_COUNT=2
GIT_CONFIG_KEY_0=credential.https://github.com.helper
GIT_CONFIG_VALUE_0=
GIT_CONFIG_KEY_1=credential.https://github.com.helper
GIT_CONFIG_VALUE_1='!f() { test "$1" = get && test -n "$REVUP_GITHUB_TOKEN" && printf "username=x-access-token\npassword=%s\n" "$REVUP_GITHUB_TOKEN"; }; f'
export GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0 GIT_CONFIG_KEY_1 GIT_CONFIG_VALUE_1

# Personal revup settings live here instead of ~/.revupconfig. Repository
# settings come from .revupconfig at the repository root.
REVUP_CONFIG_PATH="$tools_dir/revupconfig"
export REVUP_CONFIG_PATH

exec "$root/python/bin/python3" -I -m revup --git-path "$root/git/bin/git" "$@"
