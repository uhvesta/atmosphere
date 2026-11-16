#!/bin/sh
set -eu
: "${TERMINAL_LAB_SSH_CONFIG:?Set TERMINAL_LAB_SSH_CONFIG to the existing h2-terminal-c1 Lima SSH config}"
host=${TERMINAL_LAB_SSH_HOST:-lima-h2-terminal-c1}
if [ "$host" != lima-h2-terminal-c1 ]; then
    echo 'Runtime preparation is scoped to the existing h2-terminal-c1 VM only.' >&2
    exit 1
fi
root="$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/platform_test"
output=${TEST_UNDECLARED_OUTPUTS_DIR:-$TEST_TMPDIR}
mkdir -p "$output"
if ! /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ConnectTimeout=10 \
    "$host" sh -s < "$root/prepare_remote.sh" > "$output/runtime-preparation.txt" 2>&1; then
    cat "$output/runtime-preparation.txt"
    echo 'Linux browser runtime preparation failed. Inspect the attached log; keep the VM running and repair its package/network state before retrying.' >&2
    exit 1
fi
cat "$output/runtime-preparation.txt"
