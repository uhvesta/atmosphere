#!/bin/sh
set -eu
umask 077
: "${TERMINAL_LAB_SSH_CONFIG:?Set TERMINAL_LAB_SSH_CONFIG to the existing h2-terminal-c1 Lima SSH config}"
: "${TERMINAL_VIEWER_URL_FILE:?Set TERMINAL_VIEWER_URL_FILE to the private running localhost:8741 viewer URL file}"
: "${TERMINAL_EXPECT_SESSION_OS:?Set TERMINAL_EXPECT_SESSION_OS to numeric-session-ID to Darwin/Linux JSON for the same Mac host local and SSH sessions}"
host=${TERMINAL_LAB_SSH_HOST:-lima-h2-terminal-c1}
if [ "$host" != lima-h2-terminal-c1 ]; then
  echo 'This acceptance target is scoped to the existing h2-terminal-c1 VM only.' >&2
  exit 1
fi
if [ ! -f "$TERMINAL_VIEWER_URL_FILE" ]; then
  echo 'The private viewer URL file does not exist. Start the Mac terminal host with its browser bundle before running this acceptance target.' >&2
  exit 1
fi
local_port=$(
  python3 - "$TERMINAL_VIEWER_URL_FILE" <<'PY'
from pathlib import Path
import sys, urllib.parse
try:
    url=urllib.parse.urlsplit(Path(sys.argv[1]).read_text().strip())
    port=url.port or 80
    if url.scheme!='http' or url.hostname!='127.0.0.1' or url.username or url.password or not 1024<=port<=65535:
        raise ValueError()
except (ValueError,OSError):
    sys.exit('Use a private HTTP localhost viewer URL on an unprivileged port; credentials were not logged.')
print(port)
PY
)
root="$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/platform_test"
smoke="$TEST_SRCDIR/$TEST_WORKSPACE/apps/humata2/terminal/web/browser_smoke.mjs"
output=${TEST_UNDECLARED_OUTPUTS_DIR:-$TEST_TMPDIR}
mkdir -p "$output"
tunnel_dir=$(mktemp -d /tmp/h2-browser-forward.XXXXXX)
remote_dir=
cleanup() {
  /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -S "$tunnel_dir/control" \
    -O exit "$host" >/dev/null 2>&1 || true
  if [ -n "$remote_dir" ]; then
    /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ConnectTimeout=10 \
      "$host" "rm -rf -- '$remote_dir'" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$tunnel_dir"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
remote_dir=$(/usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ConnectTimeout=10 \
  "$host" 'umask 077; mktemp -d /tmp/h2-linux-browser.XXXXXX')
case "$remote_dir" in
  /tmp/h2-linux-browser.??????) ;;
  *)
    remote_dir=
    echo 'The VM did not return an owned temporary directory.' >&2
    exit 1
    ;;
esac
case "${remote_dir#/tmp/h2-linux-browser.}" in
  *[!a-zA-Z0-9]*)
    remote_dir=
    echo 'The VM returned an unsafe temporary path.' >&2
    exit 1
    ;;
esac
/usr/bin/scp -q -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes \
  "$smoke" "$root/remote_browser.sh" "$root/sanitize.mjs" "$host:$remote_dir/"
# Secret URL/token travels only through SSH stdin into a private remote file.
python3 - "$TERMINAL_VIEWER_URL_FILE" <<'PY' | /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes "$host" \
  "umask 077; cat > '$remote_dir/viewer.url'"
from pathlib import Path
import sys, urllib.parse
url=urllib.parse.urlsplit(Path(sys.argv[1]).read_text().strip())
query=urllib.parse.parse_qsl(url.query,keep_blank_values=True)
if not any(key=='backend' for key,value in query):
    query.append(('backend','webgl'))
print(urllib.parse.urlunsplit(url._replace(netloc='127.0.0.1:8741',query=urllib.parse.urlencode(query))))
PY
printf '%s' "$TERMINAL_EXPECT_SESSION_OS" | /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" \
  -o BatchMode=yes "$host" "umask 077; cat > '$remote_dir/expected-os.json'"
printf '%s' "${TERMINAL_BROWSER_MIRROR:-0}" | /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" \
  -o BatchMode=yes "$host" "umask 077; cat > '$remote_dir/mirror'"
printf '%s' "${TERMINAL_BROWSER_FLAGS:-}" | /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" \
  -o BatchMode=yes "$host" "umask 077; cat > '$remote_dir/browser-flags.json'"
printf '%s\n%s\n%s\n' "${TERMINAL_BROWSER_TAB:-0}" "${TERMINAL_BROWSER_STYLE:-0}" "${TERMINAL_BROWSER_CLIPBOARD:-0}" | /usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" \
  -o BatchMode=yes "$host" "umask 077; cat > '$remote_dir/checks'"
# An independent control master owns only this reverse loopback forward.
/usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=10 -o ServerAliveCountMax=3 -M -S "$tunnel_dir/control" \
  -fNT -R "127.0.0.1:8741:127.0.0.1:$local_port" "$host"
status=0
/usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ServerAliveInterval=10 \
  -o ServerAliveCountMax=3 "$host" "sh '$remote_dir/remote_browser.sh' '$remote_dir'" \
  >"$output/linux-runner.log" 2>&1 || status=$?
/usr/bin/scp -q -r -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes \
  "$host:$remote_dir/output/." "$output/" || {
  echo 'Could not retrieve Linux Chromium proof artifacts. Check the VM connection; no host session was stopped.' >&2
  exit 1
}
cat "$output/linux-runner.log"
if [ "$status" -ne 0 ]; then
  echo 'Linux Chromium acceptance failed. Inspect browser-error.txt and the proof logs; keep the host and VM running for diagnosis.' >&2
  exit "$status"
fi
