#!/bin/sh
set -eu
export PATH="/opt/homebrew/bin:$PATH"
if [ -z "${LIMA_HOME:-}" ]; then
  LIMA_HOME=$(python3 -c 'import os,pwd; print(os.path.join(pwd.getpwuid(os.getuid()).pw_dir,".lima"))')
  export LIMA_HOME
fi
exec bash "$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/platform_test/ssh_demo.sh" status
