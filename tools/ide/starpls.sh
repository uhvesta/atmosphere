#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -P "$(dirname "$0")/../.." && pwd)
PATH="$repo_root/.tools/bin:$PATH"
export PATH

exec "$repo_root/.tools/bin/starpls" "$@"
