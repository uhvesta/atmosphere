#!/bin/bash
# shellcheck shell=bash
set -euo pipefail
terminal_demo_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$terminal_demo_repo"
export PATH="$terminal_demo_repo/.tools/bin:$PATH"
terminal_demo_action=${1:-status}
if [[ "$terminal_demo_action" == provision ]]; then
  if [[ $# -ne 1 ]]; then
    echo 'Use ./terminal-ssh-demo.sh provision; it builds and installs the matching Linux holder.' >&2
    exit 2
  fi
  bazel build --config=linux_arm64 \
    --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
    //apps/humata2/daemon/holder:humata2_holder
  terminal_demo_output=$(bazel cquery --config=linux_arm64 \
    --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
    --output=files //apps/humata2/daemon/holder:humata2_holder)
  terminal_demo_holder=$(
    python3 - "$terminal_demo_repo" "$terminal_demo_output" <<'PY'
import os,sys
files=sys.argv[2].splitlines()
if len(files)!=1 or not files[0].startswith('bazel-out/'):
    sys.exit('Could not identify the configured Linux holder artifact. Inspect the Bazel query output before retrying provision.')
print(os.path.realpath(os.path.join(sys.argv[1],files[0])))
PY
  )
  exec bazel run //apps/humata2/terminal/platform_test:ssh_demo -- provision "$terminal_demo_holder"
fi
exec bazel run //apps/humata2/terminal/platform_test:ssh_demo -- "$@"
